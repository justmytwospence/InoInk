//! The page indicator shared by lists and articles: one segment per page while they fit, a
//! continuous line otherwise.

use crate::color::{BLACK, GRAY10, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::Fonts;
use crate::framebuffer::Framebuffer;
use crate::geom::Rectangle;
use crate::unit::{mm_to_px, scale_by_dpi};
use crate::view::{Bus, Event, Hub, Id, RenderQueue, View, ID_FEEDER};

pub const MIN_SEGMENT_MM: f32 = 6.0;
pub const SEGMENT_GAP_MM: f32 = 1.5;
pub const MAX_SEGMENTS: usize = 20;

/// Whether `total` pages can be drawn as separate segments across `width` pixels.
pub fn fits_segments(total: usize, width: i32, dpi: u16) -> bool {
    if total == 0 || total > MAX_SEGMENTS {
        return false;
    }
    total as f32 * mm_to_px(MIN_SEGMENT_MM, dpi) + (total - 1) as f32 * mm_to_px(SEGMENT_GAP_MM, dpi) <= width as f32
}

/// Draws the indicator centered on `y` from `x0` to `x1`. `current` is the 0-based page shown,
/// `fraction` (0 to 1) is how far through the content the reader is, used for the continuous line.
#[allow(clippy::too_many_arguments)]
pub fn draw(fb: &mut dyn Framebuffer, x0: i32, x1: i32, y: i32, current: usize, total: usize, fraction: f32, dpi: u16) {
    let thick = scale_by_dpi(5.0, dpi).max(2.0) as i32;
    let hair = scale_by_dpi(1.0, dpi).max(1.0) as i32;
    if fits_segments(total, x1 - x0, dpi) {
        let g = mm_to_px(SEGMENT_GAP_MM, dpi).round() as i32;
        let w = x1 - x0;
        let n = total as i32;
        let top = y - thick / 2;
        let bottom = top + thick;
        for i in 0..total {
            let sx = x0 + (i as i32 * (w + g)) / n;
            let ex = x0 + ((i as i32 + 1) * (w + g)) / n - g;
            fb.draw_rectangle(&rect![sx, top, ex, bottom], BLACK);
            if i > current {
                fb.draw_rectangle(&rect![sx + hair, top + hair, ex - hair, bottom - hair], WHITE);
            }
        }
    } else {
        fb.draw_rectangle(&rect![x0, y - hair / 2, x1, y - hair / 2 + hair], GRAY10);
        let xp = x0 + ((x1 - x0) as f32 * fraction.clamp(0.0, 1.0)).round() as i32;
        if xp > x0 {
            fb.draw_rectangle(&rect![x0, y - thick / 2, xp, y - thick / 2 + thick], BLACK);
        }
        fb.draw_disk(pt!(xp, y), thick + hair, BLACK);
    }
}

/// The indicator as a view, for the list footer.
pub struct PageProgress {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    current: usize,
    total: usize,
}

impl PageProgress {
    pub fn new(rect: Rectangle, current: usize, total: usize) -> PageProgress {
        PageProgress { id: ID_FEEDER.next(), rect, children: Vec::new(), current, total }
    }
}

impl View for PageProgress {
    fn handle_event(&mut self, _evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        false
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        let y = (self.rect.min.y + self.rect.max.y) / 2;
        draw(fb, self.rect.min.x, self.rect.max.x, y, self.current, self.total, (self.current + 1) as f32 / self.total.max(1) as f32, dpi);
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

#[cfg(test)]
mod tests {
    use super::fits_segments;

    #[test]
    fn segments_fit_by_physical_width() {
        assert!(fits_segments(14, 1264, 300));
        assert!(!fits_segments(15, 1264, 300));
        assert!(fits_segments(19, 1680, 300));
        assert!(!fits_segments(20, 1680, 300));
        assert!(!fits_segments(21, 100_000, 300));
        assert!(!fits_segments(0, 1264, 300));
        assert!(fits_segments(1, 1264, 300));
    }
}
