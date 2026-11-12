//! A full-screen QR code for an address, to open it on a phone.

use crate::color::{BLACK, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, LIST_META, NORMAL_STYLE};
use crate::framebuffer::Framebuffer;
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::view::{Bus, Event, Hub, Id, RenderQueue, View, ViewId, ID_FEEDER};
use qrcode::{Color, QrCode};

pub struct QrView {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    url: String,
    width: usize,
    dark: Vec<bool>,
}

impl QrView {
    /// `None` when the address is too long for a QR code.
    pub fn new(rect: Rectangle, url: &str) -> Option<QrView> {
        let code = QrCode::new(url.as_bytes()).ok()?;
        let width = code.width();
        let dark = code.to_colors().into_iter().map(|c| c == Color::Dark).collect();
        Some(QrView { id: ID_FEEDER.next(), rect, children: Vec::new(), url: url.to_string(), width, dark })
    }
}

impl View for QrView {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                bus.push_back(Event::Close(ViewId::QrView));
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) | Event::Gesture(GestureEvent::HoldFingerLong(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, .. }) if self.rect.includes(start) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        let (w, h) = (self.rect.width() as i32, self.rect.height() as i32);
        // Four modules of quiet zone on every side.
        let module = ((w.min(h) * 6 / 10) / (self.width as i32 + 8)).max(1);
        let side = module * self.width as i32;
        let x0 = self.rect.min.x + (w - side) / 2;
        let y0 = self.rect.min.y + h / 6;
        for row in 0..self.width {
            for col in 0..self.width {
                if self.dark[row * self.width + col] {
                    let x = x0 + col as i32 * module;
                    let y = y0 + row as i32 * module;
                    fb.draw_rectangle(&rect![x, y, x + module, y + module], BLACK);
                }
            }
        }
        let line = |fonts: &mut Fonts, small: bool, text: &str, y: i32, fb: &mut dyn Framebuffer| {
            let font = font_from_style(fonts, if small { &LIST_META } else { &NORMAL_STYLE }, dpi);
            let plan = font.plan(text, Some(w * 9 / 10), None);
            let x = self.rect.min.x + (w - plan.width) / 2;
            font.render(fb, BLACK, &plan, Point::new(x, y));
        };
        let below = y0 + side + 3 * module;
        line(fonts, true, &self.url, below, fb);
        line(fonts, false, "Tap to close", below + h / 12, fb);
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

    fn is_background(&self) -> bool {
        true
    }

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::QrView)
    }
}
