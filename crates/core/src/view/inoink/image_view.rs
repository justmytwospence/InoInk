//! Full-screen enlargement of one article image, from the local cache only.
//!
//! Pinch or spread to zoom, or use the buttons at the bottom; drag to pan; double-tap to switch
//! between fitting the screen and an enlargement around the tap. Each image's zoom and position
//! are remembered for as long as InoInk runs.

use crate::color::WHITE;
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::document::{open, Location};
use crate::font::Fonts;
use crate::framebuffer::{Framebuffer, Pixmap, UpdateMode};
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::view::button::Button;
use crate::view::icon::Icon;
use crate::view::{Bus, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The largest zoom, relative to fitting the screen.
pub const MAX_ZOOM: f32 = 4.0;
/// The largest rendered image, in pixels: zooming stops before the pixmap outgrows the memory.
pub const MAX_PIXELS: f32 = 24_000_000.0;
/// Each zoom button press multiplies or divides the zoom by this.
pub const ZOOM_STEP: f32 = 1.5;
/// Double-tapping an image that fits enlarges it this much.
pub const DOUBLE_TAP_ZOOM: f32 = 2.0;
/// Two taps closer than this in time and space are a double tap.
pub const DOUBLE_TAP_DELAY: Duration = Duration::from_millis(450);

/// The zoom buttons.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ZoomStep {
    In,
    Out,
    Fit,
}

/// How an image was last shown: its zoom (1 fits the screen) and the point of the image at the
/// centre of the screen, as fractions of its width and height.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ImageZoom {
    pub zoom: f32,
    pub center: (f32, f32),
}

/// The zoom an image of `dims` can reach on `rect`, given its fitting scale: at most
/// [`MAX_ZOOM`], and less when the enlarged image would exceed [`MAX_PIXELS`].
pub fn zoom_limit(dims: (f32, f32), fit_scale: f32) -> f32 {
    let fitted = dims.0 * dims.1 * fit_scale * fit_scale;
    if fitted <= 0.0 {
        return 1.0;
    }
    (MAX_PIXELS / fitted).sqrt().clamp(1.0, MAX_ZOOM)
}

pub struct ImageView {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    path: PathBuf,
    fit_scale: f32,
    max_zoom: f32,
    zoom: f32,
    pixmap: Option<Pixmap>,
    /// Top-left of the visible window inside the pixmap when it is larger than the screen.
    offset: Point,
    last_tap: Option<(Instant, Point)>,
}

impl ImageView {
    pub fn new(rect: Rectangle, path: PathBuf, context: &mut Context) -> ImageView {
        let side = (rect.width().min(rect.height()) / 12) as i32;
        let back = Icon::new("back", rect![rect.min, rect.min + side], Event::Back);
        let mut children: Vec<Box<dyn View>> = vec![Box::new(back)];
        // Three buttons centred at the bottom: zoom out, fit, zoom in.
        let gap = side / 4;
        let width = (rect.width() as i32 - 4 * gap) / 3;
        let width = width.min(4 * side);
        let total = 3 * width + 2 * gap;
        let x0 = rect.min.x + (rect.width() as i32 - total) / 2;
        let y1 = rect.max.y - gap;
        for (i, (label, step)) in [("Zoom out", ZoomStep::Out), ("Fit", ZoomStep::Fit), ("Zoom in", ZoomStep::In)].into_iter().enumerate() {
            let x = x0 + i as i32 * (width + gap);
            children.push(Box::new(Button::new(rect![x, y1 - side, x + width, y1], Event::ImageZoom(step), label.to_string())));
        }
        let mut view = ImageView {
            id: ID_FEEDER.next(),
            rect,
            children,
            path,
            fit_scale: 1.0,
            max_zoom: 1.0,
            zoom: 1.0,
            pixmap: None,
            offset: Point::default(),
            last_tap: None,
        };
        let remembered = context.inoink.image_zoom.get(&view.path).copied();
        view.zoom = remembered.map_or(1.0, |z| z.zoom);
        view.load();
        if let Some(z) = remembered {
            view.place(z.center, view.rect.center());
        }
        view.sync_buttons();
        view
    }

    /// The zoom (1 fits the screen).
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// The largest zoom this image allows.
    pub fn max_zoom(&self) -> f32 {
        self.max_zoom
    }

    /// The top-left of the visible part of the image, in rendered pixels.
    pub fn offset(&self) -> Point {
        self.offset
    }

    /// The rendered image's size, when it could be read.
    pub fn rendered_size(&self) -> Option<(u32, u32)> {
        self.pixmap.as_ref().map(|p| (p.width, p.height))
    }

    fn load(&mut self) {
        self.pixmap = None;
        let Some(mut doc) = open(&self.path) else { return };
        let Some((w, h)) = doc.dims(0) else { return };
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.fit_scale = (self.rect.width() as f32 / w).min(self.rect.height() as f32 / h);
        self.max_zoom = zoom_limit((w, h), self.fit_scale);
        self.zoom = self.zoom.clamp(1.0, self.max_zoom);
        let scale = self.fit_scale * self.zoom;
        self.pixmap = doc.pixmap(Location::Exact(0), scale, CURRENT_DEVICE.color_samples()).map(|(p, _)| p);
    }

