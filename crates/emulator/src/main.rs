mod framebuffer;

use std::cell::Cell;
use std::rc::Rc;
use std::path::Path;
use std::thread;
use std::sync::mpsc;
use std::collections::VecDeque;
use std::time::Duration;
use plato_core::anyhow::{Error, Context as ResultExt};
use plato_core::chrono::Local;
use sdl2::event::Event as SdlEvent;
use sdl2::keyboard::{Scancode, Keycode, Mod};
use framebuffer::{FBCanvas, SharedGeometry, window_size, window_to_device};
use sdl2::mouse::MouseState;
use plato_core::framebuffer::{Framebuffer, UpdateMode};
use plato_core::input::{DeviceEvent, FingerStatus, ButtonCode, ButtonStatus};
use plato_core::document::sys_info_as_html;
use plato_core::view::{View, Event, ViewId, EntryId, AppCmd, EntryKind, SliderId};
use plato_core::view::{process_render_queue, wait_for_all, handle_event, RenderQueue, RenderData};
use plato_core::view::inoink::home::InoInkHome;
use plato_core::view::inoink::actions::{open_article, open_document, InoInkEntry, OpenResult};
use plato_core::view::inoink::image_view::ImageView;
use plato_core::view::inoink::state::InoInk;
use plato_core::view::reader::Reader;
use plato_core::view::notification::show_notification;
use plato_core::view::dialog::Dialog;
use plato_core::view::inoink::light_panel::LightPanel;
use plato_core::view::menu::{Menu, MenuKind};
use plato_core::view::intermission::Intermission;
use plato_core::view::dictionary::Dictionary;
use plato_core::view::calculator::Calculator;
use plato_core::view::touch_events::TouchEvents;
use plato_core::view::rotation_values::RotationValues;
use plato_core::view::common::{locate, locate_by_id, transfer_notifications, overlapping_rectangle};
use plato_core::view::common::{toggle_input_history_menu, toggle_keyboard_layout_menu};
use plato_core::helpers::{load_toml, save_toml};
use plato_core::settings::{Settings, settings_path, IntermKind};
use plato_core::geom::{Rectangle, Axis, Dir};
use plato_core::gesture::{GestureEvent, gesture_events};
use plato_core::device::CURRENT_DEVICE;
use plato_core::battery::{Battery, FakeBattery};
use plato_core::frontlight::{Frontlight, LightLevels};
use plato_core::lightsensor::LightSensor;
use plato_core::font::Fonts;
use plato_core::context::Context;
use plato_core::pt;

pub const APP_NAME: &str = "InoInk";
const DEFAULT_ROTATION: i8 = 1;

const CLOCK_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub struct Options {
    data: Option<String>,
    online: bool,
    tour: bool,
    shots: Option<String>,
    scale: f64,
}

fn parse_scale(value: &str) -> Result<f64, Error> {
    let scale = value.parse::<f64>()
        .map_err(|_| plato_core::anyhow::format_err!("invalid --scale {:?}: expected a finite number from 0.1 through 2.0", value))?;
    if !scale.is_finite() || !(0.1..=2.0).contains(&scale) {
        return Err(plato_core::anyhow::format_err!("invalid --scale {:?}: expected a finite number from 0.1 through 2.0", value));
    }
    Ok(scale)
}

fn parse_options() -> Result<Options, Error> {
    parse_options_from(std::env::args().skip(1))
}

fn parse_options_from(mut args: impl Iterator<Item = String>) -> Result<Options, Error> {
    let mut o = Options { data: None, online: false, tour: false, shots: None, scale: 1.0 };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => o.data = args.next(),
            "--online" => o.online = true,
            "--scripted-tour" => o.tour = true,
            "--screenshots" => o.shots = args.next(),
            "--scale" => {
                let value = args.next().ok_or_else(|| plato_core::anyhow::format_err!("--scale requires a number from 0.1 through 2.0"))?;
                o.scale = parse_scale(&value)?;
            },
            _ => {}
        }
    }
    Ok(o)
}

fn save_screenshot(fb: &dyn Framebuffer, directory: Option<&str>, name: &str) -> Result<(), Error> {
    let path = if let Some(dir) = directory {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("can't create screenshot directory {}", dir))?;
        Path::new(dir).join(name)
    } else {
        name.into()
    };
    fb.save(path.to_str().context("screenshot path is not UTF-8")?)
}

pub fn build_context(fb: Box<dyn Framebuffer>, opts: &Options, hub: &mpsc::Sender<Event>) -> Result<Context, Error> {
    let path = settings_path();
    let mut settings = if path.exists() { load_toml::<Settings, _>(&path)? } else { Settings::default() };
    if let Some(d) = &opts.data {
        settings.inoink.data_dir = d.into();
    }
    let inoink = InoInk::open(&settings.inoink, hub.clone(), opts.online).map_err(|e| plato_core::anyhow::format_err!("{}", e))?;

    let battery = Box::new(FakeBattery::new()) as Box<dyn Battery>;
    let frontlight = Box::new(LightLevels::default()) as Box<dyn Frontlight>;
    let lightsensor = Box::new(0u16) as Box<dyn LightSensor>;
    let fonts = Fonts::load()?;

    Ok(Context::new(fb, None, inoink, settings,
                    fonts, battery, frontlight, lightsensor))
}

