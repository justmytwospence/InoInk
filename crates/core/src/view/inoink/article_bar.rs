//! The reader's only chrome: Back, the title, Star, Done and More.

use crate::context::Context;
use crate::framebuffer::Framebuffer;
use crate::font::Fonts;
use crate::geom::Rectangle;
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::view::icon::Icon;
use crate::view::inoink::actions::ArticleAction;
use crate::view::label::Label;
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
    pub fn new(rect: Rectangle, article: ArticleId, title: String, starred: bool, _context: &mut Context) -> ArticleBar {
        let id = ID_FEEDER.next();
        let side = rect.height() as i32;
        let mut children: Vec<Box<dyn View>> = Vec::new();
        children.push(Box::new(Icon::new("back", rect![rect.min, rect.min + side], Event::Back)));
        let more_rect = rect![rect.max.x - side, rect.min.y, rect.max.x, rect.max.y];
        let done_rect = rect![rect.max.x - 2 * side, rect.min.y, rect.max.x - side, rect.max.y];
        let star_rect = rect![rect.max.x - 3 * side, rect.min.y, rect.max.x - 2 * side, rect.max.y];
        let aa_rect = rect![rect.max.x - 4 * side, rect.min.y, rect.max.x - 3 * side, rect.max.y];
        let title_rect = rect![rect.min.x + side, rect.min.y, aa_rect.min.x, rect.max.y];
        children.push(Box::new(Label::new(title_rect, title, Align::Center)));
        children.push(Box::new(Icon::new(if starred { "star-filled" } else { "star" }, star_rect, Event::ArticleAction(article, ArticleAction::ToggleStar))));
        children.push(Box::new(Icon::new("check_mark", done_rect, Event::ArticleAction(article, ArticleAction::Done))));
        children.push(Box::new(Icon::new("menu", more_rect, Event::ToggleNear(ViewId::ArticleMenu, more_rect))));
        children.push(Box::new(Label::new(aa_rect, "Aa".to_string(), Align::Center).event(Some(Event::Toggle(ViewId::TextPanel)))));
        ArticleBar { id, rect, children, article }
    }

    pub fn article(&self) -> ArticleId {
        self.article
    }

    /// Updates the star icon after a change.
    pub fn set_starred(&mut self, starred: bool, rq: &mut RenderQueue) {
        if let Some(icon) = self.children[2].downcast_mut::<Icon>() {
            let name = if starred { "star-filled" } else { "star" };
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
