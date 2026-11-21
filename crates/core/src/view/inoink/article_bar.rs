//! The reader's chrome: Back, the title, Comments (link posts), Aa, Star, Done and More.

use crate::context::Context;
use crate::framebuffer::Framebuffer;
use crate::font::Fonts;
use crate::geom::{CycleDir, Rectangle, CornerSpec, BorderSpec};
use crate::gesture::GestureEvent;
use crate::input::{DeviceEvent, FingerStatus};
use crate::view::icon::{Icon, ICONS_PIXMAPS};
use crate::view::inoink::actions::{ArticleAction, DocKind, InoInkEntry};
use crate::view::label::Label;
use crate::color::{BLACK, WHITE, GRAY06, GRAY11};
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Font, RenderPlan, MD_KIND, NORMAL_STYLE};
use crate::geom::Point;
use crate::unit::scale_by_dpi;
use crate::view::{THICKNESS_SMALL, THICKNESS_MEDIUM, BORDER_RADIUS_LARGE};
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::framebuffer::UpdateMode;
use inoink::model::ArticleId;

pub struct ArticleBar {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    article: ArticleId,
}

/// The title beside the back arrow, left-aligned. Tapping it goes back too: a far bigger target
/// than the arrow alone.
fn title_label(rect: Rectangle, text: String) -> Label {
    Label::new(rect, text, Align::Left(0)).cap_centered().event(Some(Event::Back))
}

impl ArticleBar {
    pub fn new(rect: Rectangle, article: ArticleId, title: String, starred: bool, switch: Option<(DocKind, Option<u32>)>, _context: &mut Context) -> ArticleBar {
        let id = ID_FEEDER.next();
        let side = rect.height() as i32;
        let mut children: Vec<Box<dyn View>> = Vec::new();
        children.push(Box::new(Icon::new("back", rect![rect.min, rect.min + side], Event::Back)));
        let more_rect = rect![rect.max.x - side, rect.min.y, rect.max.x, rect.max.y];
        let done_rect = rect![rect.max.x - 2 * side, rect.min.y, rect.max.x - side, rect.max.y];
        let star_rect = rect![rect.max.x - 3 * side, rect.min.y, rect.max.x - 2 * side, rect.max.y];
        let aa_rect = rect![rect.max.x - 4 * side, rect.min.y, rect.max.x - 3 * side, rect.max.y];
        let title_rect = rect![rect.min.x + side, rect.min.y, aa_rect.min.x, rect.max.y];
        let mut comments_button = None;
        match switch {
            Some((current, count)) => {
                let comments_rect = rect![(aa_rect.min.x - side * 3 / 2).max(rect.min.x + side), rect.min.y, aa_rect.min.x, rect.max.y];
                comments_button = Some(CommentsButton::new(comments_rect, article, current, count));
                let title_rect = rect![rect.min.x + side, rect.min.y, comments_rect.min.x, rect.max.y];
                let text = if current == DocKind::Comments { "Comments".to_string() } else { title };
                children.push(Box::new(title_label(title_rect, text)));
            }
            None => children.push(Box::new(title_label(title_rect, title))),
        }
        children.push(Box::new(Icon::new(if starred { "bookmark-filled" } else { "bookmark" }, star_rect, Event::ArticleAction(article, ArticleAction::ToggleStar))));
        children.push(Box::new(Icon::new("archive", done_rect, Event::ArticleAction(article, ArticleAction::Done))));
        children.push(Box::new(Icon::new("more", more_rect, Event::ToggleNear(ViewId::ArticleMenu, more_rect))));
        children.push(Box::new(Label::new(aa_rect, "Aa".to_string(), Align::Center).cap_centered().event(Some(Event::Toggle(ViewId::TextPanel)))));
        if let Some(button) = comments_button {
            children.push(Box::new(button));
        }
        ArticleBar { id, rect, children, article }
    }

