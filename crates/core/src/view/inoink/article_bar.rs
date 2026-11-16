//! The reader's chrome: Back, the title, Comments (link posts), Aa, Star, Done and More.

use crate::context::Context;
use crate::framebuffer::Framebuffer;
use crate::font::Fonts;
use crate::geom::{CycleDir, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::view::icon::{Icon, ICONS_PIXMAPS};
use crate::view::inoink::actions::{ArticleAction, DocKind, InoInkEntry};
use crate::view::label::Label;
use crate::color::{BLACK, WHITE};
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, NORMAL_STYLE};
use crate::geom::Point;
use crate::unit::scale_by_dpi;
use crate::view::THICKNESS_SMALL;
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::framebuffer::UpdateMode;
use inoink::model::ArticleId;

pub struct ArticleBar {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    article: ArticleId,
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
                children.push(Box::new(Label::new(title_rect, text, Align::Center).cap_centered()));
            }
            None => children.push(Box::new(Label::new(title_rect, title, Align::Center).cap_centered())),
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
            Box::new(Label::new(title_rect, title, Align::Center).cap_centered()),
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

/// Above the footer while the bar is showing: the previous and next articles of the list.
/// Tapping a side opens that article, as swiping does.
pub struct QueueNav {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    previous: Option<String>,
    next: Option<String>,
}

impl QueueNav {
    pub fn new(rect: Rectangle, previous: Option<String>, next: Option<String>) -> QueueNav {
        QueueNav { id: ID_FEEDER.next(), rect, children: Vec::new(), previous, next }
    }

    fn halves(&self) -> (Rectangle, Rectangle) {
        let mid = (self.rect.min.x + self.rect.max.x) / 2;
        (rect![self.rect.min.x, self.rect.min.y, mid, self.rect.max.y], rect![mid, self.rect.min.y, self.rect.max.x, self.rect.max.y])
    }
}

impl View for QueueNav {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                let (left, right) = self.halves();
                if left.includes(c) && self.previous.is_some() {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::QueueArticle(CycleDir::Previous))));
                } else if right.includes(c) && self.next.is_some() {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::QueueArticle(CycleDir::Next))));
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
        fb.draw_rectangle(&self.rect, WHITE);
        let thickness = scale_by_dpi(THICKNESS_SMALL, dpi) as i32;
        fb.draw_rectangle(&rect![self.rect.min.x, self.rect.min.y, self.rect.max.x, self.rect.min.y + thickness], BLACK);
        let (left, right) = self.halves();
        let inset = self.rect.height() as i32 / 4;
        fb.draw_rectangle(&rect![left.max.x, self.rect.min.y + inset, left.max.x + thickness, self.rect.max.y - inset], BLACK);
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        let pad = scale_by_dpi(24.0, dpi) as i32;
        let x_height = font.x_heights.0 as i32;
        if let Some(title) = self.previous.as_ref() {
            let mut x = left.min.x + pad;
            let mut icon_w = 0;
            if let Some(pixmap) = ICONS_PIXMAPS.get("angle-left-small") {
                icon_w = pixmap.width as i32;
                let y = left.min.y + (left.height() as i32 - pixmap.height as i32) / 2;
                fb.draw_blended_pixmap(pixmap, Point::new(x, y), BLACK);
                x += icon_w + pad;
            }
            let plan = font.plan(title, Some((left.width() as i32 - 3 * pad - icon_w).max(0)), None);
            font.render(fb, BLACK, &plan, Point::new(x, left.min.y + (left.height() as i32 + x_height) / 2));
        }
        if let Some(title) = self.next.as_ref() {
            let mut x_end = right.max.x - pad;
            let mut icon_w = 0;
            if let Some(pixmap) = ICONS_PIXMAPS.get("angle-right-small") {
                icon_w = pixmap.width as i32;
                let y = right.min.y + (right.height() as i32 - pixmap.height as i32) / 2;
                fb.draw_blended_pixmap(pixmap, Point::new(x_end - icon_w, y), BLACK);
                x_end -= icon_w + pad;
            }
            let plan = font.plan(title, Some((right.width() as i32 - 3 * pad - icon_w).max(0)), None);
            font.render(fb, BLACK, &plan, Point::new(x_end - plan.width, right.min.y + (right.height() as i32 + x_height) / 2));
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
