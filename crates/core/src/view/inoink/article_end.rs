//! The end-of-article popup: mark the article read or unstar it, or keep it, and choose where
//! that leads (the next article or the list). Opened by paging past the last page.

use crate::color::{BLACK, TEXT_INVERTED_HARD, TEXT_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{BorderSpec, CornerSpec, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{ButtonCode, ButtonStatus, DeviceEvent};
use crate::settings::AfterEnd;
use crate::unit::scale_by_dpi;
use crate::view::inoink::actions::{EndChoice, InoInkEntry};
use crate::view::{Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::view::{BORDER_RADIUS_LARGE, BORDER_RADIUS_MEDIUM, THICKNESS_LARGE, THICKNESS_MEDIUM};

const TITLE: &str = "End of article";
const THEN: &str = "Then";
const SEGMENTS: [(&str, AfterEnd); 2] = [("Next article", AfterEnd::Next), ("Back to list", AfterEnd::List)];

pub struct ArticleEnd {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    title_rect: Rectangle,
    rows: Vec<(Rectangle, EndChoice, String)>,
    then_rect: Rectangle,
    toggle: [Rectangle; 2],
    after: AfterEnd,
}

impl ArticleEnd {
    /// `starred`: the Unstar row is offered; `read_later`: it is called "Remove from Read later";
    /// `read`: Keep unread becomes Close.
    pub fn new(screen: Rectangle, starred: bool, read_later: bool, read: bool, after: AfterEnd, context: &mut Context) -> ArticleEnd {
        let dpi = CURRENT_DEVICE.dpi;
        let font = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi);
        let x_height = font.x_heights.0 as i32;
        let padding = font.em() as i32;
        let row_h = 4 * x_height;

        let mut choices = vec![(EndChoice::MarkRead, "Mark read".to_string())];
        if starred {
            let label = if read_later { "Remove from Read later" } else { "Unstar" };
            choices.push((EndChoice::Unstar, label.to_string()));
        }
        choices.push((EndChoice::KeepUnread, if read { "Close" } else { "Keep unread" }.to_string()));

        let width = screen.width() as i32 * 2 / 3;
        // Title, the rows, then the toggle line, each a row high with padding between.
        let n = choices.len() as i32 + 2;
        let height = n * row_h + (n + 1) * padding;
        let x0 = screen.min.x + (screen.width() as i32 - width) / 2;
        let y0 = screen.min.y + (screen.height() as i32 - height) / 2;
        let rect = rect![x0, y0, x0 + width, y0 + height];

        let inner = |i: i32| rect![x0 + padding, y0 + padding + i * (row_h + padding), x0 + width - padding, y0 + padding + i * (row_h + padding) + row_h];
        let title_rect = inner(0);
        let rows = choices.into_iter().enumerate().map(|(i, (c, l))| (inner(i as i32 + 1), c, l)).collect::<Vec<_>>();
        let line = inner(n - 1);
        let then_w = font.plan(THEN, None, None).width + padding;
        let then_rect = rect![line.min.x, line.min.y, line.min.x + then_w, line.max.y];
        let seg_w = (line.max.x - then_rect.max.x) / 2;
        let toggle = [
            rect![then_rect.max.x, line.min.y, then_rect.max.x + seg_w, line.max.y],
            rect![then_rect.max.x + seg_w, line.min.y, line.max.x, line.max.y],
        ];
        ArticleEnd { id: ID_FEEDER.next(), rect, children: Vec::new(), title_rect, rows, then_rect, toggle, after }
    }

    /// The choices offered, top to bottom.
    pub fn choices(&self) -> Vec<EndChoice> {
        self.rows.iter().map(|r| r.1).collect()
    }

    /// Next article, Back to list.
    pub fn toggle_rects(&self) -> [Rectangle; 2] {
        self.toggle
    }
}

fn render_text(fb: &mut dyn Framebuffer, fonts: &mut Fonts, rect: &Rectangle, text: &str, color: crate::color::Color) {
    let dpi = CURRENT_DEVICE.dpi;
    let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
    let x_height = font.x_heights.0 as i32;
    let plan = font.plan(text, Some(rect.width() as i32), None);
    let dx = (rect.width() as i32 - plan.width) / 2;
    let dy = (rect.height() as i32 - x_height) / 2;
    font.render(fb, color, &plan, pt!(rect.min.x + dx, rect.max.y - dy));
}

impl View for ArticleEnd {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) => {
                if !self.rect.includes(c) {
                    bus.push_back(Event::Close(ViewId::ArticleEnd));
                } else if let Some(&(_, choice, _)) = self.rows.iter().find(|r| r.0.includes(c)) {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(choice))));
                } else if let Some(i) = self.toggle.iter().position(|r| r.includes(c)) {
                    let after = SEGMENTS[i].1;
                    if after != self.after {
                        self.after = after;
                        context.settings.inoink.after_end = after;
                        context.inoink.settings.after_end = after;
                        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                    }
                }
                true
            }
            Event::Gesture(..) => true,
            Event::Device(DeviceEvent::Button { code: code @ (ButtonCode::Forward | ButtonCode::Backward), status, .. }) => {
                if status == ButtonStatus::Released {
                    bus.push_back(if code == ButtonCode::Forward {
                        Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(EndChoice::Skip)))
                    } else {
                        Event::Close(ViewId::ArticleEnd)
                    });
                }
                true
            }
            Event::Device(DeviceEvent::Finger { .. }) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let thick = scale_by_dpi(THICKNESS_LARGE, dpi) as u16;
        fb.draw_rounded_rectangle_with_border(&self.rect, &CornerSpec::Uniform(radius), &BorderSpec { thickness: thick, color: BLACK }, &WHITE);
        render_text(fb, fonts, &self.title_rect, TITLE, BLACK);

        let button_radius = scale_by_dpi(BORDER_RADIUS_LARGE, dpi) as i32;
        let button_thick = scale_by_dpi(THICKNESS_MEDIUM, dpi) as u16;
        for (r, _, label) in &self.rows {
            fb.draw_rounded_rectangle_with_border(r, &CornerSpec::Uniform(button_radius), &BorderSpec { thickness: button_thick, color: BLACK }, &WHITE);
            render_text(fb, fonts, r, label, BLACK);
        }

        render_text(fb, fonts, &self.then_rect, THEN, BLACK);
        for (i, (label, after)) in SEGMENTS.iter().enumerate() {
            let scheme = if *after == self.after { TEXT_INVERTED_HARD } else { TEXT_NORMAL };
            let corners = if i == 0 { CornerSpec::West(button_radius) } else { CornerSpec::East(button_radius) };
            let r = &self.toggle[i];
            fb.draw_rounded_rectangle_with_border(r, &corners, &BorderSpec { thickness: button_thick, color: BLACK }, &scheme[0]);
            render_text(fb, fonts, r, label, scheme[1]);
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
        Some(ViewId::ArticleEnd)
    }
}