    /// The bar of a linked page: Back, its address and the link menu.
    pub fn for_web(rect: Rectangle, title: String) -> ArticleBar {
        let side = rect.height() as i32;
        let more_rect = rect![rect.max.x - side, rect.min.y, rect.max.x, rect.max.y];
        let title_rect = rect![rect.min.x + side, rect.min.y, more_rect.min.x, rect.max.y];
        let children: Vec<Box<dyn View>> = vec![
            Box::new(Icon::new("back", rect![rect.min, rect.min + side], Event::Back)),
            Box::new(title_label(title_rect, title)),
            Box::new(Icon::new("more", more_rect, Event::ToggleNear(ViewId::LinkMenu, more_rect))),
        ];
        ArticleBar { id: ID_FEEDER.next(), rect, children, article: ArticleId(0) }
    }

    pub fn article(&self) -> ArticleId {
        self.article
    }

    /// Updates the star icon after a change.
    pub fn set_starred(&mut self, starred: bool, rq: &mut RenderQueue) {
        if self.children.len() < 5 {
            return;
        }
        if let Some(icon) = self.children[2].downcast_mut::<Icon>() {
            let name = if starred { "bookmark-filled" } else { "bookmark" };
            if icon.name != name {
                icon.name = name.to_string();
                rq.add(RenderData::new(icon.id(), *icon.rect(), UpdateMode::Gui));
            }
        }
    }
}

impl View for ArticleBar {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) | Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, end, .. }) if self.rect.includes(start) && self.rect.includes(end) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, _fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {}

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
    use super::comments_count_label;
    use crate::view::icon::ICONS_PIXMAPS;

    #[test]
    fn count_labels() {
        for (count, expected) in [(None, None), (Some(0), Some("0")), (Some(1), Some("1")), (Some(999), Some("999")), (Some(1000), Some("999+")), (Some(u32::MAX), Some("999+"))] {
            assert_eq!(comments_count_label(count).as_deref(), expected);
        }
    }

    #[test]
    fn comment_icons_resolve() {
        assert!(ICONS_PIXMAPS.contains_key("comments") && ICONS_PIXMAPS.contains_key("article"));
    }
}

pub(crate) fn comments_count_label(count: Option<u32>) -> Option<String> {
    count.map(|n| if n <= 999 { n.to_string() } else { "999+".to_string() })
}

/// On the article, the comments icon with its count opens the comments; on the comments, an
/// article icon returns to the article.
pub struct CommentsButton {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    article: ArticleId,
    current: DocKind,
    count: Option<u32>,
}

impl CommentsButton {
    pub fn new(rect: Rectangle, article: ArticleId, current: DocKind, count: Option<u32>) -> CommentsButton {
        CommentsButton { id: ID_FEEDER.next(), rect, children: Vec::new(), article, current, count }
    }
}

impl View for CommentsButton {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                let other = if self.current == DocKind::Comments { DocKind::Article } else { DocKind::Comments };
                bus.push_back(Event::ShowDocument(self.article, other));
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        // On the comments the button returns to the article: an article icon, no count.
        let on_comments = self.current == DocKind::Comments;
        let Some(pixmap) = ICONS_PIXMAPS.get(if on_comments { "article" } else { "comments" }) else { return };
        let gap = scale_by_dpi(8.0, dpi) as i32;
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        let icon_w = pixmap.width as i32;
        let label = if on_comments { None } else { comments_count_label(self.count) };
        let plan = label.as_ref().map(|l| font.plan(l, Some((self.rect.width() as i32 - icon_w - gap).max(0)), None));
        let w = icon_w + plan.as_ref().map_or(0, |p| gap + p.width);
        let x0 = (self.rect.min.x + (self.rect.width() as i32 - w) / 2).max(self.rect.min.x);
        let y = self.rect.min.y + (self.rect.height() as i32 - pixmap.height as i32) / 2;
        fb.draw_blended_pixmap(pixmap, Point::new(x0, y), BLACK);
        if let Some(plan) = plan {
            let baseline = self.rect.min.y + (self.rect.height() as i32 + font.x_heights.1 as i32) / 2;
            font.render(fb, BLACK, &plan, Point::new(x0 + icon_w + gap, baseline));
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

/// One line under the article while the bar is showing: progress, time left, place in the list.
pub struct ArticleFooter {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    text: String,
}

impl ArticleFooter {
    pub fn new(rect: Rectangle, text: String) -> ArticleFooter {
        ArticleFooter { id: ID_FEEDER.next(), rect, children: Vec::new(), text }
    }
}

impl View for ArticleFooter {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) | Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        let thickness = scale_by_dpi(THICKNESS_SMALL, dpi) as i32;
        fb.draw_rectangle(&rect![self.rect.min.x, self.rect.min.y, self.rect.max.x, self.rect.min.y + thickness], BLACK);
        let font = font_from_style(fonts, &crate::font::LIST_META, dpi);
        let plan = font.plan(&self.text, Some(self.rect.width() as i32 * 95 / 100), None);
        let x = self.rect.min.x + (self.rect.width() as i32 - plan.width) / 2;
        let y = self.rect.min.y + (self.rect.height() as i32 + font.x_heights.0 as i32) / 2;
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

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::ArticleFooter)
    }
}

