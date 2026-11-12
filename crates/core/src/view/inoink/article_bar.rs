//! The reader's only chrome: Back, the title, Star, Done and More.

use crate::context::Context;
use crate::framebuffer::Framebuffer;
use crate::font::Fonts;
use crate::geom::Rectangle;
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::view::icon::Icon;
use crate::view::inoink::actions::{ArticleAction, DocKind};
use crate::view::label::Label;
use crate::color::{BLACK, WHITE};
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, NORMAL_STYLE};
use crate::geom::{BorderSpec, Point};
use crate::unit::scale_by_dpi;
use crate::view::THICKNESS_SMALL;
use crate::view::{Align, Bus, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::framebuffer::UpdateMode;
use inoink::model::ArticleId;

pub struct ArticleBar {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    article: ArticleId,
}

impl ArticleBar {
    pub fn new(rect: Rectangle, article: ArticleId, title: String, starred: bool, switch: Option<(DocKind, String)>, _context: &mut Context) -> ArticleBar {
        let id = ID_FEEDER.next();
        let side = rect.height() as i32;
        let mut children: Vec<Box<dyn View>> = Vec::new();
        children.push(Box::new(Icon::new("back", rect![rect.min, rect.min + side], Event::Back)));
        let more_rect = rect![rect.max.x - side, rect.min.y, rect.max.x, rect.max.y];
        let done_rect = rect![rect.max.x - 2 * side, rect.min.y, rect.max.x - side, rect.max.y];
        let star_rect = rect![rect.max.x - 3 * side, rect.min.y, rect.max.x - 2 * side, rect.max.y];
        let aa_rect = rect![rect.max.x - 4 * side, rect.min.y, rect.max.x - 3 * side, rect.max.y];
        let title_rect = rect![rect.min.x + side, rect.min.y, aa_rect.min.x, rect.max.y];
        match switch {
            Some((current, comments)) => {
                // A link post: an Article | comments switch replaces the title.
                let w = (title_rect.width() as i32).min(side * 6);
                let cx = (title_rect.min.x + title_rect.max.x) / 2;
                let r = rect![cx - w / 2, title_rect.min.y, cx + w / 2, title_rect.max.y];
                children.push(Box::new(DocSwitch::new(r, article, current, comments)));
            }
            None => children.push(Box::new(Label::new(title_rect, title, Align::Center))),
        }
        children.push(Box::new(Icon::new(if starred { "bookmark-filled" } else { "bookmark" }, star_rect, Event::ArticleAction(article, ArticleAction::ToggleStar))));
        children.push(Box::new(Icon::new("archive", done_rect, Event::ArticleAction(article, ArticleAction::Done))));
        children.push(Box::new(Icon::new("more", more_rect, Event::ToggleNear(ViewId::ArticleMenu, more_rect))));
        children.push(Box::new(Label::new(aa_rect, "Aa".to_string(), Align::Center).event(Some(Event::Toggle(ViewId::TextPanel)))));
        ArticleBar { id, rect, children, article }
    }

    /// The bar of a linked page: Back, its address and the link menu.
    pub fn for_web(rect: Rectangle, title: String) -> ArticleBar {
        let side = rect.height() as i32;
        let more_rect = rect![rect.max.x - side, rect.min.y, rect.max.x, rect.max.y];
        let title_rect = rect![rect.min.x + side, rect.min.y, more_rect.min.x, rect.max.y];
        let children: Vec<Box<dyn View>> = vec![
            Box::new(Icon::new("back", rect![rect.min, rect.min + side], Event::Back)),
            Box::new(Label::new(title_rect, title, Align::Center)),
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

/// "Article | 12 comments": the half that is not showing switches documents when tapped.
pub struct DocSwitch {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    article: ArticleId,
    current: DocKind,
    comments: String,
}

impl DocSwitch {
    pub fn new(rect: Rectangle, article: ArticleId, current: DocKind, comments: String) -> DocSwitch {
        DocSwitch { id: ID_FEEDER.next(), rect, children: Vec::new(), article, current, comments }
    }

    fn halves(&self) -> (Rectangle, Rectangle) {
        let inset = self.rect.height() as i32 / 6;
        let mid = (self.rect.min.x + self.rect.max.x) / 2;
        (
            rect![self.rect.min.x, self.rect.min.y + inset, mid, self.rect.max.y - inset],
            rect![mid, self.rect.min.y + inset, self.rect.max.x, self.rect.max.y - inset],
        )
    }
}

impl View for DocSwitch {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) if self.rect.includes(c) => {
                let (left, right) = self.halves();
                if self.current == DocKind::Comments && left.includes(c) {
                    bus.push_back(Event::ShowDocument(self.article, DocKind::Article));
                } else if self.current == DocKind::Article && right.includes(c) {
                    bus.push_back(Event::ShowDocument(self.article, DocKind::Comments));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let (left, right) = self.halves();
        fb.draw_rectangle(&self.rect, WHITE);
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        for (half, text, active) in [(left, "Article", self.current == DocKind::Article), (right, self.comments.as_str(), self.current == DocKind::Comments)] {
            fb.draw_rectangle(&half, if active { BLACK } else { WHITE });
            let plan = font.plan(text, Some(half.width() as i32), None);
            let x = half.min.x + (half.width() as i32 - plan.width) / 2;
            let y = half.min.y + (half.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, if active { WHITE } else { BLACK }, &plan, Point::new(x, y));
        }
        let both = rect![left.min.x, left.min.y, right.max.x, right.max.y];
        fb.draw_rectangle_outline(&both, &BorderSpec { thickness: scale_by_dpi(THICKNESS_SMALL, dpi) as u16, color: BLACK });
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
