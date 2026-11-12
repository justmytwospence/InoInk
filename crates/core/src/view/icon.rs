use fxhash::FxHashMap;
use lazy_static::lazy_static;
use crate::device::CURRENT_DEVICE;
use crate::framebuffer::{Framebuffer, Pixmap, UpdateMode};
use super::{View, Event, Hub, Bus, Id, ID_FEEDER, RenderQueue, RenderData, ViewId, Align};
use crate::gesture::GestureEvent;
use crate::input::{DeviceEvent, FingerStatus};
use crate::document::pdf::PdfOpener;
use crate::color::{Color, TEXT_NORMAL, TEXT_INVERTED_HARD};
use crate::unit::scale_by_dpi_raw;
use crate::geom::{Rectangle, CornerSpec};
use crate::font::Fonts;
use crate::context::Context;

const ICON_SCALE: f32 = 1.0 / 32.0;

/// Icons are compiled into the program. Loose files on the Kobo's FAT storage proved fragile:
/// its disk check renamed one of them on every mount after a copy from macOS.
const EMBEDDED_ICONS: &[(&str, &[u8])] = &[
    ("home", include_bytes!("../../../../icons/home.svg")),
    ("search", include_bytes!("../../../../icons/search.svg")),
    ("back", include_bytes!("../../../../icons/back.svg")),
    ("frontlight", include_bytes!("../../../../icons/frontlight.svg")),
    ("frontlight-disabled", include_bytes!("../../../../icons/frontlight-disabled.svg")),
    ("menu", include_bytes!("../../../../icons/menu.svg")),
    ("angle-left", include_bytes!("../../../../icons/angle-left.svg")),
    ("angle-right", include_bytes!("../../../../icons/angle-right.svg")),
    ("angle-left-small", include_bytes!("../../../../icons/angle-left-small.svg")),
    ("angle-right-small", include_bytes!("../../../../icons/angle-right-small.svg")),
    ("return", include_bytes!("../../../../icons/return.svg")),
    ("shift", include_bytes!("../../../../icons/shift.svg")),
    ("combine", include_bytes!("../../../../icons/combine.svg")),
    ("alternate", include_bytes!("../../../../icons/alternate.svg")),
    ("delete-backward", include_bytes!("../../../../icons/delete-backward.svg")),
    ("delete-forward", include_bytes!("../../../../icons/delete-forward.svg")),
    ("move-backward", include_bytes!("../../../../icons/move-backward.svg")),
    ("move-backward-short", include_bytes!("../../../../icons/move-backward-short.svg")),
    ("move-forward", include_bytes!("../../../../icons/move-forward.svg")),
    ("move-forward-short", include_bytes!("../../../../icons/move-forward-short.svg")),
    ("close", include_bytes!("../../../../icons/close.svg")),
    ("check_mark-small", include_bytes!("../../../../icons/check_mark-small.svg")),
    ("check_mark", include_bytes!("../../../../icons/check_mark.svg")),
    ("check_mark-large", include_bytes!("../../../../icons/check_mark-large.svg")),
    ("bullet", include_bytes!("../../../../icons/bullet.svg")),
    ("arrow-left", include_bytes!("../../../../icons/arrow-left.svg")),
    ("arrow-right", include_bytes!("../../../../icons/arrow-right.svg")),
    ("angle-down", include_bytes!("../../../../icons/angle-down.svg")),
    ("angle-up", include_bytes!("../../../../icons/angle-up.svg")),
    ("crop", include_bytes!("../../../../icons/crop.svg")),
    ("toc", include_bytes!("../../../../icons/toc.svg")),
    ("font_family", include_bytes!("../../../../icons/font_family.svg")),
    ("font_size", include_bytes!("../../../../icons/font_size.svg")),
    ("line_height", include_bytes!("../../../../icons/line_height.svg")),
    ("align-justify", include_bytes!("../../../../icons/align-justify.svg")),
    ("align-left", include_bytes!("../../../../icons/align-left.svg")),
    ("align-right", include_bytes!("../../../../icons/align-right.svg")),
    ("align-center", include_bytes!("../../../../icons/align-center.svg")),
    ("margin", include_bytes!("../../../../icons/margin.svg")),
    ("plug", include_bytes!("../../../../icons/plug.svg")),
    ("cover", include_bytes!("../../../../icons/cover.svg")),
    ("enclosed_menu", include_bytes!("../../../../icons/enclosed_menu.svg")),
    ("contrast", include_bytes!("../../../../icons/contrast.svg")),
    ("gray", include_bytes!("../../../../icons/gray.svg")),
    ("star", include_bytes!("../../../../icons/star.svg")),
    ("star-filled", include_bytes!("../../../../icons/star-filled.svg")),
    ("bookmark", include_bytes!("../../../../icons/bookmark.svg")),
    ("bookmark-filled", include_bytes!("../../../../icons/bookmark-filled.svg")),
    ("archive", include_bytes!("../../../../icons/archive.svg")),
    ("more", include_bytes!("../../../../icons/more.svg")),
    ("sync", include_bytes!("../../../../icons/sync.svg")),
    ("tag", include_bytes!("../../../../icons/tag.svg")),
    ("wifi", include_bytes!("../../../../icons/wifi.svg")),
    ("wifi-connecting", include_bytes!("../../../../icons/wifi-connecting.svg")),
    ("wifi-off", include_bytes!("../../../../icons/wifi-off.svg")),
];

