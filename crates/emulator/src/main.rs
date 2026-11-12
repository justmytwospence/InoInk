use std::mem;
use std::thread;
use std::fs::File;
use std::sync::mpsc;
use std::collections::VecDeque;
use std::time::Duration;
use plato_core::anyhow::{Error, Context as ResultExt};
use plato_core::chrono::Local;
use sdl2::event::Event as SdlEvent;
use sdl2::keyboard::{Scancode, Keycode, Mod};
use sdl2::render::{WindowCanvas, BlendMode};
use sdl2::pixels::{Color as SdlColor, PixelFormatEnum};
use sdl2::mouse::MouseState;
use sdl2::rect::Point as SdlPoint;
use sdl2::rect::Rect as SdlRect;
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
use plato_core::view::notification::Notification;
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
use plato_core::geom::{Rectangle, Axis};
use plato_core::color::Color;
use plato_core::gesture::{GestureEvent, gesture_events};
use plato_core::device::CURRENT_DEVICE;
use plato_core::battery::{Battery, FakeBattery};
use plato_core::frontlight::{Frontlight, LightLevels};
use plato_core::lightsensor::LightSensor;
use plato_core::font::Fonts;
use plato_core::context::Context;
use plato_core::pt;
use plato_core::png;

pub const APP_NAME: &str = "InoInk";
const DEFAULT_ROTATION: i8 = 1;

const CLOCK_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub struct Options {
    data: Option<String>,
    online: bool,
    tour: bool,
    shots: Option<String>,
}

