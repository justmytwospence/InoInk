//! The card shown when the last page of an article is passed: finish it and continue.

use crate::color::WHITE;
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, LIST_META, NORMAL_STYLE};
use crate::framebuffer::Framebuffer;
use crate::geom::{Dir, Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{ButtonCode, ButtonStatus, DeviceEvent};
use crate::unit::scale_by_dpi;
use crate::view::button::Button;
use crate::view::inoink::actions::{FinishAction, InoInkEntry};
use crate::view::label::Label;
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderQueue, View, ViewId, BIG_BAR_HEIGHT, ID_FEEDER};

pub struct EndCardInfo {
    pub title: String,
    pub primary: String,
    pub keep: Option<String>,
    pub comments: Option<String>,
    pub next_title: Option<String>,
    /// Another forward swipe or press finishes the article.
    pub auto: bool,
    /// The release that ends the hold that opened the card must not count as a press.
    pub ignore_release: Option<ButtonCode>,
}

pub struct EndCard {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    auto: bool,
    ignore_release: Option<ButtonCode>,
    /// Lines of plain text drawn by the card itself: (text, centre y, small).
    texts: Vec<(String, i32, bool)>,
}

impl EndCard {
    pub fn new(rect: Rectangle, info: EndCardInfo) -> EndCard {
        let dpi = CURRENT_DEVICE.dpi;
        let big = scale_by_dpi(BIG_BAR_HEIGHT, dpi) as i32;
        let gap = big / 4;
        let mut children: Vec<Box<dyn View>> = Vec::new();
        let mut texts = Vec::new();
        let mut y = rect.min.y + big;
        children.push(Box::new(Label::new(rect![rect.min.x, y, rect.max.x, y + big], "End of article".to_string(), Align::Center)));
        y += big;
        texts.push((info.title.clone(), y + big / 4, true));
        y += big / 2 + gap;
        let w = rect.width() as i32 * 8 / 10;
        let x0 = rect.min.x + (rect.width() as i32 - w) / 2;
        let button = |label: String, action: FinishAction, y: &mut i32, children: &mut Vec<Box<dyn View>>| {
            let h = big * 5 / 4;
            children.push(Box::new(Button::new(rect![x0, *y, x0 + w, *y + h], Event::Select(EntryId::InoInk(InoInkEntry::Finish(action))), label)));
            *y += h + gap;
        };
        button(info.primary.clone(), FinishAction::Primary, &mut y, &mut children);
        if let Some(k) = info.keep.clone() {
            button(k, FinishAction::Keep, &mut y, &mut children);
        }
        if let Some(c) = info.comments.clone() {
            button(c, FinishAction::Comments, &mut y, &mut children);
        }
        button("Back to list".to_string(), FinishAction::BackToList, &mut y, &mut children);
        let next = match &info.next_title {
            Some(t) => format!("Next: {t}"),
            None => "This was the last article in the list".to_string(),
        };
        texts.push((next, y + big / 4, true));
        y += big / 2;
        if info.auto {
            texts.push(("Swipe or press forward to finish and continue".to_string(), y + big / 4, true));
        }
        EndCard { id: ID_FEEDER.next(), rect, children, auto: info.auto, ignore_release: info.ignore_release, texts }
    }
}

impl View for EndCard {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Device(DeviceEvent::Button { code, status: ButtonStatus::Released, .. }) => {
                if self.ignore_release == Some(code) {
                    self.ignore_release = None;
                    return true;
                }
                match code {
                    ButtonCode::Forward if self.auto => bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::Finish(FinishAction::Primary)))),
                    ButtonCode::Backward => bus.push_back(Event::Close(ViewId::EndCard)),
                    _ => {}
                }
                true
            }
            Event::Device(DeviceEvent::Button { .. }) => true,
            Event::Gesture(GestureEvent::Swipe { dir, start, .. }) if self.rect.includes(start) => {
                match dir {
                    Dir::West if self.auto => bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::Finish(FinishAction::Primary)))),
                    Dir::East => bus.push_back(Event::Close(ViewId::EndCard)),
                    _ => {}
                }
                true
            }
            Event::Gesture(GestureEvent::Tap(c)) | Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) | Event::Gesture(GestureEvent::HoldFingerLong(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Corner { .. }) | Event::Gesture(GestureEvent::Arrow { .. }) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => false,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        fb.draw_rectangle(&self.rect, WHITE);
        for (text, cy, small) in &self.texts {
            let font = font_from_style(fonts, if *small { &LIST_META } else { &NORMAL_STYLE }, dpi);
            let plan = font.plan(text, Some(self.rect.width() as i32 * 9 / 10), None);
            let x = self.rect.min.x + (self.rect.width() as i32 - plan.width) / 2;
            let y = cy + font.x_heights.0 as i32 / 2;
            font.render(fb, crate::color::BLACK, &plan, Point::new(x, y));
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

    fn is_background(&self) -> bool {
        true
    }

    fn view_id(&self) -> Option<ViewId> {
        Some(ViewId::EndCard)
    }
}