lazy_static! {
    pub static ref ICONS_PIXMAPS: FxHashMap<&'static str, Pixmap> = {
        let mut m = FxHashMap::default();
        let scale = scale_by_dpi_raw(ICON_SCALE, CURRENT_DEVICE.dpi);
        for (name, svg) in EMBEDDED_ICONS {
            let pixmap = PdfOpener::new()
                .and_then(|o| o.open_memory("svg", svg))
                .and_then(|doc| doc.page(0).and_then(|p| p.pixmap(scale, 1)));
            match pixmap {
                Some(p) => {
                    m.insert(*name, p);
                }
                None => eprintln!("Can't render icon {}.", name),
            }
        }
        m
    };
}

pub struct Icon {
    id: Id,
    pub rect: Rectangle,
    children: Vec<Box<dyn View>>,
    pub name: String,
    background: Color,
    align: Align,
    corners: Option<CornerSpec>,
    event: Event,
    pub active: bool,
}

impl Icon {
    pub fn new(name: &str, rect: Rectangle, event: Event) -> Icon {
        Icon {
            id: ID_FEEDER.next(),
            rect,
            children: Vec::new(),
            name: name.to_string(),
            background: TEXT_NORMAL[0],
            align: Align::Center,
            corners: None,
            event,
            active: false,
        }
    }

    pub fn background(mut self, background: Color) -> Icon {
        self.background = background;
        self
    }

    pub fn align(mut self, align: Align) -> Icon {
        self.align = align;
        self
    }

    pub fn corners(mut self, corners: Option<CornerSpec>) -> Icon {
        self.corners = corners;
        self
    }
}

impl View for Icon {
    fn handle_event(&mut self, evt: &Event, hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Device(DeviceEvent::Finger { status, position, .. }) => {
                match status {
                    FingerStatus::Down if self.rect.includes(position) => {
                        self.active = true;
                        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Fast));
                        true
                    },
                    FingerStatus::Up if self.active => {
                        self.active = false;
                        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                        true
                    },
                    _ => false,
                }
            },
            Event::Gesture(GestureEvent::Tap(center)) if self.rect.includes(center) => {
                bus.push_back(self.event.clone());
                true
            },
            Event::Gesture(GestureEvent::HoldFingerShort(center, ..)) if self.rect.includes(center) => {
                match self.event {
                    Event::Page(dir) => bus.push_back(Event::Chapter(dir)),
                    Event::Show(ViewId::Frontlight) => {
                        hub.send(Event::ToggleFrontlight).ok();
                    },
                    Event::Show(ViewId::MarginCropper) => {
                        bus.push_back(Event::ToggleNear(ViewId::MarginCropperMenu, self.rect));
                    },
                    Event::History(dir, false) => {
                        bus.push_back(Event::History(dir, true));
                    },
                    _ => (),
                }
                true
            },
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {
        let scheme = if self.active {
            TEXT_INVERTED_HARD
        } else {
            TEXT_NORMAL
        };

        let Some(pixmap) = ICONS_PIXMAPS.get(&self.name[..]) else {
            fb.draw_rectangle(&self.rect, self.background);
            return;
        };
        let dx = self.align.offset(pixmap.width as i32, self.rect.width() as i32);
        let dy = (self.rect.height() as i32 - pixmap.height as i32) / 2;
        let pt = self.rect.min + pt!(dx, dy);

        let background = if self.active {
            scheme[0]
        } else {
            self.background
        };

        if let Some(ref cs) = self.corners {
            fb.draw_rounded_rectangle(&self.rect, cs, background);
        } else {
            fb.draw_rectangle(&self.rect, background);
        }

        fb.draw_blended_pixmap(pixmap, pt, scheme[1]);
    }

    fn resize(&mut self, rect: Rectangle, _hub: &Hub, _rq: &mut RenderQueue, _context: &mut Context) {
        if let Event::ToggleNear(_, ref mut event_rect) = self.event {
            *event_rect = rect;
        }
        self.rect = rect;
    }

    fn rect(&self) -> &Rectangle {
        &self.rect
    }

    fn rect_mut(&mut self) -> &mut Rectangle {
        &mut self.rect
    }

    fn children(&self) -> &Vec<Box<dyn View>> {
        &self.children
    }

    fn children_mut(&mut self) -> &mut Vec<Box<dyn View>> {
        &mut self.children
    }

    fn id(&self) -> Id {
        self.id
    }
}
