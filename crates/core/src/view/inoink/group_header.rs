//! A half-height header line in the article list ("Unread", a source name).

use crate::color::{BLACK, GRAY07, GRAY13};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, LIST_GROUP, LIST_TITLE};
use crate::framebuffer::Framebuffer;
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::unit::scale_by_dpi;
use crate::view::THICKNESS_SMALL;
use crate::view::{Bus, Event, Hub, Id, RenderQueue, View, ID_FEEDER};

pub struct GroupHeader {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    text: String,
}

impl GroupHeader {
    pub fn new(rect: Rectangle, text: String) -> GroupHeader {
        GroupHeader { id: ID_FEEDER.next(), rect, children: Vec::new(), text }
    }
}

impl View for GroupHeader {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        matches!(*evt, Event::Gesture(GestureEvent::Tap(c)) | Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c))
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, GRAY13);
        let thickness = scale_by_dpi(THICKNESS_SMALL, dpi) as i32;
        fb.draw_rectangle(&rect![self.rect.min.x, self.rect.max.y - thickness, self.rect.max.x, self.rect.max.y], GRAY07);
        let em = font_from_style(fonts, &LIST_TITLE, dpi).em() as i32;
        let x = self.rect.min.x + em;
        let max_w = self.rect.max.x - em - x;
        let font = font_from_style(fonts, &LIST_GROUP, dpi);
        let plan = font.plan(&self.text, Some(max_w), None);
        let y = self.rect.min.y + (self.rect.height() as i32 - thickness + font.x_heights.1 as i32) / 2;
        font.render(fb, BLACK, &plan, Point::new(x, y));
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
