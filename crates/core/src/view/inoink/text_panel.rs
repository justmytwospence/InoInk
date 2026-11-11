//! The reading text panel, modelled on the Kobo firmware's: font, size, line spacing, margins and
//! alignment, in a panel at the bottom of the article. Settings apply to every article.

use crate::color::{BLACK, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::Rectangle;
use crate::gesture::GestureEvent;
use crate::input::{DeviceEvent, FingerStatus};
use crate::metadata::TextAlign;
use crate::settings::ReaderSettings;
use crate::unit::scale_by_dpi;
use crate::view::icon::Icon;
use crate::view::label::Label;
use crate::view::slider::Slider;
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, SliderId, View, ViewId, ID_FEEDER, THICKNESS_MEDIUM};

struct Stepper {
    rect: Rectangle,
    id: SliderId,
    delta: f32,
}

pub struct TextPanel {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    font_size: f32,
    line_height: f32,
    margin: i32,
    align: TextAlign,
    steppers: Vec<Stepper>,
    ranges: [(f32, f32); 3],
    // Indexes into `children`.
    family_label: usize,
    sliders: [usize; 3],
    align_rects: [Rectangle; 2],
}

fn snap(id: SliderId, v: f32) -> f32 {
    match id {
        SliderId::FontSize => (v * 2.0).round() / 2.0,
        SliderId::LineHeight => (v * 10.0).round() / 10.0,
        _ => v.round(),
    }
}

impl TextPanel {
    pub fn new(rect: Rectangle, family: &str, font_size: f32, line_height: f32, margin: i32, align: TextAlign, settings: &ReaderSettings) -> TextPanel {
        let dpi = CURRENT_DEVICE.dpi;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let top = rect.min.y + thickness;
        let row_h = (rect.max.y - top) / 5;
        let padding = row_h / 3;
        let label_w = rect.width() as i32 * 3 / 10;
        let ctl_x = rect.min.x + label_w;
        let ranges = [
            (settings.min_font_size, settings.max_font_size),
            (1.0, 2.0),
            (settings.min_margin_width as f32, settings.max_margin_width as f32),
        ];
        let mut children: Vec<Box<dyn View>> = Vec::new();
        let names = ["Font", "Size", "Line spacing", "Margins", "Alignment"];
        let row = |i: usize| rect![rect.min.x, top + i as i32 * row_h, rect.max.x, top + (i as i32 + 1) * row_h];
        for (i, name) in names.iter().enumerate() {
            let r = row(i);
            children.push(Box::new(Label::new(rect![r.min.x, r.min.y, ctl_x, r.max.y], name.to_string(), Align::Left(padding))));
        }
        // Font family drop-down.
        let fam_rect = rect![ctl_x, row(0).min.y, rect.max.x, row(0).max.y];
        let family_label = children.len();
        children.push(Box::new(Label::new(fam_rect, format!("{family} \u{25be}"), Align::Left(0)).event(Some(Event::ToggleNear(ViewId::FontFamilyMenu, fam_rect)))));

        let values = [font_size, line_height, margin as f32];
        let ids = [SliderId::FontSize, SliderId::LineHeight, SliderId::MarginWidth];
        let deltas = [0.5, 0.1, 1.0];
        let mut sliders = [0usize; 3];
        let mut steppers = Vec::new();
        for k in 0..3 {
            let r = row(k + 1);
            let minus = rect![ctl_x, r.min.y, ctl_x + row_h, r.max.y];
            let plus = rect![rect.max.x - row_h, r.min.y, rect.max.x, r.max.y];
            steppers.push(Stepper { rect: minus, id: ids[k], delta: -deltas[k] });
            steppers.push(Stepper { rect: plus, id: ids[k], delta: deltas[k] });
            sliders[k] = children.len();
            children.push(Box::new(Slider::new(rect![minus.max.x, r.min.y, plus.min.x, r.max.y], ids[k], values[k], ranges[k].0, ranges[k].1)));
        }
        // Alignment: left and justified.
        let r = row(4);
        let icon_h = row_h - 3 * thickness;
        let left = rect![ctl_x, r.min.y, ctl_x + row_h, r.min.y + icon_h];
        let just = rect![ctl_x + 2 * row_h, r.min.y, ctl_x + 3 * row_h, r.min.y + icon_h];
        children.push(Box::new(Icon::new("align-left", left, Event::Select(EntryId::SetTextAlign(TextAlign::Left)))));
        children.push(Box::new(Icon::new("align-justify", just, Event::Select(EntryId::SetTextAlign(TextAlign::Justify)))));

        TextPanel {
            id: ID_FEEDER.next(),
            rect,
            children,
            font_size,
            line_height,
            margin,
            align,
            steppers,
            ranges,
            family_label,
            sliders,
            align_rects: [left, just],
        }
    }

