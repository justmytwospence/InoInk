//! One line of the article list: title, source and date, with an unobtrusive offline marker.

use crate::chrono::{DateTime, Datelike, Local, TimeZone};
use crate::color::{BLACK, GRAY03, TEXT_INVERTED_HARD, TEXT_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, LIST_META, LIST_TITLE, LIST_TITLE_READ};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{Point, Rectangle};
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

}

/// "Just now", "10 min ago", "3 h ago", "Yesterday", "Tue", "Oct 1", "Mar 4, 2025".
pub fn relative_date<Tz: TimeZone>(published: i64, now: &DateTime<Tz>) -> String {
    if published <= 0 {
        return String::new();
    }
    let Some(then) = now.timezone().timestamp_opt(published, 0).single() else { return String::new() };
    let diff = now.timestamp() - published;
    if diff < 60 {
        return "Just now".to_string();
    }
    if diff < 3600 {
        return format!("{} min ago", diff / 60);
    }
    if diff < 86400 {
        return format!("{} h ago", diff / 3600);
    }
    if now.date_naive().pred_opt() == Some(then.date_naive()) {
        return "Yesterday".to_string();
    }
    if diff < 7 * 86400 {
        return ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][then.weekday().num_days_from_monday() as usize].to_string();
    }
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][then.month0() as usize];
    if then.year() == now.year() {
        format!("{} {}", month, then.day())
    } else {
        format!("{} {}, {}", month, then.day(), then.year())
    }
}

#[cfg(test)]
mod tests {
    use super::relative_date;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn dates_read_naturally() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = tz.with_ymd_and_hms(2026, 11, 11, 12, 0, 0).unwrap();
        let at = |y, m, d, h| tz.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp();
        assert_eq!(relative_date(now.timestamp() - 30, &now), "Just now");
        assert_eq!(relative_date(now.timestamp() - 600, &now), "10 min ago");
        assert_eq!(relative_date(now.timestamp() - 3 * 3600, &now), "3 h ago");
        assert_eq!(relative_date(at(2026, 11, 10, 8), &now), "Yesterday");
        assert_eq!(relative_date(at(2026, 11, 7, 12), &now), "Sat");
        assert_eq!(relative_date(at(2026, 10, 1, 12), &now), "Oct 1");
        assert_eq!(relative_date(at(2025, 3, 4, 12), &now), "Mar 4, 2025");
        assert_eq!(relative_date(0, &now), "");
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
        let title_style = if self.item.read { &*LIST_TITLE_READ } else { &*LIST_TITLE };
        let fg = scheme[1];
        let sub = if self.active { scheme[2] } else { GRAY03 };

        let (padding, tlh, asc) = {
            let font = font_from_style(fonts, title_style, dpi);
            (font.em() as i32, font.line_height(), font.ascender())
        };
        let (mlh, masc, mxh) = {
            let font = font_from_style(fonts, &LIST_META, dpi);
            (font.line_height(), font.ascender(), font.x_heights.0 as i32)
        };
        let marker_w = 2 * padding;
        let text_w = self.rect.width() as i32 - 2 * padding - marker_w;
        let start_x = self.rect.min.x + padding + marker_w;

        // Title: one or two lines.
        let (plan, second) = {
            let font = font_from_style(fonts, title_style, dpi);
            let mut plan = font.plan(&self.item.title, None, None);
            if plan.width > text_w {
                let (index, usable) = font.cut_point(&plan, text_w);
                let mut second = plan.split_off(index, usable);
                font.trim_left(&mut second);
                font.crop_right(&mut second, text_w);
                (plan, Some(second))
            } else {
                (plan, None)
            }
        };
        let n = if second.is_some() { 2 } else { 1 };
        let gap = mlh / 4;
        let block = n * tlh + gap + mlh;
        let top = self.rect.min.y + (self.rect.height() as i32 - block) / 2;
        {
            let font = font_from_style(fonts, title_style, dpi);
            font.render(fb, fg, &plan, Point::new(start_x, top + asc));
            if let Some(second) = &second {
                font.render(fb, fg, second, Point::new(start_x, top + tlh + asc));
            }
        }

        // Star marker, centred on the first title line.
        if self.item.starred {
            let c = Point::new(self.rect.min.x + padding + marker_w / 3, top + tlh / 2);
            let r = (mxh / 2).max(3);
            for (dx, dy) in [(0, r), (r, 0), (r, r), (r, -r)] {
                fb.draw_segment(c - Point::new(dx, dy), c + Point::new(dx, dy), 2.0, 2.0, if self.active { WHITE } else { BLACK });
            }
        }

        // Meta line: source, relative date, reading time, offline mark.
        let mb = top + n * tlh + gap + masc;
        let r = (mxh / 2).max(4);
        let has_mark = !matches!(self.item.download, DownloadState::Failed | DownloadState::NotDownloaded);
        let ow = if has_mark { 2 * r + padding / 2 } else { 0 };
        let mut rest = String::new();
        for part in [relative_date(self.item.published, &Local::now()), inoink::content::format_read_time(self.item.words).unwrap_or_default()] {
            if !part.is_empty() {
                rest.push_str(" \u{b7} ");
                rest.push_str(&part);
            }
        }
        {
            let font = font_from_style(fonts, &LIST_META, dpi);
            let rest_plan = font.plan(&rest, None, None);
            let src_w = (text_w - ow - rest_plan.width).max(text_w / 3);
            let src_plan = font.plan(&self.item.source, Some(src_w), None);
            let sw = src_plan.width;
            font.render(fb, sub, &src_plan, Point::new(start_x, mb));
            font.render(fb, sub, &rest_plan, Point::new(start_x + sw, mb));
        }
        if has_mark {
            let thickness = scale_by_dpi(1.5, dpi) as i32;
            let c = Point::new(self.rect.max.x - padding - r, mb - mxh / 2);
            let ink = if self.active { WHITE } else { BLACK };
            let paper = if self.active { BLACK } else { WHITE };
            match self.item.download {
                DownloadState::Complete => fb.draw_disk(c, r, ink),
                DownloadState::TextReady => {
                    fb.draw_disk(c, r, ink);
                    fb.draw_disk(c, (r - thickness).max(1), paper);
                    fb.draw_disk(c, (r / 2).max(1), ink);
                }
                DownloadState::MissingImages => {
                    fb.draw_disk(c, r, ink);
                    fb.draw_disk(c, (r - thickness).max(1), paper);
                }
                DownloadState::Failed | DownloadState::NotDownloaded => {}
            }
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