/// Height of the titles row above the footer.
pub const QUEUE_NAV_HEIGHT: f32 = 200.0;
// Space around and between the cards, the padding inside them, and the label's tracking.
const CARD_MARGIN: f32 = 16.0;
const CARD_PADDING: f32 = 24.0;
const LABEL_TRACKING: f32 = 2.0;

/// The title on one line, or two with the second ending at a whole word with an ellipsis.
fn title_lines(font: &mut Font, title: &str, max_width: i32) -> (RenderPlan, Option<RenderPlan>) {
    if max_width <= 0 {
        return (font.plan(title, Some(1), None), None);
    }
    let mut plan = font.plan(title, None, None);
    if plan.width > max_width {
        let (index, usable) = font.cut_point(&plan, max_width);
        let mut second = plan.split_off(index, usable);
        font.trim_left(&mut second);
        font.crop_right(&mut second, max_width);
        (plan, Some(second))
    } else {
        (plan, None)
    }
}

/// Above the footer while the bar is showing: the previous and next articles of the list as
/// cards (one across the row when alone). Tapping a card opens that article, as swiping does;
/// pressing inverts it.
pub struct QueueNav {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    previous: Option<String>,
    next: Option<String>,
    active: Option<CycleDir>,
}

impl QueueNav {
    pub fn new(rect: Rectangle, previous: Option<String>, next: Option<String>) -> QueueNav {
        QueueNav { id: ID_FEEDER.next(), rect, children: Vec::new(), previous, next, active: None }
    }

    /// The previous and next cards: side by side, or one across the row when it is alone.
    fn cards(&self) -> (Option<Rectangle>, Option<Rectangle>) {
        let dpi = CURRENT_DEVICE.dpi;
        let m = scale_by_dpi(CARD_MARGIN, dpi) as i32;
        let line = scale_by_dpi(THICKNESS_SMALL, dpi) as i32;
        let inner = rect![self.rect.min.x + m, self.rect.min.y + line + m, self.rect.max.x - m, self.rect.max.y - m];
        match (self.previous.is_some(), self.next.is_some()) {
            (true, true) => {
                let mid = (inner.min.x + inner.max.x) / 2;
                (Some(rect![inner.min.x, inner.min.y, mid - m / 2, inner.max.y]),
                 Some(rect![mid + (m - m / 2), inner.min.y, inner.max.x, inner.max.y]))
            }
            (true, false) => (Some(inner), None),
            (false, true) => (None, Some(inner)),
            (false, false) => (None, None),
        }
    }
}