#[inline]
fn seconds(timestamp: u32) -> f64 {
    timestamp as f64 / 1000.0
}

#[inline]
pub fn device_event(event: SdlEvent, geometry: &SharedGeometry) -> Option<DeviceEvent> {
    let (window, logical) = geometry.get();
    match event {
        SdlEvent::MouseButtonDown { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Down,
                                       position: window_to_device(x, y, window, logical),
                                       time: seconds(timestamp) }),
        SdlEvent::MouseButtonUp { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Up,
                                       position: window_to_device(x, y, window, logical),
                                       time: seconds(timestamp) }),
        SdlEvent::MouseMotion { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Motion,
                                       position: window_to_device(x, y, window, logical),
                                       time: seconds(timestamp) }),
        _ => None,
    }
}

fn code_from_key(key: Scancode) -> Option<ButtonCode> {
    match key {
        Scancode::B => Some(ButtonCode::Backward),
        Scancode::F => Some(ButtonCode::Forward),
        Scancode::P => Some(ButtonCode::Power),
        Scancode::L => Some(ButtonCode::Light),
        Scancode::H => Some(ButtonCode::Home),
        Scancode::E => Some(ButtonCode::Erase),
        Scancode::G => Some(ButtonCode::Highlight),
        _ => None,
    }
}

