//! One line of the article list: title, source and date, with an unobtrusive offline marker.

use crate::color::{BLACK, GRAY09, TEXT_INVERTED_HARD, TEXT_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, MD_AUTHOR, MD_TITLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{halves, Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::unit::scale_by_dpi;
use crate::view::{Bus, Event, Hub, Id, RenderData, RenderQueue, View, ID_FEEDER};
use inoink::model::{ArticleSummary, DownloadState};

pub struct ArticleRow {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    pub item: ArticleSummary,
    index: usize,
    active: bool,
}

impl ArticleRow {
    pub fn new(rect: Rectangle, item: ArticleSummary, index: usize) -> ArticleRow {
        ArticleRow { id: ID_FEEDER.next(), rect, children: Vec::new(), item, index, active: false }
    }

    /// `YYYY-MM-DD` of a unix timestamp (UTC).
    pub fn date_text(published: i64) -> String {
        inoink::cache::format_date(published)
    }
}

impl View for ArticleRow {
    fn handle_event(&mut self, evt: &Event, hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(center)) if self.rect.includes(center) => {
                self.active = true;
                rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                hub.send(Event::OpenArticle(self.item.id)).ok();
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(center, ..)) if self.rect.includes(center) => {
                let pt = Point::new(center.x, self.rect.center().y);
                bus.push_back(Event::ToggleBookMenu(Rectangle::from_point(pt), self.index));
                true
            }
            Event::Invalid(..) => {
                if self.active {
                    self.active = false;
                    rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                }
                false
            }
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let scheme = if self.active { TEXT_INVERTED_HARD } else { TEXT_NORMAL };
        fb.draw_rectangle(&self.rect, scheme[0]);
        // Read articles are drawn lighter so unread ones stand out.
        let fg = if self.item.read && !self.active { GRAY09 } else { scheme[1] };
        let sub = if self.active { scheme[2] } else { TEXT_NORMAL[2] };

        let (x_height, padding) = {
            let font = font_from_style(fonts, &MD_TITLE, dpi);
            (font.x_heights.0 as i32, font.em() as i32)
        };
        let marker_w = 2 * padding;
        let right_w = 6 * x_height;
        let text_w = self.rect.width() as i32 - 2 * padding - marker_w - right_w;
        let start_x = self.rect.min.x + padding + marker_w;
        let top = self.rect.min.y + (self.rect.height() as i32 - 4 * x_height) / 2;

        // Star marker on the left.
        if self.item.starred {
            let c = Point::new(self.rect.min.x + padding + marker_w / 3, self.rect.center().y);
            let r = (x_height / 2).max(3);
            for (dx, dy) in [(0, r), (r, 0), (r, r), (r, -r)] {
                fb.draw_segment(c - Point::new(dx, dy), c + Point::new(dx, dy), 2.0, 2.0, if self.active { WHITE } else { BLACK });
            }
        }

        // Title (up to two lines).
        {
            let font = font_from_style(fonts, &MD_TITLE, dpi);
            let mut plan = font.plan(&self.item.title, None, None);
            let baseline = top + x_height + x_height / 2;
            if plan.width > text_w {
                let (index, usable) = font.cut_point(&plan, text_w);
                let mut second = plan.split_off(index, usable);
                font.trim_left(&mut second);
                font.crop_right(&mut second, text_w);
                font.render(fb, fg, &second, Point::new(start_x, baseline + 2 * x_height));
                font.render(fb, fg, &plan, Point::new(start_x, baseline));
            } else {
                font.render(fb, fg, &plan, Point::new(start_x, baseline + x_height));
            }
        }
        // Source line.
        {
            let font = font_from_style(fonts, &MD_AUTHOR, dpi);
            let plan = font.plan(&self.item.source, Some(text_w), None);
            font.render(fb, sub, &plan, Point::new(start_x, self.rect.max.y - x_height));
        }
        // Date, right aligned.
        {
            let font = font_from_style(fonts, &MD_AUTHOR, dpi);
            let date = Self::date_text(self.item.published);
            let plan = font.plan(&date, None, None);
            font.render(fb, sub, &plan, Point::new(self.rect.max.x - padding - plan.width, self.rect.min.y + 2 * x_height + x_height / 2));
        }
        // Offline state: a small mark, absent when nothing is cached.
        {
            let thickness = scale_by_dpi(1.5, dpi) as i32;
            let r = (x_height / 2).max(4);
            let c = Point::new(self.rect.max.x - padding - r, self.rect.max.y - x_height - r / 2);
            let ink = if self.active { WHITE } else { BLACK };
            match self.item.download {
                DownloadState::Complete => fb.draw_disk(c, r, ink),
                DownloadState::TextReady => {
                    fb.draw_disk(c, r, ink);
                    fb.draw_disk(c, (r - thickness).max(1), if self.active { BLACK } else { WHITE });
                    fb.draw_disk(c, (r / 2).max(1), ink);
                }
                DownloadState::MissingImages => {
                    fb.draw_disk(c, r, ink);
                    fb.draw_disk(c, (r - thickness).max(1), if self.active { BLACK } else { WHITE });
                }
                DownloadState::Failed | DownloadState::NotDownloaded => {}
            }
            let _ = halves(1);
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
