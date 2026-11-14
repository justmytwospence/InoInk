//! The offer to undo the last Archive or bulk change: a box floating above the footer.

use crate::color::BLACK;
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{BorderSpec, CornerSpec, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::unit::scale_by_dpi;
use crate::view::filler::Filler;
use crate::view::inoink::actions::InoInkEntry;
use crate::view::label::Label;
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::view::{BORDER_RADIUS_MEDIUM, SMALL_BAR_HEIGHT, THICKNESS_LARGE};
use std::thread;
use std::time::Duration;

pub const UNDO_TOAST_DELAY: Duration = Duration::from_secs(6);

pub struct UndoToast {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
}

impl UndoToast {
    /// `area` is the article list's rectangle; the box sits at its bottom.
    pub fn new(area: Rectangle, text: String, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) -> UndoToast {
        let id = ID_FEEDER.next();
        let hub2 = hub.clone();
        thread::spawn(move || {
            thread::sleep(UNDO_TOAST_DELAY);
            hub2.send(Event::Close(ViewId::UndoToast(id))).ok();
        });
        let dpi = CURRENT_DEVICE.dpi;
        let padding = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi).em() as i32;
        let height = scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32;
        let t = scale_by_dpi(THICKNESS_LARGE, dpi) as i32;
        let radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let rect = rect![area.min.x + padding, area.max.y - padding - height, area.max.x - padding, area.max.y - padding];
        let undo_w = 2 * height;
        let mut children: Vec<Box<dyn View>> = Vec::new();
        children.push(Box::new(Label::new(rect![rect.min.x + radius, rect.min.y + t, rect.max.x - undo_w, rect.max.y - t], text, Align::Left(padding / 2))));
        children.push(Box::new(Filler::new(rect![rect.max.x - undo_w, rect.min.y + 2 * t, rect.max.x - undo_w + t, rect.max.y - 2 * t], BLACK)));
        children.push(Box::new(
            Label::new(rect![rect.max.x - undo_w + t, rect.min.y + t, rect.max.x - radius, rect.max.y - t], "Undo".to_string(), Align::Center)
                .bold()
                .event(Some(Event::Select(EntryId::InoInk(InoInkEntry::UndoDone)))),
        ));
        rq.add(RenderData::new(id, rect, UpdateMode::Gui));
        UndoToast { id, rect, children }
    }
}

impl View for UndoToast {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, .. }) if self.rect.includes(start) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let t = scale_by_dpi(THICKNESS_LARGE, dpi) as u16;
        fb.draw_rounded_rectangle_with_border(&self.rect, &CornerSpec::Uniform(radius), &BorderSpec { thickness: t, color: BLACK }, &crate::color::WHITE);
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
    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::UndoToast(self.id))
    }
}