fn main() -> Result<(), Error> {
    let opts = parse_options()?;
    let sdl_context = sdl2::init().map_err(Error::msg)?;
    let video_subsystem = sdl_context.video().map_err(Error::msg)?;
    let (width, height) = CURRENT_DEVICE.dims;
    let (physical_width, physical_height) = window_size(width, height, opts.scale);
    if physical_width == 0 || physical_height == 0 {
        return Err(plato_core::anyhow::format_err!("--scale produces a zero-sized emulator window"));
    }
    let window = video_subsystem
                 .window("InoInk Emulator", physical_width, physical_height)
                 .position_centered()
                 .build()?;

    let canvas = window.into_canvas().software().build()?;
    let geometry: SharedGeometry = Rc::new(Cell::new(((0, 0), (0, 0))));
    let fb = FBCanvas::new(canvas, width, height, opts.scale, Rc::clone(&geometry));
    let (tx, rx) = mpsc::channel();
    let mut context = build_context(Box::new(fb), &opts, &tx)?;
    // Live mode uses the computer's own network, which is up from the start.
    context.online = opts.online;
    context.settings.wifi = opts.online || context.settings.wifi;

    context.load_dictionaries();
    context.load_keyboard_layouts();

    let (ty, ry) = mpsc::channel();
    let touch_screen = gesture_events(ry);

    let tx2 = tx.clone();
    thread::spawn(move || {
        while let Ok(evt) = touch_screen.recv() {
            tx2.send(evt).ok();
        }
    });

    let tx3 = tx.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(CLOCK_REFRESH_INTERVAL);
            tx3.send(Event::ClockTick).ok();
        }
    });

    let mut history: Vec<Box<dyn View>> = Vec::new();
    let mut rq = RenderQueue::new();
    let mut view: Box<dyn View> = Box::new(InoInkHome::new(context.fb.rect(), &tx, &mut rq, &mut context));
    let mut tour = if opts.tour { scripted_tour(context.fb.rect()) } else { VecDeque::new() };
    let mut tour_next = std::time::Instant::now() + Duration::from_millis(800);
    if let Some(dir) = &opts.shots {
        std::fs::create_dir_all(dir).ok();
    }

    let mut updating = Vec::new();

    if context.settings.frontlight {
        let levels = context.settings.frontlight_levels;
        context.frontlight.set_intensity(levels.intensity);
        context.frontlight.set_warmth(levels.warmth);
    } else {
        context.frontlight.set_warmth(0.0);
        context.frontlight.set_intensity(0.0);
    }

    println!("{} is running on a Kobo {}.", APP_NAME,
                                            CURRENT_DEVICE.model);
    println!("The framebuffer resolution is {} by {}.", context.fb.rect().width(),
                                                        context.fb.rect().height());

    let mut bus = VecDeque::with_capacity(4);

    'outer: loop {
        let mut event_pump = sdl_context.event_pump().unwrap();
        while let Some(sdl_evt) = event_pump.poll_event() {
            match sdl_evt {
                SdlEvent::Quit { .. } |
                SdlEvent::KeyDown { keycode: Some(Keycode::Escape), keymod: Mod::NOMOD, .. } => {
                    view.handle_event(&Event::Back, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
                    while let Some(mut view) = history.pop() {
                        view.handle_event(&Event::Back, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
                    }
                    break 'outer;
                },
                SdlEvent::KeyUp { scancode: Some(scancode), keymod: Mod::NOMOD, timestamp, .. } => {
                    if let Some(code) = code_from_key(scancode) {
                        ty.send(DeviceEvent::Button {
                            time: seconds(timestamp),
                            code,
                            status: ButtonStatus::Released,
                        }).ok();
                    }
                },
                SdlEvent::KeyDown { scancode: Some(scancode), keymod, timestamp, repeat, .. } => {
                    match keymod {
                        Mod::NOMOD => {
                            match scancode {
                                Scancode::LeftBracket => {
                                    let rot = (3 + context.display.rotation) % 4;
                                    ty.send(DeviceEvent::RotateScreen(rot)).ok();
                                },
                                Scancode::RightBracket => {
                                    let rot = (5 + context.display.rotation) % 4;
                                    ty.send(DeviceEvent::RotateScreen(rot)).ok();
                                },
                                Scancode::S => {
                                    tx.send(Event::Select(EntryId::TakeScreenshot)).ok();
                                },
                                Scancode::B | Scancode::F | Scancode::P | Scancode::L | Scancode::H |
                                    Scancode::E | Scancode::G => {
                                    if let Some(code) = code_from_key(scancode) {
                                        let status = if repeat {
                                            ButtonStatus::Repeated
                                        } else {
                                            ButtonStatus::Pressed
                                        };
                                        ty.send(DeviceEvent::Button {
                                            time: seconds(timestamp),
                                            code,
                                            status,
                                        }).ok();
                                    }
                                },
                                Scancode::I | Scancode::O => {
                                    let mouse_state = MouseState::new(&event_pump);
                                    let (window, logical) = geometry.get();
                                    let center = window_to_device(mouse_state.x(), mouse_state.y(), window, logical);
                                    if scancode == Scancode::I {
                                        tx.send(Event::Gesture(GestureEvent::Spread { center,
                                                                                      factor: 2.0,
                                                                                      axis: Axis::Diagonal })).ok();
                                    } else {
                                        tx.send(Event::Gesture(GestureEvent::Pinch { center,
                                                                                     factor: 0.5,
                                                                                     axis: Axis::Diagonal })).ok();
                                    }
                                },
                                _ => (),
                            }
                        },
                        Mod::LSHIFTMOD | Mod::RSHIFTMOD => {
                            match scancode {
                                Scancode::S | Scancode::P | Scancode::C => {
                                    if let Some(index) = locate::<Intermission>(view.as_ref()) {
                                        let rect = *view.child(index).rect();
                                        view.children_mut().remove(index);
                                        rq.add(RenderData::expose(rect, UpdateMode::Full));
                                    } else {
                                        view.handle_event(&Event::Suspend, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
                                        let kind = match scancode {
                                            Scancode::S => IntermKind::Suspend,
                                            Scancode::P => IntermKind::PowerOff,
                                            Scancode::C => IntermKind::Share,
                                            _ => unreachable!(),
                                        };
                                        let interm = Intermission::new(context.fb.rect(), kind, &context);
                                        rq.add(RenderData::new(interm.id(), *interm.rect(), UpdateMode::Full));
                                        view.children_mut().push(Box::new(interm) as Box<dyn View>);
                                    }
                                },
                                _ => (),
                            }
                        },
                        _ => (),
                    }
                },
                _ => {
                    if let Some(dev_evt) = device_event(sdl_evt, &geometry) {
                        ty.send(dev_evt).ok();
                    }
                },
            }
        }

        while let Ok(evt) = rx.recv_timeout(Duration::from_millis(20)) {
            match evt {
                Event::OpenArticle(id) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    match open_article(&mut context, id) {
                        OpenResult::Ready(article) => {
                            if let Some(r) = Reader::new_article(context.fb.rect(), &article, &tx, &mut context) {
                                let mut next_view = Box::new(r) as Box<dyn View>;
                                transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                                history.push(view as Box<dyn View>);
                                view = next_view;
                            } else {
                                plato_core::view::inoink::actions::release_article(&mut context, article.id, article.version);
                                handle_event(view.as_mut(), &Event::Invalid(Default::default()), &tx, &mut bus, &mut rq, &mut context);
                            }
                        },
                        OpenResult::Fetching => {
                            show_notification(view.as_mut(), "Downloading the article".to_string(), &tx, &mut rq, &mut context);
                        },
                        OpenResult::Unavailable(why) => {
                            show_notification(view.as_mut(), why, &tx, &mut rq, &mut context);
                            handle_event(view.as_mut(), &Event::Invalid(Default::default()), &tx, &mut bus, &mut rq, &mut context);
                        },
                    }
                },
                Event::OpenNext(id) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    if view.is::<Reader>() {
                        match open_article(&mut context, id) {
                            OpenResult::Ready(article) => {
                                // The current article saves its position and releases its lease.
                                handle_event(view.as_mut(), &Event::Back, &tx, &mut bus, &mut rq, &mut context);
                                if let Some(r) = Reader::new_article(context.fb.rect(), &article, &tx, &mut context) {
                                    let mut next_view = Box::new(r) as Box<dyn View>;
                                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                                    view = next_view;
                                    rq.add(RenderData::new(view.id(), context.fb.rect(), UpdateMode::Full));
                                } else {
                                    plato_core::view::inoink::actions::release_article(&mut context, article.id, article.version);
                                    show_notification(view.as_mut(), "This article could not be opened".to_string(), &tx, &mut rq, &mut context);
                                    tx.send(Event::Back).ok();
                                }
                            },
                            OpenResult::Fetching => {
                                context.inoink.opening = Some((id, plato_core::view::inoink::state::OpenMode::Replace));
                                context.inoink.opening_at = Some(std::time::Instant::now());
                                show_notification(view.as_mut(), "Downloading the next article".to_string(), &tx, &mut rq, &mut context);
                            },
                            OpenResult::Unavailable(why) => {
                                show_notification(view.as_mut(), why, &tx, &mut rq, &mut context);
                            },
                        }
                    }
                },
                Event::OpenWebPage(ref url) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    let why = if context.inoink.handle.is_none() {
                        Some("Sign in to open links".to_string())
                    } else if !context.online {
                        Some("Connect to Wi-Fi to open links".to_string())
                    } else {
                        None
                    };
                    match why {
                        Some(m) => {
                            show_notification(view.as_mut(), m, &tx, &mut rq, &mut context);
                        },
                        None => {
                            context.inoink.opening_page = Some(url.clone());
                            context.inoink.submit(plato_core::inoink::runtime::Command::FetchPage(url.clone()));
                            show_notification(view.as_mut(), "Loading the page".to_string(), &tx, &mut rq, &mut context);
                        },
                    }
                },
                Event::ShowDocument(id, kind) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    if view.is::<Reader>() {
                        match open_document(&mut context, id, kind) {
                            OpenResult::Ready(article) => {
                                // The current document saves its position and releases its lease.
                                handle_event(view.as_mut(), &Event::Back, &tx, &mut bus, &mut rq, &mut context);
                                if let Some(r) = Reader::new_article(context.fb.rect(), &article, &tx, &mut context) {
                                    let mut next_view = Box::new(r) as Box<dyn View>;
                                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                                    view = next_view;
                                    rq.add(RenderData::new(view.id(), context.fb.rect(), UpdateMode::Full));
                                } else {
                                    show_notification(view.as_mut(), "This document could not be opened".to_string(), &tx, &mut rq, &mut context);
                                }
                            },
                            OpenResult::Fetching => {
                                show_notification(view.as_mut(), "Downloading the comments".to_string(), &tx, &mut rq, &mut context);
                            },
                            OpenResult::Unavailable(why) => {
                                show_notification(view.as_mut(), why, &tx, &mut rq, &mut context);
                            },
                        }
                    }
                },
                Event::OpenDetails(id) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    let html = context.inoink.store().and_then(|s| s.article_record(id).ok().flatten().map(|r| plato_core::view::inoink::details::details_html(s, id, &r)));
                    if let Some(html) = html {
                        let r = Reader::from_html(context.fb.rect(), &html, None, &tx, &mut context);
                        let mut next_view = Box::new(r) as Box<dyn View>;
                        transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                        history.push(view as Box<dyn View>);
                        view = next_view;
                    }
                },
                Event::OpenImage(ref path) => {
                    let inside = path.canonicalize().ok().zip(context.inoink.cache.root().canonicalize().ok()).is_some_and(|(p, root)| p.starts_with(root));
                    if inside {
                        let iv = ImageView::new(context.fb.rect(), path.clone(), &mut context);
                        let next_view = Box::new(iv) as Box<dyn View>;
                        rq.add(RenderData::new(next_view.id(), context.fb.rect(), UpdateMode::Full));
                        history.push(view as Box<dyn View>);
                        view = next_view;
                    }
                },
                Event::Sync(ref u) => {
                    context.inoink.apply(u);
                    if let plato_core::inoink::runtime::Update::Cached(id, _) = u {
                        if let Some((open, mode)) = context.inoink.opening {
                            if open == *id {
                                context.inoink.opening = None;
                                context.inoink.opening_at = None;
                                tx.send(match mode {
                                    plato_core::view::inoink::state::OpenMode::Push => Event::OpenArticle(*id),
                                    plato_core::view::inoink::state::OpenMode::Replace => Event::OpenNext(*id),
                                }).ok();
                            }
                        }
                    }
                    if let plato_core::inoink::runtime::Update::Cycle(rep) = u {
                        if let Some(m) = plato_core::view::inoink::actions::share_message(rep) {
                            show_notification(view.as_mut(), m, &tx, &mut rq, &mut context);
                        }
                    }
                    if let plato_core::inoink::runtime::Update::PageReady(url, r) = u {
                        if context.inoink.opening_page.as_deref() == Some(url.as_str()) {
                            context.inoink.opening_page = None;
                            if let Ok(path) = r {
                                if let Some(r) = Reader::new_web(context.fb.rect(), path, url, &tx, &mut context) {
                                    let mut next_view = Box::new(r) as Box<dyn View>;
                                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                                    history.push(view as Box<dyn View>);
                                    view = next_view;
                                }
                            }
                        }
                    }
                    handle_event(view.as_mut(), &evt, &tx, &mut bus, &mut rq, &mut context);
                },
                Event::Open(info) => {
                    let rotation = context.display.rotation;
                    if let Some(n) = info.reader.as_ref()
                                         .and_then(|r| r.rotation.map(|n| CURRENT_DEVICE.from_canonical(n))) {
                        if n != rotation {
                            if let Ok(dims) = context.fb.set_rotation(n) {
                                context.display.rotation = n;
                                context.display.dims = dims;
                            }
                        }
                    }
                    let path = info.file.path.clone();
                    if let Some(r) = Reader::new(context.fb.rect(), *info, &tx, &mut context) {
                        let mut next_view = Box::new(r) as Box<dyn View>;
                        transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                        history.push(view as Box<dyn View>);
                        view = next_view;
                    } else {
                        if context.display.rotation != rotation {
                            if let Ok(dims) = context.fb.set_rotation(rotation) {
                                context.display.rotation = rotation;
                                context.display.dims = dims;
                            }
                        }
                        handle_event(view.as_mut(), &Event::Invalid(path), &tx, &mut bus, &mut rq, &mut context);
                    }
                },
                Event::OpenHtml(ref html, ref link_uri) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    let r = Reader::from_html(context.fb.rect(), html, link_uri.as_deref(), &tx, &mut context);
                    let mut next_view = Box::new(r) as Box<dyn View>;
                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                    history.push(view as Box<dyn View>);
                    view = next_view;
                },
                Event::Select(EntryId::Launch(app_cmd)) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    let mut next_view: Box<dyn View> = match app_cmd {
                        AppCmd::Calculator => {
                            Box::new(Calculator::new(context.fb.rect(), &tx, &mut rq, &mut context)?)
                        },
                        AppCmd::Dictionary { ref query, ref language } => {
                            Box::new(Dictionary::new(context.fb.rect(), query, language, &tx, &mut rq, &mut context))
                        },
                        AppCmd::TouchEvents => {
                            Box::new(TouchEvents::new(context.fb.rect(), &mut rq, &mut context))
                        },
                        AppCmd::RotationValues => {
                            Box::new(RotationValues::new(context.fb.rect(), &mut rq, &mut context))
                        },
                    };
                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                    history.push(view as Box<dyn View>);
                    view = next_view;
                },
                Event::Back => {
                    if let Some(v) = history.pop() {
                        view = v;
                        if view.is::<InoInkHome>() {
                            if context.display.rotation % 2 != 1 {
                                if let Ok(dims) = context.fb.set_rotation(DEFAULT_ROTATION) {
                                    context.display.rotation = DEFAULT_ROTATION;
                                    context.display.dims = dims;
                                }
                            }
                        }
                        view.handle_event(&Event::Reseed, &tx, &mut bus, &mut rq, &mut context);
                    }
                },
                Event::TogglePresetMenu(rect, index) => {
                    if let Some(index) = locate_by_id(view.as_ref(), ViewId::PresetMenu) {
                        let rect = *view.child(index).rect();
                        view.children_mut().remove(index);
                        rq.add(RenderData::expose(rect, UpdateMode::Gui));
                    } else {
                        let preset_menu = Menu::new(rect, ViewId::PresetMenu, MenuKind::Contextual,
                                                    vec![EntryKind::Command("Remove".to_string(),
                                                                            EntryId::RemovePreset(index))],
                                                    &mut context);
                        rq.add(RenderData::new(preset_menu.id(), *preset_menu.rect(), UpdateMode::Gui));
                        view.children_mut().push(Box::new(preset_menu) as Box<dyn View>);
                    }
                },
                Event::Show(ViewId::Frontlight) => {
                    if !context.settings.frontlight {
                        context.set_frontlight(true);
                        view.handle_event(&Event::ToggleFrontlight, &tx, &mut bus, &mut rq, &mut context);
                    }
                    let flw = LightPanel::new(context.fb.rect(), &context);
                    rq.add(RenderData::new(flw.id(), *flw.rect(), UpdateMode::Gui));
                    view.children_mut().push(Box::new(flw) as Box<dyn View>);
                },
                Event::ToggleFrontlight => {
                    context.set_frontlight(!context.settings.frontlight);
                    view.handle_event(&Event::ToggleFrontlight, &tx, &mut bus, &mut rq, &mut context);
                },
                Event::ToggleInputHistoryMenu(id, rect) => {
                    toggle_input_history_menu(view.as_mut(), id, rect, None, &mut rq, &mut context);
                },
                Event::ToggleNear(ViewId::KeyboardLayoutMenu, rect) => {
                    toggle_keyboard_layout_menu(view.as_mut(), rect, None, &mut rq, &mut context);
                },
                Event::Close(ViewId::Frontlight) => {
                    if let Some(index) = locate_by_id(view.as_ref(), ViewId::Frontlight) {
                        let rect = *view.child(index).rect();
                        view.children_mut().remove(index);
                        rq.add(RenderData::expose(rect, UpdateMode::Gui));
                    }
                },
                Event::Close(id) => {
                    if let Some(index) = locate_by_id(view.as_ref(), id) {
                        let rect = overlapping_rectangle(view.child(index));
                        rq.add(RenderData::expose(rect, UpdateMode::Gui));
                        view.children_mut().remove(index);
                    }
                },
                Event::Select(EntryId::About) => {
                    let dialog = Dialog::new(ViewId::AboutDialog,
                                             None,
                                             format!("InoInk {}", plato_core::inoink::VERSION),
                                             &mut context);
                    rq.add(RenderData::new(dialog.id(), *dialog.rect(), UpdateMode::Gui));
                    view.children_mut().push(Box::new(dialog) as Box<dyn View>);
                },
                Event::Select(EntryId::SystemInfo) => {
                    view.children_mut().retain(|child| !child.is::<Menu>());
                    let html = sys_info_as_html();
                    let r = Reader::from_html(context.fb.rect(), &html, None, &tx, &mut context);
                    let mut next_view = Box::new(r) as Box<dyn View>;
                    transfer_notifications(view.as_mut(), next_view.as_mut(), &mut rq, &mut context);
                    history.push(view as Box<dyn View>);
                    view = next_view;
                },
                Event::Select(EntryId::Rotate(n)) if n != context.display.rotation && view.might_rotate() => {
                    wait_for_all(&mut updating, &mut context);
                    if let Ok(dims) = context.fb.set_rotation(n) {
                        context.display.rotation = n;
                        let fb_rect = Rectangle::from(dims);
                        if context.display.dims != dims {
                            context.display.dims = dims;
                            view.resize(fb_rect, &tx, &mut rq, &mut context);
                        }
                    }
                },
                Event::Select(EntryId::SetButtonScheme(button_scheme)) => {
                    context.settings.button_scheme = button_scheme;
                },
                Event::Select(EntryId::ToggleInverted) => {
                    context.fb.toggle_inverted();
                    rq.add(RenderData::new(view.id(), context.fb.rect(), UpdateMode::Gui));
                },
                Event::Select(EntryId::TakeScreenshot) => {
                    let name = Local::now().format("screenshot-%Y%m%d_%H%M%S.png");
                    let msg = match save_screenshot(context.fb.as_ref(), opts.shots.as_deref(), &name.to_string()) {
                        Err(e) => format!("Couldn't take screenshot: {}).", e),
                        Ok(_) => format!("Saved {}.", name),
                    };
                    show_notification(view.as_mut(), msg, &tx, &mut rq, &mut context);
                },
                Event::Notify(msg) => {
                    show_notification(view.as_mut(), msg, &tx, &mut rq, &mut context);
                },
                Event::Device(DeviceEvent::NetUp) if opts.online && !context.online => {
                    context.online = true;
                    context.inoink.lifecycle(plato_core::inoink::runtime::Lifecycle::NetworkUp);
                    tx.send(Event::NetworkChanged).ok();
                    if view.is::<InoInkHome>() {
                        handle_event(view.as_mut(), &evt, &tx, &mut bus, &mut rq, &mut context);
                    } else if let Some(home) = history.get_mut(0).filter(|view| view.is::<InoInkHome>()) {
                        let (tx, _rx) = mpsc::channel();
                        home.handle_event(&evt, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
                    }
                },
                Event::Device(DeviceEvent::NetUp) if !view.is::<InoInkHome>() => {
                    if let Some(home) = history.get_mut(0).filter(|view| view.is::<InoInkHome>()) {
                        let (tx, _rx) = mpsc::channel();
                        home.handle_event(&evt, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
                    }
                },
                Event::SetWifi(enable) => {
                    if context.settings.wifi != enable {
                        context.settings.wifi = enable;
                        if enable {
                            let tx2 = tx.clone();
                            thread::spawn(move || {
                                thread::sleep(Duration::from_secs(2));
                                tx2.send(Event::Device(DeviceEvent::NetUp)).ok();
                            });
                        } else {
                            context.online = false;
                            if opts.online {
                                context.inoink.lifecycle(plato_core::inoink::runtime::Lifecycle::NetworkDown);
                            }
                        }
                    }
                },
                Event::Device(DeviceEvent::RotateScreen(n)) => {
                    tx.send(Event::Select(EntryId::Rotate(n))).ok();
                },
                Event::Select(EntryId::Quit) => {
                    break 'outer;
                },
                _ => {
                    handle_event(view.as_mut(), &evt, &tx, &mut bus, &mut rq, &mut context);
                },
            }
        }

        process_render_queue(view.as_ref(), &mut rq, &mut context, &mut updating);

        // Scripted tour: replays taps and takes screenshots for emulator verification.
        if !tour.is_empty() && std::time::Instant::now() >= tour_next {
            match tour.pop_front().unwrap() {
                TourStep::Wait(ms) => tour_next = std::time::Instant::now() + Duration::from_millis(ms),
                TourStep::Tap(x, y) => {
                    tx.send(Event::Gesture(GestureEvent::Tap(pt!(x, y)))).ok();
                    tour_next = std::time::Instant::now() + Duration::from_millis(900);
                },
                TourStep::Send(e) => {
                    tx.send(e).ok();
                    tour_next = std::time::Instant::now() + Duration::from_millis(900);
                },
                TourStep::Shot(name) => {
                    if let Some(dir) = &opts.shots {
                        let path = format!("{}/{}.png", dir, name);
                        context.fb.save(&path).map_err(|e| eprintln!("screenshot {}: {:#}", path, e)).ok();
                        println!("screenshot {}", path);
                    }
                    tour_next = std::time::Instant::now() + Duration::from_millis(200);
                },
                TourStep::Quit => break 'outer,
            }
        }

        while let Some(ce) = bus.pop_front() {
            tx.send(ce).ok();
        }
    }

    if !history.is_empty() {
        let (tx, _rx) = mpsc::channel();
        view.handle_event(&Event::Back, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
        while let Some(mut view) = history.pop() {
            view.handle_event(&Event::Back, &tx, &mut VecDeque::new(), &mut RenderQueue::new(), &mut context);
        }
    }

    if context.settings.frontlight {
        context.settings.frontlight_levels = context.frontlight.levels();
    }

    if let Some(h) = context.inoink.handle.take() {
        h.shutdown();
    }
    if let Some(store) = context.inoink.store() {
        store.checkpoint().ok();
    }

    let path = settings_path();
    save_toml(&context.settings, path.as_path()).context("can't save settings")?;

    Ok(())
}