    /// Where the pixmap's top-left corner is drawn: centred while it is smaller than the screen.
    fn origin(&self) -> Point {
        let Some(p) = &self.pixmap else { return self.rect.min };
        Point::new(
            self.rect.min.x + ((self.rect.width() as i32 - p.width as i32) / 2).max(0),
            self.rect.min.y + ((self.rect.height() as i32 - p.height as i32) / 2).max(0),
        )
    }

    /// The point of the image under `pt`, as fractions of its width and height.
    fn image_point(&self, pt: Point) -> (f32, f32) {
        let Some(p) = &self.pixmap else { return (0.5, 0.5) };
        let o = self.origin();
        let x = (pt.x - o.x + self.offset.x) as f32 / p.width.max(1) as f32;
        let y = (pt.y - o.y + self.offset.y) as f32 / p.height.max(1) as f32;
        (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))
    }

    /// Scrolls so that the image point `frac` lies under the screen point `pt`, as far as the
    /// edges allow.
    fn place(&mut self, frac: (f32, f32), pt: Point) {
        let Some(p) = &self.pixmap else { return };
        let o = self.origin();
        self.offset = Point::new((frac.0 * p.width as f32) as i32 - (pt.x - o.x), (frac.1 * p.height as f32) as i32 - (pt.y - o.y));
        self.clamp();
    }

    fn clamp(&mut self) {
        if let Some(p) = &self.pixmap {
            let max_x = (p.width as i32 - self.rect.width() as i32).max(0);
            let max_y = (p.height as i32 - self.rect.height() as i32).max(0);
            self.offset.x = self.offset.x.clamp(0, max_x);
            self.offset.y = self.offset.y.clamp(0, max_y);
        } else {
            self.offset = Point::default();
        }
    }

    /// Zooms to `zoom` keeping the image point under `anchor` where it is.
    fn set_zoom(&mut self, zoom: f32, anchor: Point, rq: &mut RenderQueue, context: &mut Context) {
        let zoom = zoom.clamp(1.0, self.max_zoom.max(1.0));
        if !zoom.is_finite() || (zoom - self.zoom).abs() < 0.01 {
            return;
        }
        let frac = self.image_point(anchor);
        self.zoom = zoom;
        self.load();
        self.place(frac, anchor);
        self.sync_buttons();
        self.remember(context);
        self.refresh(rq);
    }

    fn pan(&mut self, delta: Point, rq: &mut RenderQueue, context: &mut Context) {
        let before = self.offset;
        self.offset = self.offset - delta;
        self.clamp();
        if self.offset != before {
            self.remember(context);
            self.refresh(rq);
        }
    }

    fn remember(&self, context: &mut Context) {
        if self.pixmap.is_none() {
            return;
        }
        let z = ImageZoom { zoom: self.zoom, center: self.image_point(self.rect.center()) };
        context.inoink.image_zoom.insert(self.path.clone(), z);
    }

    fn sync_buttons(&mut self) {
        let (zoom, max) = (self.zoom, self.max_zoom);
        for (i, disabled) in [zoom <= 1.0, zoom <= 1.0, zoom >= max - 0.01].into_iter().enumerate() {
            if let Some(b) = self.children.get_mut(i + 1).and_then(|c| c.downcast_mut::<Button>()) {
                b.disabled = disabled || self.pixmap.is_none();
            }
        }
    }

    fn refresh(&mut self, rq: &mut RenderQueue) {
        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Full));
    }
}

impl View for ImageView {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, rq: &mut RenderQueue, context: &mut Context) -> bool {
        match *evt {
            Event::ImageZoom(step) => {
                let center = self.rect.center();
                match step {
                    ZoomStep::In => self.set_zoom(self.zoom * ZOOM_STEP, center, rq, context),
                    ZoomStep::Out => self.set_zoom(self.zoom / ZOOM_STEP, center, rq, context),
                    ZoomStep::Fit => self.set_zoom(1.0, center, rq, context),
                }
                true
            }
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                let now = Instant::now();
                let near = (self.rect.width().min(self.rect.height()) / 10) as i32;
                let double = self.last_tap.is_some_and(|(t, p)| now.duration_since(t) <= DOUBLE_TAP_DELAY && (p.x - c.x).abs() <= near && (p.y - c.y).abs() <= near);
                if double {
                    self.last_tap = None;
                    let target = if self.zoom > 1.0 { 1.0 } else { DOUBLE_TAP_ZOOM };
                    self.set_zoom(target, c, rq, context);
                } else {
                    self.last_tap = Some((now, c));
                }
                true
            }
            Event::Gesture(GestureEvent::Spread { center, factor, .. }) | Event::Gesture(GestureEvent::Pinch { center, factor, .. }) if self.rect.includes(center) => {
                if factor.is_finite() && factor > 0.0 {
                    self.set_zoom(self.zoom * factor, center, rq, context);
                }
                true
            }
            Event::Gesture(GestureEvent::Swipe { start, end, .. }) | Event::Gesture(GestureEvent::SlantedSwipe { start, end, .. }) if self.rect.includes(start) => {
                self.pan(end - start, rq, context);
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
            fb.draw_framed_pixmap(p, &frame, self.origin());
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