fn parse_options() -> Options {
    let mut o = Options { data: None, online: false, tour: false, shots: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => o.data = args.next(),
            "--online" => o.online = true,
            "--scripted-tour" => o.tour = true,
            "--screenshots" => o.shots = args.next(),
            _ => {}
        }
    }
    o
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
pub fn device_event(event: SdlEvent) -> Option<DeviceEvent> {
    match event {
        SdlEvent::MouseButtonDown { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Down,
                                       position: pt!(x, y),
                                       time: seconds(timestamp) }),
        SdlEvent::MouseButtonUp { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Up,
                                       position: pt!(x, y),
                                       time: seconds(timestamp) }),
        SdlEvent::MouseMotion { timestamp, x, y, .. } =>
            Some(DeviceEvent::Finger { id: 0,
                                       status: FingerStatus::Motion,
                                       position: pt!(x, y),
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

struct FBCanvas(WindowCanvas);

impl Framebuffer for FBCanvas {
    fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        let [red, green, blue] = color.rgb();
        self.0.set_draw_color(SdlColor::RGB(red, green, blue));
        self.0.draw_point(SdlPoint::new(x as i32, y as i32)).unwrap();
    }

    fn set_blended_pixel(&mut self, x: u32, y: u32, color: Color, alpha: f32) {
        let [red, green, blue] = color.rgb();
        self.0.set_draw_color(SdlColor::RGBA(red, green, blue, (alpha * 255.0) as u8));
        self.0.draw_point(SdlPoint::new(x as i32, y as i32)).unwrap();
    }

    fn invert_region(&mut self, rect: &Rectangle) {
        let width = rect.width();
        let s_rect = Some(SdlRect::new(rect.min.x, rect.min.y,
                                       width, rect.height()));
        if let Ok(data) = self.0.read_pixels(s_rect, PixelFormatEnum::RGB24) {
            for y in rect.min.y..rect.max.y {
                let v = (y - rect.min.y) as u32;
                for x in rect.min.x..rect.max.x {
                    let u = (x - rect.min.x) as u32;
                    let addr = 3 * (v * width + u);
                    let red = data[addr as usize];
                    let green = data[(addr+1) as usize];
                    let blue = data[(addr+2) as usize];
                    let mut color = Color::Rgb(red, green, blue);
                    color.invert();
                    self.set_pixel(x as u32, y as u32, color);
                }
            }
        }
    }

    fn shift_region(&mut self, rect: &Rectangle, drift: u8) {
        let width = rect.width();
        let s_rect = Some(SdlRect::new(rect.min.x, rect.min.y,
                                       width, rect.height()));
        if let Ok(data) = self.0.read_pixels(s_rect, PixelFormatEnum::RGB24) {
            for y in rect.min.y..rect.max.y {
                let v = (y - rect.min.y) as u32;
                for x in rect.min.x..rect.max.x {
                    let u = (x - rect.min.x) as u32;
                    let addr = 3 * (v * width + u);
                    let red = data[addr as usize];
                    let green = data[(addr+1) as usize];
                    let blue = data[(addr+2) as usize];
                    let mut color = Color::Rgb(red, green, blue);
                    color.shift(drift);
                    self.set_pixel(x as u32, y as u32, color);
                }
            }
        }
    }

    fn update(&mut self, _rect: &Rectangle, _mode: UpdateMode) -> Result<u32, Error> {
        self.0.present();
        Ok(Local::now().timestamp_subsec_millis())
    }

    fn wait(&self, _tok: u32) -> Result<i32, Error> {
        Ok(1)
    }

    fn save(&self, path: &str) -> Result<(), Error> {
        let (width, height) = self.dims();
        let file = File::create(path).with_context(|| format!("can't create output file {}", path))?;
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().with_context(|| format!("can't write PNG header for {}", path))?;
        let data = self.0.read_pixels(self.0.viewport(), PixelFormatEnum::RGB24).unwrap_or_default();
        writer.write_image_data(&data).with_context(|| format!("can't write PNG data to {}", path))?;
        Ok(())
    }

    fn rotation(&self) -> i8 {
        DEFAULT_ROTATION
    }

    fn set_rotation(&mut self, n: i8) -> Result<(u32, u32), Error> {
        let (mut width, mut height) = self.dims();
        if (width < height && n % 2 == 0) || (width > height && n % 2 == 1) {
            mem::swap(&mut width, &mut height);
        }
        self.0.window_mut().set_size(width, height).ok();
        Ok((width, height))
    }

    fn set_monochrome(&mut self, _enable: bool) {
    }

    fn set_dithered(&mut self, _enable: bool) {
    }

    fn set_inverted(&mut self, _enable: bool) {
    }

    fn monochrome(&self) -> bool {
        false
    }

    fn dithered(&self) -> bool {
        false
    }

    fn inverted(&self) -> bool {
        false
    }

    fn width(&self) -> u32 {
        self.0.window().size().0
    }

    fn height(&self) -> u32 {
        self.0.window().size().1
    }
}

fn main() -> Result<(), Error> {
    let sdl_context = sdl2::init().unwrap();
    let video_subsystem = sdl_context.video().unwrap();
    let (width, height) = CURRENT_DEVICE.dims;
    let window = video_subsystem
                 .window("InoInk Emulator", width, height)
                 .position_centered()
                 .build()
                 .unwrap();

    let mut fb = window.into_canvas().software().build().unwrap();
    fb.set_blend_mode(BlendMode::Blend);

    let opts = parse_options();
    let (tx, rx) = mpsc::channel();
    let mut context = build_context(Box::new(FBCanvas(fb)), &opts, &tx)?;

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
                                    let x = mouse_state.x() as i32;
                                    let y = mouse_state.y() as i32;
                                    let center = pt!(x, y);
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
                    if let Some(dev_evt) = device_event(sdl_evt) {
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
                            let notif = Notification::new("Downloading the article".to_string(), &tx, &mut rq, &mut context);
                            view.children_mut().push(Box::new(notif) as Box<dyn View>);
                        },
                        OpenResult::Unavailable(why) => {
                            let notif = Notification::new(why, &tx, &mut rq, &mut context);
                            view.children_mut().push(Box::new(notif) as Box<dyn View>);
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
                                    let notif = Notification::new("This article could not be opened".to_string(), &tx, &mut rq, &mut context);
                                    view.children_mut().push(Box::new(notif) as Box<dyn View>);
                                    tx.send(Event::Back).ok();
                                }
                            },
                            OpenResult::Fetching => {
                                context.inoink.opening = Some((id, plato_core::view::inoink::state::OpenMode::Replace));
                                let notif = Notification::new("Downloading the next article".to_string(), &tx, &mut rq, &mut context);
                                view.children_mut().push(Box::new(notif) as Box<dyn View>);
                            },
                            OpenResult::Unavailable(why) => {
                                let notif = Notification::new(why, &tx, &mut rq, &mut context);
                                view.children_mut().push(Box::new(notif) as Box<dyn View>);
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
                            let notif = Notification::new(m, &tx, &mut rq, &mut context);
                            view.children_mut().push(Box::new(notif) as Box<dyn View>);
                        },
                        None => {
                            context.inoink.opening_page = Some(url.clone());
                            context.inoink.submit(plato_core::inoink::runtime::Command::FetchPage(url.clone()));
                            let notif = Notification::new("Loading the page".to_string(), &tx, &mut rq, &mut context);
                            view.children_mut().push(Box::new(notif) as Box<dyn View>);
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
                                    let notif = Notification::new("This document could not be opened".to_string(), &tx, &mut rq, &mut context);
                                    view.children_mut().push(Box::new(notif) as Box<dyn View>);
                                }
                            },
                            OpenResult::Fetching => {
                                let notif = Notification::new("Downloading the comments".to_string(), &tx, &mut rq, &mut context);
                                view.children_mut().push(Box::new(notif) as Box<dyn View>);
                            },
                            OpenResult::Unavailable(why) => {
                                let notif = Notification::new(why, &tx, &mut rq, &mut context);
                                view.children_mut().push(Box::new(notif) as Box<dyn View>);
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
                                tx.send(match mode {
                                    plato_core::view::inoink::state::OpenMode::Push => Event::OpenArticle(*id),
                                    plato_core::view::inoink::state::OpenMode::Replace => Event::OpenNext(*id),
                                }).ok();
                            }
                        }
                    }
                    if let plato_core::inoink::runtime::Update::Cycle(rep) = u {
                        if let Some(m) = plato_core::view::inoink::actions::share_message(rep) {
                            let notif = Notification::new(m, &tx, &mut rq, &mut context);
                            view.children_mut().push(Box::new(notif) as Box<dyn View>);
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
                    let msg = match context.fb.save(&name.to_string()) {
                        Err(e) => format!("Couldn't take screenshot: {}).", e),
                        Ok(_) => format!("Saved {}.", name),
                    };
                    let notif = Notification::new(msg, &tx, &mut rq, &mut context);
                    view.children_mut().push(Box::new(notif) as Box<dyn View>);
                },
                Event::Notify(msg) => {
                    let notif = Notification::new(msg, &tx, &mut rq, &mut context);
                    view.children_mut().push(Box::new(notif) as Box<dyn View>);
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

/// A deterministic walk through the tabs, an article, its end card, the QR code, the light
/// panel and the menu.
fn scripted_tour(rect: Rectangle) -> VecDeque<TourStep> {
    use plato_core::inoink::model::Collection;
    use plato_core::view::inoink::actions::FinishAction;
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
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::HoldFingerShort(pt!(w / 2, row_y(2)), 0))));
    t.push_back(TourStep::Wait(600));
    t.push_back(TourStep::Shot("02-saved-by-hold"));
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
    t.push_back(TourStep::Tap(w / 2, rect.height() as i32 / 2));
    t.push_back(TourStep::Wait(500));
    t.push_back(TourStep::Shot("07-article-bar-footer"));
    t.push_back(TourStep::Send(Event::Gesture(GestureEvent::HoldButtonShort(ButtonCode::Forward))));
    t.push_back(TourStep::Wait(800));
    t.push_back(TourStep::Shot("08-end-card"));
    t.push_back(select(InoInkEntry::Finish(FinishAction::Primary)));
    t.push_back(TourStep::Wait(1500));
    t.push_back(TourStep::Shot("09-next-article"));
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
    t.push_back(TourStep::Quit);
    t
}
