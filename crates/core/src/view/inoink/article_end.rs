//! The end-of-article popup: what to do with the finished article (Mark read by default), the
//! next article of the list, and Back to list. Swiping on (or the forward button) opens the next
//! article; swiping back, the back button or a tap outside closes the popup. Opened by paging past
//! the last page.

use crate::color::{Color, BLACK, GRAY06, GRAY10, TEXT_INVERTED_HARD, TEXT_NORMAL, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Font, Fonts, RenderPlan, LIST_META, MD_KIND, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{BorderSpec, CornerSpec, Dir, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{ButtonCode, ButtonStatus, DeviceEvent};
use crate::settings::ButtonScheme;
use crate::unit::scale_by_dpi;
use crate::view::inoink::actions::{EndChoice, EndState, InoInkEntry};
use crate::view::{Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, View, ViewId, ID_FEEDER};
use crate::view::{BORDER_RADIUS_LARGE, BORDER_RADIUS_MEDIUM, THICKNESS_LARGE, THICKNESS_MEDIUM};
use std::time::{Duration, Instant};

const TITLE: &str = "End of article";
const BACK: &str = "Back to list";
/// Onward swipes and forward presses this soon after the popup opens are ignored, so a quick run
/// of page turns does not overshoot into the next article.
pub const CONTINUE_GRACE: Duration = Duration::from_millis(600);
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

pub struct ArticleEnd {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    title_rect: Rectangle,
    toggle: Vec<(Rectangle, EndState, &'static str)>,
    state: EndState,
    separator: Rectangle,
    next_rect: Rectangle,
    next_title: Option<String>,
    back_rect: Rectangle,
    opened: Instant,
}

impl ArticleEnd {
    /// `starred`: Archive is offered; `read`: Mark read is not (and Leave is called "Leave as is");
    /// `next_title`: the next article of the list, if any.
    pub fn new(screen: Rectangle, starred: bool, read: bool, next_title: Option<String>, context: &mut Context) -> ArticleEnd {
        let dpi = CURRENT_DEVICE.dpi;
        let mut options: Vec<(EndState, &'static str)> = Vec::new();
        if !read {
            options.push((EndState::MarkRead, "Mark read"));
        }
        if starred {
            options.push((EndState::Archive, "Archive"));
        }
        options.push((EndState::Leave, if read { "Leave as is" } else { "Keep unread" }));
        let state = if read { EndState::Leave } else { EndState::MarkRead };

        let (x_height, padding, tlh) = {
            let font = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi);
            (font.x_heights.0 as i32, font.em() as i32, font.line_height())
        };
        let row_h = 4 * x_height;
        let llh = font_from_style(&mut context.fonts, &MD_KIND, dpi).line_height();
        let hlh = font_from_style(&mut context.fonts, &LIST_META, dpi).line_height();

        let width = screen.width() as i32 * 2 / 3;
        let inner_w = width - 2 * padding;
        let lines = match next_title.as_ref() {
            Some(t) => {
                let font = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi);
                if title_lines(font, t, inner_w).1.is_some() { 2 } else { 1 }
            }
            None => 0,
        };
        let block_h = llh + llh / 4 + lines * tlh + tlh / 3 + hlh;
        let toggle_h = if options.len() >= 2 { row_h + padding } else { 0 };
        let height = padding + row_h + padding + toggle_h + 1 + padding + block_h + padding + row_h + padding;
        let x0 = screen.min.x + (screen.width() as i32 - width) / 2;
        let y0 = screen.min.y + (screen.height() as i32 - height) / 2;
        let rect = rect![x0, y0, x0 + width, y0 + height];
        let (x, x_end) = (x0 + padding, x0 + width - padding);

        let mut y = y0 + padding;
        let title_rect = rect![x, y, x_end, y + row_h];
        y += row_h + padding;
        let mut toggle = Vec::new();
        if options.len() >= 2 {
            let n = options.len() as i32;
            let seg_w = (x_end - x) / n;
            for (i, (s, label)) in options.iter().enumerate() {
                let i = i as i32;
                let x1 = if i == n - 1 { x_end } else { x + (i + 1) * seg_w };
                toggle.push((rect![x + i * seg_w, y, x1, y + row_h], *s, *label));
            }
            y += row_h + padding;
        }
        let separator = rect![x, y, x_end, y + 1];
        y += 1 + padding;
        let next_rect = rect![x, y, x_end, y + block_h];
        y += block_h + padding;
        let back_rect = rect![x, y, x_end, y + row_h];
        ArticleEnd { id: ID_FEEDER.next(), rect, children: Vec::new(), title_rect, toggle, state, separator, next_rect, next_title, back_rect, opened: Instant::now() }
    }

    /// The states offered, left to right (just the default when there is nothing to choose).
    pub fn choices(&self) -> Vec<EndState> {
        if self.toggle.is_empty() {
            vec![self.state]
        } else {
            self.toggle.iter().map(|t| t.1).collect()
        }
    }

    pub fn state(&self) -> EndState {
        self.state
    }

    pub fn toggle_rects(&self) -> Vec<Rectangle> {
        self.toggle.iter().map(|t| t.0).collect()
    }

    pub fn back_rect(&self) -> Rectangle {
        self.back_rect
    }

    pub fn next_title(&self) -> Option<&str> {
        self.next_title.as_deref()
    }
}

fn render_text(fb: &mut dyn Framebuffer, fonts: &mut Fonts, rect: &Rectangle, text: &str, color: Color) {
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
        let onward_ok = self.opened.elapsed() >= CONTINUE_GRACE;
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) => {
                if !self.rect.includes(c) {
                    bus.push_back(Event::Close(ViewId::ArticleEnd));
                } else if self.back_rect.includes(c) {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(EndChoice::BackToList(self.state)))));
                } else if let Some(&(_, state, _)) = self.toggle.iter().find(|t| t.0.includes(c)) {
                    if state != self.state {
                        self.state = state;
                        rq.add(RenderData::new(self.id, self.rect, UpdateMode::Gui));
                    }
                }
                true
            }
            Event::Gesture(GestureEvent::Swipe { dir, .. }) => {
                let next_is_up = context.settings.button_scheme != ButtonScheme::Inverted;
                let onward = match dir {
                    Dir::West => true,
                    Dir::East => false,
                    Dir::North => next_is_up,
                    Dir::South => !next_is_up,
                };
                if !onward {
                    bus.push_back(Event::Close(ViewId::ArticleEnd));
                } else if onward_ok {
                    bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(EndChoice::Continue(self.state)))));
                }
                true
            }
            Event::Gesture(..) => true,
            Event::Device(DeviceEvent::Button { code: code @ (ButtonCode::Forward | ButtonCode::Backward), status, .. }) => {
                if status == ButtonStatus::Released {
                    if code == ButtonCode::Backward {
                        bus.push_back(Event::Close(ViewId::ArticleEnd));
                    } else if onward_ok {
                        bus.push_back(Event::Select(EntryId::InoInk(InoInkEntry::ArticleEnd(EndChoice::Continue(self.state)))));
                    }
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
        let border = BorderSpec { thickness: button_thick, color: BLACK };
        let last = self.toggle.len().saturating_sub(1);
        for (i, (r, state, label)) in self.toggle.iter().enumerate() {
            let scheme = if *state == self.state { TEXT_INVERTED_HARD } else { TEXT_NORMAL };
            let corners = if i == 0 {
                CornerSpec::West(button_radius)
            } else if i == last {
                CornerSpec::East(button_radius)
            } else {
                CornerSpec::Uniform(0)
            };
            fb.draw_rounded_rectangle_with_border(r, &corners, &border, &scheme[0]);
            render_text(fb, fonts, r, label, scheme[1]);
        }

        fb.draw_rectangle(&self.separator, GRAY10);

        // The next article: a small label, its title (two lines at most) and the way to go on.
        let center = |width: i32| self.next_rect.min.x + (self.next_rect.width() as i32 - width) / 2;
        let top = self.next_rect.min.y;
        let (llh, lasc, label) = {
            let font = font_from_style(fonts, &MD_KIND, dpi);
            let mut plan = font.plan(if self.next_title.is_some() { "UP NEXT" } else { "END OF LIST" }, None, None);
            plan.space_out(scale_by_dpi(LABEL_TRACKING, dpi) as i32);
            (font.line_height(), font.ascender(), plan)
        };
        font_from_style(fonts, &MD_KIND, dpi).render(fb, GRAY06, &label, pt!(center(label.width), top + lasc));
        let mut lines_count = 0;
        let tlh = font_from_style(fonts, &NORMAL_STYLE, dpi).line_height();
        if let Some(title) = self.next_title.as_ref() {
            let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
            let asc = font.ascender();
            let (first, second) = title_lines(font, title, self.next_rect.width() as i32);
            let y = top + llh + llh / 4 + asc;
            font.render(fb, BLACK, &first, pt!(center(first.width), y));
            lines_count = 1;
            if let Some(second) = second {
                font.render(fb, BLACK, &second, pt!(center(second.width), y + tlh));
                lines_count = 2;
            }
        }
        let font = font_from_style(fonts, &LIST_META, dpi);
        let hint = font.plan(if self.next_title.is_some() { "Swipe to continue" } else { "Swipe to return to the list" }, None, None);
        let y = top + llh + llh / 4 + lines_count * tlh + tlh / 3 + font.ascender();
        font.render(fb, GRAY06, &hint, pt!(center(hint.width), y));

        fb.draw_rounded_rectangle_with_border(&self.back_rect, &CornerSpec::Uniform(button_radius), &border, &WHITE);
        render_text(fb, fonts, &self.back_rect, BACK, BLACK);
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
