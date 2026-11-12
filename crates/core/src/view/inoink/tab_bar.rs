//! The three tabs under the top bar: Newsfeed, Categories and Read later.

use crate::color::{BLACK, SEPARATOR_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::view::inoink::actions::InoInkEntry;
use crate::view::inoink::state::Tab;
use crate::view::{Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ID_FEEDER};

pub const TABS: [Tab; 3] = [Tab::Newsfeed, Tab::Categories, Tab::ReadLater];

pub struct TabBar {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    current: Tab,
    labels: [String; 3],
}

impl TabBar {
    pub fn new(rect: Rectangle, current: Tab, labels: [String; 3]) -> TabBar {
        TabBar { id: ID_FEEDER.next(), rect, children: Vec::new(), current, labels }
    }

    /// Redraws only when the selected tab or a label changed.
    pub fn update(&mut self, current: Tab, labels: [String; 3], rq: &mut RenderQueue) {
        if self.current != current || self.labels != labels {
            self.current = current;
            self.labels = labels;
            rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
        }
    }

    fn cell(&self, i: usize) -> Rectangle {
        let w = self.rect.width() as i32;
        rect![self.rect.min.x + w * i as i32 / 3, self.rect.min.y, self.rect.min.x + w * (i as i32 + 1) / 3, self.rect.max.y]
    }
}

impl View for TabBar {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                if let Some(i) = (0..3).find(|i| self.cell(*i).includes(c)) {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::Tab(TABS[i]))));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, end, .. }) if self.rect.includes(start) && self.rect.includes(end) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        for i in 0..3 {
            let cell = self.cell(i);
            let active = TABS[i] == self.current;
            fb.draw_rectangle(&cell, if active { BLACK } else { WHITE });
            if i > 0 {
                fb.draw_rectangle(&rect![cell.min.x, cell.min.y, cell.min.x + 1, cell.max.y], SEPARATOR_NORMAL);
            }
            let pad = font.em() as i32 / 2;
            let plan = font.plan(&self.labels[i], Some(cell.width() as i32 - 2 * pad), None);
            let x = cell.min.x + (cell.width() as i32 - plan.width) / 2;
            let y = cell.min.y + (cell.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, if active { WHITE } else { BLACK }, &plan, Point::new(x, y));
        }
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
