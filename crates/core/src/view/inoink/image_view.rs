//! Full-screen enlargement of one article image, from the local cache only.

use crate::color::WHITE;
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::document::{open, Location};
use crate::font::Fonts;
use crate::framebuffer::{Framebuffer, Pixmap, UpdateMode};
use crate::geom::{Dir, Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::view::icon::Icon;
use crate::view::{Bus, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use std::path::PathBuf;

const MAX_ZOOM: u32 = 3;

pub struct ImageView {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    path: PathBuf,
    fit_scale: f32,
    zoom: u32,
    pixmap: Option<Pixmap>,
    /// Top-left of the visible window inside the pixmap when zoomed.
    offset: Point,
}

impl ImageView {
    pub fn new(rect: Rectangle, path: PathBuf, context: &mut Context) -> ImageView {
        let _ = context;
        let side = (rect.width().min(rect.height()) / 12) as i32;
        let back = Icon::new("back", rect![rect.min, rect.min + side], Event::Back);
        let mut view = ImageView { id: ID_FEEDER.next(), rect, children: vec![Box::new(back)], path, fit_scale: 1.0, zoom: 1, pixmap: None, offset: Point::default() };
        view.load();
        view
    }

    fn load(&mut self) {
        let Some(mut doc) = open(&self.path) else {
            self.pixmap = None;
            return;
        };
        let Some((w, h)) = doc.dims(0) else {
            self.pixmap = None;
            return;
        };
        self.fit_scale = (self.rect.width() as f32 / w).min(self.rect.height() as f32 / h);
        let scale = self.fit_scale * self.zoom as f32;
        self.pixmap = doc.pixmap(Location::Exact(0), scale, CURRENT_DEVICE.color_samples()).map(|(p, _)| p);
    }

    fn clamp(&mut self) {
        if let Some(p) = &self.pixmap {
            let max_x = (p.width as i32 - self.rect.width() as i32).max(0);
            let max_y = (p.height as i32 - self.rect.height() as i32).max(0);
            self.offset.x = self.offset.x.clamp(0, max_x);
            self.offset.y = self.offset.y.clamp(0, max_y);
        }
    }

    fn refresh(&mut self, rq: &mut RenderQueue) {
        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Full));
    }
}

impl View for ImageView {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                // Tap cycles fit, 2x, 3x and back.
                self.zoom = if self.zoom >= MAX_ZOOM { 1 } else { self.zoom + 1 };
                self.offset = Point::default();
                self.load();
                self.refresh(rq);
                true
            }
            Event::Gesture(GestureEvent::Swipe { dir, start, .. }) if self.rect.includes(start) && self.zoom > 1 => {
                let step_x = self.rect.width() as i32 * 2 / 3;
                let step_y = self.rect.height() as i32 * 2 / 3;
                match dir {
                    Dir::West => self.offset.x += step_x,
                    Dir::East => self.offset.x -= step_x,
                    Dir::North => self.offset.y += step_y,
                    Dir::South => self.offset.y -= step_y,
                }
                self.clamp();
                self.refresh(rq);
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {
        fb.draw_rectangle(&self.rect, WHITE);
        if let Some(p) = &self.pixmap {
            let w = (p.width as i32).min(self.rect.width() as i32);
            let h = (p.height as i32).min(self.rect.height() as i32);
            let frame = rect![self.offset.x, self.offset.y, self.offset.x + w, self.offset.y + h];
            let at = Point::new(self.rect.min.x + (self.rect.width() as i32 - w) / 2, self.rect.min.y + (self.rect.height() as i32 - h) / 2);
            fb.draw_framed_pixmap(p, &frame, at);
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

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::ImageViewer)
    }
}
