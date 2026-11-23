//! The reading text panel, modelled on the Kobo firmware's: text presets, font, size, line
//! spacing, margins and alignment, in a panel at the bottom of the article. Settings apply to
//! every article. Presets follow Kobalt's (https://github.com/nicoverbruggen/kobalt, MIT): a row
//! of cards showing "Aa" in each preset's font, an asterisk once the text differs from the preset
//! in force, Update and Save as new.

use crate::color::{BLACK, GRAY08, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, FONT_SIZES, MD_SIZE, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{BorderSpec, CornerSpec, Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{DeviceEvent, FingerStatus};
use crate::metadata::TextAlign;
use crate::settings::{preset_in_force, ReaderSettings, TextPreset};
use crate::unit::scale_by_dpi;
use crate::view::icon::Icon;
use crate::view::label::Label;
use crate::view::slider::Slider;
use crate::view::{Align, Bus, EntryId, Event, Hub, Id, RenderData, RenderQueue, SliderId, View, ViewId, ID_FEEDER};
use crate::view::{BORDER_RADIUS_MEDIUM, SMALL_BAR_HEIGHT, THICKNESS_LARGE, THICKNESS_MEDIUM};

/// What a preset card or button asks the reader to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresetAction {
    /// Apply the preset at this index.
    Apply(usize),
    /// Name the current text as a new preset.
    SaveNew,
    /// Save the current text into the preset in force.
    Update,
    /// The card of the preset at this index was held (offer to delete it).
    Hold(usize, Rectangle),
}

/// What the panel shows: the current text, the margins and the presets.
#[derive(Debug, Clone, PartialEq)]
pub struct TextState {
    /// The current text (its name is unused).
    pub text: TextPreset,
    pub margin: i32,
    pub presets: Vec<TextPreset>,
    /// The preset last applied, saved or updated.
    pub chosen: Option<String>,
}

impl TextState {
    /// The preset in force and whether the text changed since.
    pub fn in_force(&self) -> Option<(usize, bool)> {
        preset_in_force(&self.presets, self.chosen.as_deref(), &self.text)
    }

    /// Update is offered when the text differs from the preset in force.
    pub fn can_update(&self) -> bool {
        matches!(self.in_force(), Some((_, true)))
    }

    /// Save as new is offered when no preset has this text.
    pub fn can_save(&self) -> bool {
        !self.presets.iter().any(|p| p.same_text(&self.text))
    }

    /// A card's label: the preset's name, with an asterisk when it is in force but modified.
    pub fn card_label(&self, index: usize) -> String {
        let name = &self.presets[index].name;
        match self.in_force() {
            Some((i, true)) if i == index => format!("{name}*"),
            _ => name.clone(),
        }
    }
}

struct Stepper {
    rect: Rectangle,
    id: SliderId,
    delta: f32,
}

/// Cards shown at once; with more presets, one slot becomes a pager.
const CARDS_PER_ROW: usize = 4;

pub struct TextPanel {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    state: TextState,
    steppers: Vec<Stepper>,
    ranges: [(f32, f32); 3],
    // Indexes into `children`.
    sliders: [usize; 3],
    family_rect: Rectangle,
    align_rects: [Rectangle; 2],
    cards_rect: Rectangle,
    save_rect: Rectangle,
    update_rect: Rectangle,
    page: usize,
}

pub fn snap(id: SliderId, v: f32) -> f32 {
    match id {
        SliderId::FontSize => (v * 2.0).round() / 2.0,
        SliderId::LineHeight => (v * 10.0).round() / 10.0,
        _ => v.round(),
    }
}

/// Height of the panel: the presets row (one and a half bars) and five bars of controls.
pub fn panel_height(dpi: u16) -> i32 {
    let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
    let row_h = scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32;
    thickness + 3 * row_h / 2 + 5 * row_h
}

/// The panel's five control rows (font, size, line spacing, margins, alignment), full width, and
/// where their controls start.
pub fn control_rows(rect: Rectangle) -> ([Rectangle; 5], i32) {
    let thickness = scale_by_dpi(THICKNESS_MEDIUM, CURRENT_DEVICE.dpi) as i32;
    let top = rect.min.y + thickness;
    // One and a half rows of cards, then five rows of controls.
    let row_h = 2 * (rect.max.y - top) / 13;
    let controls_top = rect.max.y - 5 * row_h;
    let rows = std::array::from_fn(|i| rect![rect.min.x, controls_top + i as i32 * row_h, rect.max.x, controls_top + (i as i32 + 1) * row_h]);
    (rows, rect.min.x + rect.width() as i32 * 3 / 10)
}

impl TextPanel {
    pub fn new(rect: Rectangle, state: TextState, settings: &ReaderSettings) -> TextPanel {
        let dpi = CURRENT_DEVICE.dpi;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let top = rect.min.y + thickness;
        let (rows, ctl_x) = control_rows(rect);
        let row_h = rows[0].height() as i32;
        let cards_h = rows[0].min.y - top;
        let padding = row_h / 3;
        let ranges = [
            (settings.min_font_size, settings.max_font_size),
            (1.0, 2.0),
            (settings.min_margin_width as f32, settings.max_margin_width as f32),
        ];
        let mut children: Vec<Box<dyn View>> = Vec::new();
        let names = ["Font", "Size", "Line spacing", "Margins", "Alignment"];
        let row = |i: usize| rows[i];
        for (i, name) in names.iter().enumerate() {
            let r = row(i);
            children.push(Box::new(Label::new(rect![r.min.x, r.min.y, ctl_x, r.max.y], name.to_string(), Align::Left(padding))));
        }
        // Presets: Save as new and Update in the label column, the cards beside them.
        let gap = padding / 2;
        let button_h = (cards_h - 3 * gap) / 2;
        let save_rect = rect![rect.min.x + padding, top + gap, ctl_x - gap, top + gap + button_h];
        let update_rect = rect![rect.min.x + padding, save_rect.max.y + gap, ctl_x - gap, save_rect.max.y + gap + button_h];
        let cards_rect = rect![ctl_x, top + gap, rect.max.x - padding, top + cards_h - gap];
        // Font family drop-down, drawn in the family's own face.
        let family_rect = rect![ctl_x, row(0).min.y, rect.max.x, row(0).max.y];

        let values = [state.text.font_size, state.text.line_height, state.margin as f32];
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
            children.push(Box::new(Slider::new(rect![minus.max.x, r.min.y, plus.min.x, r.max.y], ids[k], values[k], ranges[k].0, ranges[k].1).step(deltas[k])));
        }
        // Alignment: left and justified.
        let r = row(4);
        let icon_h = row_h - 3 * thickness;
        let left = rect![ctl_x, r.min.y, ctl_x + row_h, r.min.y + icon_h];
        let just = rect![ctl_x + 2 * row_h, r.min.y, ctl_x + 3 * row_h, r.min.y + icon_h];
        children.push(Box::new(Icon::new("align-left", left, Event::Select(EntryId::SetTextAlign(TextAlign::Left)))));
        children.push(Box::new(Icon::new("align-justify", just, Event::Select(EntryId::SetTextAlign(TextAlign::Justify)))));

        let mut panel = TextPanel {
            id: ID_FEEDER.next(),
            rect,
            children,
            state,
            steppers,
            ranges,
            sliders,
            family_rect,
            align_rects: [left, just],
            cards_rect,
            save_rect,
            update_rect,
            page: 0,
        };
        panel.page = panel.page_of_preset_in_force();
        panel
    }

    pub fn update(&mut self, state: TextState, rq: &mut RenderQueue) {
        let old = std::mem::replace(&mut self.state, state);
        let values = [self.state.text.font_size, self.state.text.line_height, self.state.margin as f32];
        for k in 0..3 {
            if let Some(s) = self.children[self.sliders[k]].downcast_mut::<Slider>() {
                s.update(values[k], rq);
            }
        }
        if old.text.font_family != self.state.text.font_family {
            rq.add(RenderData::new(self.id, self.family_rect, UpdateMode::Gui));
        }
        if old.text.text_align != self.state.text.text_align {
            rq.add(RenderData::new(self.id, self.align_area(), UpdateMode::Gui));
        }
        if old.presets != self.state.presets || old.in_force() != self.state.in_force() {
            if old.presets.len() != self.state.presets.len() || old.in_force().map(|p| p.0) != self.state.in_force().map(|p| p.0) {
                self.page = self.page_of_preset_in_force();
            }
            rq.add(RenderData::new(self.id, self.presets_area(), UpdateMode::Gui));
        } else if old.can_save() != self.state.can_save() {
            rq.add(RenderData::new(self.id, self.presets_area(), UpdateMode::Gui));
        }
    }

    pub fn state(&self) -> &TextState {
        &self.state
    }

    pub fn family_rect(&self) -> Rectangle {
        self.family_rect
    }

    pub fn save_rect(&self) -> Rectangle {
        self.save_rect
    }

    pub fn update_rect(&self) -> Rectangle {
        self.update_rect
    }

    /// The track of a slider (font size, line spacing, margins).
    pub fn slider_rect(&self, id: SliderId) -> Option<Rectangle> {
        let k = match id {
            SliderId::FontSize => 0,
            SliderId::LineHeight => 1,
            SliderId::MarginWidth => 2,
            _ => return None,
        };
        Some(*self.children[self.sliders[k]].rect())
    }

    /// The visible cards: preset index and rectangle.
    pub fn cards(&self) -> Vec<(usize, Rectangle)> {
        let (per_page, _) = self.layout();
        let start = self.page * per_page;
        let end = (start + per_page).min(self.state.presets.len());
        (start..end).enumerate().map(|(slot, index)| (index, self.slot_rect(slot))).collect()
    }

    /// The pager cell, when the presets don't fit on one row.
    pub fn pager_rect(&self) -> Option<Rectangle> {
        let (per_page, paged) = self.layout();
        paged.then(|| {
            let r = self.slot_rect(per_page);
            rect![r.min.x, r.min.y, self.cards_rect.max.x, r.max.y]
        })
    }

    fn pages(&self) -> usize {
        let (per_page, _) = self.layout();
        self.state.presets.len().div_ceil(per_page).max(1)
    }

    // Cards per page, and whether a pager is needed.
    fn layout(&self) -> (usize, bool) {
        if self.state.presets.len() <= CARDS_PER_ROW {
            (CARDS_PER_ROW, false)
        } else {
            (CARDS_PER_ROW - 1, true)
        }
    }

    fn slot_rect(&self, slot: usize) -> Rectangle {
        let gap = (self.cards_rect.height() as i32 / 10).max(1);
        let width = (self.cards_rect.width() as i32 - (CARDS_PER_ROW as i32 - 1) * gap) / CARDS_PER_ROW as i32;
        let x = self.cards_rect.min.x + slot as i32 * (width + gap);
        rect![x, self.cards_rect.min.y, x + width, self.cards_rect.max.y]
    }

    fn page_of_preset_in_force(&self) -> usize {
        let (per_page, _) = self.layout();
        self.state.in_force().map_or(0, |(i, _)| i / per_page).min(self.pages() - 1)
    }

    fn presets_area(&self) -> Rectangle {
        rect![self.rect.min.x, self.save_rect.min.y, self.rect.max.x, self.cards_rect.max.y]
    }

    fn align_area(&self) -> Rectangle {
        let mut r = self.align_rects[0];
        r.absorb(&self.align_rects[1]);
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, CURRENT_DEVICE.dpi) as i32;
        r.max.y += 3 * thickness;
        r
    }

    fn current(&self, id: SliderId) -> (f32, usize) {
        match id {
            SliderId::FontSize => (self.state.text.font_size, 0),
            SliderId::LineHeight => (self.state.text.line_height, 1),
            _ => (self.state.margin as f32, 2),
        }
    }

    fn render_button(&self, fb: &mut dyn Framebuffer, fonts: &mut Fonts, rect: Rectangle, text: &str, enabled: bool) {
        let dpi = CURRENT_DEVICE.dpi;
        let radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as u16;
        let color = if enabled { BLACK } else { GRAY08 };
        fb.draw_rounded_rectangle_with_border(&rect, &CornerSpec::Uniform(radius), &BorderSpec { thickness, color }, &WHITE);
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        let plan = font.plan(text, Some(rect.width() as i32 - 2 * thickness as i32), None);
        let x = rect.min.x + (rect.width() as i32 - plan.width) / 2;
        let y = rect.min.y + (rect.height() as i32 + font.x_heights.0 as i32) / 2;
        font.render(fb, color, &plan, Point::new(x, y));
    }

    fn render_card(&self, fb: &mut dyn Framebuffer, fonts: &mut Fonts, index: usize, rect: Rectangle) {
        let dpi = CURRENT_DEVICE.dpi;
        let preset = &self.state.presets[index];
        let selected = self.state.in_force().map(|p| p.0) == Some(index);
        let radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let thickness = scale_by_dpi(if selected { 2.0 * THICKNESS_LARGE } else { THICKNESS_MEDIUM }, dpi) as u16;
        fb.draw_rounded_rectangle_with_border(&rect, &CornerSpec::Uniform(radius), &BorderSpec { thickness, color: BLACK }, &WHITE);
        // The name below, inverted on the selected card.
        let font = font_from_style(fonts, &MD_SIZE, dpi);
        let band_h = 2 * font.x_heights.1 as i32 + 2 * thickness as i32;
        let band = rect![rect.min.x, rect.max.y - band_h, rect.max.x, rect.max.y];
        let fg = if selected {
            fb.draw_rounded_rectangle(&band, &CornerSpec::South(radius), BLACK);
            WHITE
        } else {
            BLACK
        };
        let label = self.state.card_label(index);
        let max_w = rect.width() as i32 - 4 * thickness as i32;
        let plan = font.plan(&label, Some(max_w), None);
        let x = rect.min.x + (rect.width() as i32 - plan.width) / 2;
        let y = band.min.y + (band.height() as i32 + font.x_heights.1 as i32) / 2 - thickness as i32;
        font.render(fb, fg, &plan, Point::new(x, y));
        // "Aa" in the preset's face (the interface font when it can't be loaded).
        let size = 2 * FONT_SIZES[1];
        let font = match fonts.has_reading_face(&preset.font_family) {
            true => fonts.reading_face(&preset.font_family, size, dpi).unwrap(),
            false => font_from_style(fonts, &NORMAL_STYLE, dpi),
        };
        let plan = font.plan("Aa", Some(max_w), None);
        let cap = font.ascender() * 7 / 10;
        let x = rect.min.x + (rect.width() as i32 - plan.width) / 2;
        let y = rect.min.y + (band.min.y - rect.min.y + cap) / 2;
        font.render(fb, BLACK, &plan, Point::new(x, y));
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
                if self.family_rect.includes(c) {
                    bus.push_back(Event::ToggleNear(ViewId::FontFamilyMenu, self.family_rect));
                    return true;
                }
                if let Some((index, _)) = self.cards().into_iter().find(|(_, r)| r.includes(c)) {
                    bus.push_back(Event::TextPreset(PresetAction::Apply(index)));
                    return true;
                }
                if self.pager_rect().is_some_and(|r| r.includes(c)) {
                    self.page = (self.page + 1) % self.pages();
                    rq.add(RenderData::new(self.id, self.presets_area(), UpdateMode::Gui));
                    return true;
                }
                if self.save_rect.includes(c) {
                    if self.state.can_save() {
                        bus.push_back(Event::TextPreset(PresetAction::SaveNew));
                    }
                    return true;
                }
                if self.update_rect.includes(c) {
                    if self.state.can_update() {
                        bus.push_back(Event::TextPreset(PresetAction::Update));
                    }
                    return true;
                }
                let hit = self.steppers.iter().find(|s| s.rect.includes(c)).map(|s| (s.id, s.delta));
                if let Some((id, delta)) = hit {
                    let (cur, k) = self.current(id);
                    let (lo, hi) = self.ranges[k];
                    let v = snap(id, (cur + delta).clamp(lo, hi));
                    match id {
                        SliderId::FontSize => self.state.text.font_size = v,
                        SliderId::LineHeight => self.state.text.line_height = v,
                        _ => self.state.margin = v as i32,
                    }
                    if let Some(s) = self.children[self.sliders[k]].downcast_mut::<Slider>() {
                        s.update(v, rq);
                    }
                    bus.push_back(Event::Slider(id, v, FingerStatus::Up));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerShort(c, ..)) if self.rect.includes(c) => {
                if let Some((index, rect)) = self.cards().into_iter().find(|(_, r)| r.includes(c)) {
                    bus.push_back(Event::TextPreset(PresetAction::Hold(index, rect)));
                }
                true
            }
            Event::Gesture(GestureEvent::HoldFingerLong(c, ..)) if self.rect.includes(c) => true,
            Event::Gesture(GestureEvent::Swipe { start, .. }) if self.rect.includes(start) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, rect: Rectangle, fonts: &mut Fonts) {
        // Draw only what `rect` covers: the children outside it are not redrawn.
        let Some(area) = self.rect.intersection(&rect) else { return };
        let dpi = CURRENT_DEVICE.dpi;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        fb.draw_rectangle(&area, WHITE);
        let rule = rect![self.rect.min.x, self.rect.min.y, self.rect.max.x, self.rect.min.y + thickness];
        if let Some(r) = rule.intersection(&area) {
            fb.draw_rectangle(&r, BLACK);
        }
        // Presets.
        if self.presets_area().overlaps(&area) {
            self.render_button(fb, fonts, self.save_rect, "Save as new", self.state.can_save());
            self.render_button(fb, fonts, self.update_rect, "Update", self.state.can_update());
            for (index, r) in self.cards() {
                self.render_card(fb, fonts, index, r);
            }
            if let Some(r) = self.pager_rect() {
                let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
                let text = format!("{}/{} \u{203a}", self.page + 1, self.pages());
                let plan = font.plan(&text, Some(r.width() as i32), None);
                let x = r.min.x + (r.width() as i32 - plan.width) / 2;
                let y = r.min.y + (r.height() as i32 + font.x_heights.0 as i32) / 2;
                font.render(fb, BLACK, &plan, Point::new(x, y));
            }
        }
        // The family's name in its own face.
        if self.family_rect.overlaps(&area) {
            let family = &self.state.text.font_family;
            let text = format!("{family} \u{25be}");
            let font = match fonts.has_reading_face(family) {
                true => fonts.reading_face(family, NORMAL_STYLE.size, dpi).unwrap(),
                false => font_from_style(fonts, &NORMAL_STYLE, dpi),
            };
            let r = self.family_rect;
            let plan = font.plan(&text, Some(r.width() as i32), None);
            let y = r.min.y + (r.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, BLACK, &plan, Point::new(r.min.x, y));
        }
        // Stepper signs.
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        for s in self.steppers.iter().filter(|s| s.rect.overlaps(&area)) {
            let text = if s.delta < 0.0 { "\u{2212}" } else { "+" };
            let plan = font.plan(text, None, None);
            let x = s.rect.min.x + (s.rect.width() as i32 - plan.width) / 2;
            let y = s.rect.min.y + (s.rect.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, BLACK, &plan, Point::new(x, y));
        }
        // Mark the active alignment.
        let r = if self.state.text.text_align == TextAlign::Justify { self.align_rects[1] } else { self.align_rects[0] };
        let mark = rect![r.min.x, r.max.y + thickness / 2, r.max.x, r.max.y + 5 * thickness / 2];
        if mark.overlaps(&area) {
            fb.draw_rectangle(&mark, BLACK);
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
        Some(ViewId::TextPanel)
    }
}