impl View for QueueNav {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                let (previous, next) = self.cards();
                if previous.is_some_and(|r| r.includes(c)) {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::QueueArticle(CycleDir::Previous))));
                } else if next.is_some_and(|r| r.includes(c)) {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::QueueArticle(CycleDir::Next))));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, end, .. }) if self.rect.includes(start) && self.rect.includes(end) => true,
            Event::Device(DeviceEvent::Finger { status, position, .. }) if self.rect.includes(position) || self.active.is_some() => {
                match status {
                    FingerStatus::Down => {
                        let (previous, next) = self.cards();
                        self.active = if previous.is_some_and(|r| r.includes(position)) {
                            Some(CycleDir::Previous)
                        } else if next.is_some_and(|r| r.includes(position)) {
                            Some(CycleDir::Next)
                        } else {
                            None
                        };
                        if self.active.is_some() {
                            rq.add(RenderData::new(self.id, self.rect, UpdateMode::Fast));
                        }
                    }
                    FingerStatus::Up if self.active.is_some() => {
                        self.active = None;
                        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                    }
                    _ => (),
                }
                self.rect.includes(position)
            }
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        let thickness = scale_by_dpi(THICKNESS_SMALL, dpi) as i32;
        fb.draw_rectangle(&rect![self.rect.min.x, self.rect.min.y, self.rect.max.x, self.rect.min.y + thickness], BLACK);
        let (previous, next) = self.cards();
        let radius = scale_by_dpi(BORDER_RADIUS_LARGE, dpi) as i32;
        let border = BorderSpec { thickness: scale_by_dpi(THICKNESS_MEDIUM, dpi) as u16, color: BLACK };
        let pad = scale_by_dpi(CARD_PADDING, dpi) as i32;
        let cards = [(previous, self.previous.as_ref(), CycleDir::Previous), (next, self.next.as_ref(), CycleDir::Next)];
        for (card, title, dir) in cards {
            let (Some(card), Some(title)) = (card, title) else { continue };
            let left = dir == CycleDir::Previous;
            let active = self.active == Some(dir);
            let (paper, ink, label_color) = if active { (BLACK, WHITE, GRAY11) } else { (WHITE, BLACK, GRAY06) };
            fb.draw_rounded_rectangle_with_border(&card, &CornerSpec::Uniform(radius), &border, &paper);

            let mut icon_w = 0;
            if let Some(pixmap) = ICONS_PIXMAPS.get(if left { "angle-left-small" } else { "angle-right-small" }) {
                icon_w = pixmap.width as i32;
                let x = if left { card.min.x + pad } else { card.max.x - pad - icon_w };
                let y = card.min.y + (card.height() as i32 - pixmap.height as i32) / 2;
                fb.draw_blended_pixmap(pixmap, Point::new(x, y), ink);
            }
            let (col_min, col_max) = if left {
                (card.min.x + pad + icon_w + pad / 2, card.max.x - pad)
            } else {
                (card.min.x + pad, card.max.x - pad - icon_w - pad / 2)
            };
            let col_w = col_max - col_min;

            let (llh, lasc, label) = {
                let font = font_from_style(fonts, &MD_KIND, dpi);
                let mut plan = font.plan(if left { "PREVIOUS" } else { "NEXT" }, Some(col_w.max(1)), None);
                plan.space_out(scale_by_dpi(LABEL_TRACKING, dpi) as i32);
                (font.line_height(), font.ascender(), plan)
            };
            let (tlh, tasc, first, second) = {
                let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
                let (first, second) = title_lines(font, title, col_w);
                (font.line_height(), font.ascender(), first, second)
            };
            let gap = llh / 4;
            let lines = 1 + second.is_some() as i32;
            let block = llh + gap + lines * tlh;
            let top = card.min.y + (card.height() as i32 - block) / 2;
            let x_for = |width: i32| if left { col_min } else { col_max - width };

            {
                let font = font_from_style(fonts, &MD_KIND, dpi);
                font.render(fb, label_color, &label, Point::new(x_for(label.width), top + lasc));
            }
            let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
            let y0 = top + llh + gap + tasc;
            font.render(fb, ink, &first, Point::new(x_for(first.width), y0));
            if let Some(second) = &second {
                font.render(fb, ink, second, Point::new(x_for(second.width), y0 + tlh));
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

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::QueueNav)
    }
}