enum TourStep {
    Wait(u64),
    Tap(i32, i32),
    Send(Event),
    Shot(&'static str),
    Quit,
}

/// A deterministic walk through the tabs, an article, the next article, the QR code, the light
/// panel and the menu.
fn scripted_tour(rect: Rectangle) -> VecDeque<TourStep> {
    use plato_core::inoink::model::{ArticleId, Collection};
    use plato_core::view::inoink::state::Tab;
    let w = rect.width() as i32;
    let dpi = CURRENT_DEVICE.dpi as f32 / 300.0;
    let bar = (121.0 * dpi) as i32;
    let row = (163.0 * dpi) as i32;
    // Top bar and tab bar, then rows of `row` pixels.
    let row_y = |n: i32| 2 * bar + n * row + row / 2;
    let select = |e: InoInkEntry| TourStep::Send(Event::Select(EntryId::InoInk(e)));
    let mut t = VecDeque::new();
    t.push_back(TourStep::Wait(1500));
    t.push_back(TourStep::Shot("01-newsfeed"));
    t.push_back(TourStep::Tap(w - (48.0 * dpi) as i32, row_y(2)));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("02-saved-by-tap"));
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::HoldFingerShort(pt!(w / 2, row_y(3)), 0))));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("02b-row-menu"));
    t.push_back(select(InoInkEntry::CancelSelection));
    t.push_back(select(InoInkEntry::Tab(Tab::Categories)));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("03-categories"));
    t.push_back(select(InoInkEntry::OpenCategory(Collection::Folder("Reading".into()))));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("04-folder"));
    t.push_back(select(InoInkEntry::Tab(Tab::ReadLater)));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("05-read-later"));
    t.push_back(TourStep::Tap(w / 2, row_y(0)));
    t.push_back(TourStep::Wait(1500));
    t.push_back(TourStep::Shot("06-article"));
    t.push_back(TourStep::Send(Event::OpenDetails(ArticleId(1000))));
    t.push_back(TourStep::Wait(1200));
    t.push_back(TourStep::Shot("06b-details"));
    t.push_back(TourStep::Send(Event::Back));
    t.push_back(TourStep::Wait(1000));
    // Hold a word of paragraph 3 (a point on its first line at this geometry), lift the finger
    // for the selection menu, then Highlight.
    let word = pt!(370 * w / 1264, 1062 * rect.height() as i32 / 1680);
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::HoldFingerShort(word, 0))));
    t.push_back(TourStep::Send(Event::Device(DeviceEvent::Finger { id: 0, time: 0.0, status: FingerStatus::Up, position: word })));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("06c-selection-menu"));
    t.push_back(TourStep::Send(Event::Select(EntryId::HighlightSelection)));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("06d-highlight"));
    // The selection menu is still open (the tour chose Highlight without tapping it): the first
    // tap closes it, the second shows the bar.
    t.push_back(TourStep::Tap(w / 2, rect.height() as i32 / 2));
    t.push_back(TourStep::Wait(500));
    t.push_back(TourStep::Tap(w / 2, rect.height() as i32 / 2));
    t.push_back(TourStep::Wait(500));
    t.push_back(TourStep::Shot("07-article-bar-footer"));
    let h = rect.height() as i32;
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::Swipe { dir: Dir::North, start: pt!(w / 2, 3 * h / 4), end: pt!(w / 2, h / 4) })));
    t.push_back(TourStep::Wait(1500));
    t.push_back(TourStep::Shot("09-next-article"));
    // Jump to the last page, then page past it: the end popup opens.
    t.push_back(TourStep::Send(Event::GoTo(usize::MAX)));
    t.push_back(TourStep::Wait(500));
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::Swipe { dir: Dir::West, start: pt!(3 * w / 4, h / 2), end: pt!(w / 4, h / 2) })));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("09b-article-end"));
    t.push_back(TourStep::Send(Event::Close(ViewId::ArticleEnd)));
    t.push_back(TourStep::Wait(500));
    t.push_back(select(InoInkEntry::ShowQr("https://news.example/issue/42".into())));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("10-qr"));
    t.push_back(TourStep::Send(Event::Close(ViewId::QrView)));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Send(Event::Show(ViewId::Frontlight)));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("11-light-panel"));
    t.push_back(TourStep::Send(Event::Close(ViewId::Frontlight)));
    t.push_back(TourStep::Wait(500));
    t.push_back(TourStep::Send(Event::Back));
    t.push_back(TourStep::Wait(1000));
    t.push_back(TourStep::Tap(w - bar / 2, bar / 2));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("12-main-menu"));
    t.push_back(TourStep::Tap(w - bar / 2, bar / 2));
    t.push_back(TourStep::Wait(400));
    t.push_back(select(InoInkEntry::Tab(Tab::Newsfeed)));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("13-end-of-unread"));
    t.push_back(TourStep::Send(Event::OpenArticle(ArticleId(1005))));
    t.push_back(TourStep::Wait(1000));
    t.push_back(TourStep::Send(Event::Toggle(ViewId::TopBottomBars)));
    t.push_back(TourStep::Wait(400));
    t.push_back(TourStep::Shot("14-linked-article-tabs"));
    t.push_back(TourStep::Send(Event::ShowDocument(ArticleId(1005), plato_core::view::inoink::actions::DocKind::Comments)));
    t.push_back(TourStep::Wait(1000));
    t.push_back(TourStep::Send(Event::Toggle(ViewId::TopBottomBars)));
    t.push_back(TourStep::Wait(400));
    t.push_back(TourStep::Shot("15-linked-comments-tabs"));
    t.push_back(TourStep::Quit);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use plato_core::framebuffer::Pixmap;
    use sdl2::mouse::MouseButton;

    fn options(args: &[&str]) -> Result<Options, Error> {
        parse_options_from(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn scale_defaults_and_existing_options() -> Result<(), Error> {
        assert_eq!(options(&[])?.scale, 1.0);
        let opts = options(&[
            "--data",
            "sample data",
            "--online",
            "--scripted-tour",
            "--screenshots",
            "sample shots",
            "--scale",
            "0.5",
        ])?;
        assert_eq!(opts.scale, 0.5);
        assert_eq!(opts.data.as_deref(), Some("sample data"));
        assert_eq!(opts.shots.as_deref(), Some("sample shots"));
        assert!(opts.online && opts.tour);
        for value in ["0.1", "0.5", "1", "2.0"] {
            assert!(parse_scale(value).is_ok(), "{}", value);
        }
        Ok(())
    }

    #[test]
    fn malformed_scale_is_an_error() {
        for value in [
            "", "invalid", "NaN", "inf", "-inf", "Infinity", "1e999", "0", "-0", "-1", "0.099",
            "2.001", "--online",
        ] {
            let error = parse_scale(value).unwrap_err().to_string();
            assert!(error.contains("--scale"), "{}", error);
            assert!(options(&["--scale", value]).is_err(), "{}", value);
        }
        let error = options(&["--scale"]).err().unwrap().to_string();
        assert!(error.contains("--scale requires"), "{}", error);
    }

    #[test]
    fn mouse_events_use_shared_geometry() {
        let geometry = Rc::new(Cell::new(((632, 840), (1264, 1680))));
        let events = [
            SdlEvent::MouseButtonDown {
                timestamp: 1000,
                window_id: 0,
                which: 0,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 631,
                y: 839,
            },
            SdlEvent::MouseButtonUp {
                timestamp: 1000,
                window_id: 0,
                which: 0,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 631,
                y: 839,
            },
            SdlEvent::MouseMotion {
                timestamp: 1000,
                window_id: 0,
                which: 0,
                mousestate: MouseState::from_sdl_state(0),
                x: 631,
                y: 839,
                xrel: 1,
                yrel: 1,
            },
        ];
        for (event, expected_status) in
            events
                .into_iter()
                .zip([FingerStatus::Down, FingerStatus::Up, FingerStatus::Motion])
        {
            match device_event(event.clone(), &geometry).unwrap() {
                DeviceEvent::Finger {
                    position,
                    status,
                    time,
                    ..
                } => {
                    assert_eq!(position, pt!(1262, 1678));
                    assert_eq!(status, expected_status);
                    assert_eq!(time, 1.0);
                }
                other => panic!("unexpected event: {:?}", other),
            }
            geometry.set(((840, 632), (1680, 1264)));
            match device_event(event, &geometry).unwrap() {
                DeviceEvent::Finger { position, .. } => assert_eq!(position, pt!(1262, 1263)),
                other => panic!("unexpected event: {:?}", other),
            }
            geometry.set(((632, 840), (1264, 1680)));
        }
    }

    #[test]
    fn manual_screenshot_uses_requested_directory() -> Result<(), Error> {
        let root = std::env::temp_dir().join(format!("inoink-manual-shots-{}", std::process::id()));
        let directory = root.join("isolated shots");
        let name = "screenshot-20000101_000000.png";
        let fb = Pixmap::new(1264, 1680, 3);
        save_screenshot(&fb, directory.to_str(), name)?;
        let saved = Pixmap::from_png(directory.join(name))?;
        assert_eq!(saved.dims(), (1264, 1680));
        assert!(!root.join(name).exists());
        // With no override, the old path is passed through unchanged.
        let legacy_path = root.join("legacy.png");
        save_screenshot(&fb, None, legacy_path.to_str().unwrap())?;
        assert!(legacy_path.is_file());
        let blocked = directory.join(name);
        assert!(save_screenshot(&fb, blocked.to_str(), name).is_err());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