    pub fn update(&mut self, family: &str, font_size: f32, line_height: f32, margin: i32, align: TextAlign, rq: &mut RenderQueue) {
        self.font_size = font_size;
        self.line_height = line_height;
        self.margin = margin;
        if let Some(l) = self.children[self.family_label].downcast_mut::<Label>() {
            l.update(&format!("{family} \u{25be}"), rq);
        }
        let values = [font_size, line_height, margin as f32];
        for k in 0..3 {
            if let Some(s) = self.children[self.sliders[k]].downcast_mut::<Slider>() {
                s.update(values[k], rq);
            }
        }
        if self.align != align {
            self.align = align;
            let mut r = self.align_rects[0];
            r.absorb(&self.align_rects[1]);
            let thickness = scale_by_dpi(THICKNESS_MEDIUM, CURRENT_DEVICE.dpi) as i32;
            r.max.y += 3 * thickness;
            rq.add(RenderData::new(self.id, r, UpdateMode::Gui));
        }
    }

    fn current(&self, id: SliderId) -> (f32, usize) {
        match id {
            SliderId::FontSize => (self.font_size, 0),
            SliderId::LineHeight => (self.line_height, 1),
            _ => (self.margin as f32, 2),
        }
    }
}

impl View for TextPanel {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(c)) => {
                if !self.rect.includes(c) {
                    bus.push_back(Event::Close(ViewId::TextPanel));
                    return true;
                }
                let hit = self.steppers.iter().find(|s| s.rect.includes(c)).map(|s| (s.id, s.delta));
                if let Some((id, delta)) = hit {
                    let (cur, k) = self.current(id);
                    let (lo, hi) = self.ranges[k];
                    let v = snap(id, (cur + delta).clamp(lo, hi));
                    match id {
                        SliderId::FontSize => self.font_size = v,
                        SliderId::LineHeight => self.line_height = v,
                        _ => self.margin = v as i32,
                    }
                    if let Some(s) = self.children[self.sliders[k]].downcast_mut::<Slider>() {
                        s.update(v, rq);
                    }
                    bus.push_back(Event::Slider(id, v, FingerStatus::Up));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) | Event::Gesture(GestureEvent::HoldFingerLong(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, .. }) if self.rect.includes(start) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        fb.draw_rectangle(&self.rect, WHITE);
        fb.draw_rectangle(&rect![self.rect.min.x, self.rect.min.y, self.rect.max.x, self.rect.min.y + thickness], BLACK);
        // Stepper signs.
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        for s in &self.steppers {
            let text = if s.delta < 0.0 { "\u{2212}" } else { "+" };
            let plan = font.plan(text, None, None);
            let x = s.rect.min.x + (s.rect.width() as i32 - plan.width) / 2;
            let y = s.rect.min.y + (s.rect.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, BLACK, &plan, crate::geom::Point::new(x, y));
        }
        // Mark the active alignment.
        let r = if self.align == TextAlign::Justify { self.align_rects[1] } else { self.align_rects[0] };
        fb.draw_rectangle(&rect![r.min.x, r.max.y + thickness / 2, r.max.x, r.max.y + 5 * thickness / 2], BLACK);
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
        Some(ViewId::TextPanel)
    }
}
