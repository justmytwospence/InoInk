//! The light panel: on or off, brightness and warmth, in a panel at the bottom of the screen.

use crate::color::{BLACK, WHITE};
use crate::context::Context;
use crate::device::CURRENT_DEVICE;
use crate::font::{font_from_style, Fonts, NORMAL_STYLE};
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::{DeviceEvent, FingerStatus};
use crate::unit::scale_by_dpi;
use crate::view::label::Label;
use crate::view::slider::Slider;
use crate::view::{Align, Bus, Event, Hub, Id, RenderData, RenderQueue, SliderId, View, ViewId, ID_FEEDER, SMALL_BAR_HEIGHT, THICKNESS_MEDIUM};

struct Stepper {
    rect: Rectangle,
    id: SliderId,
    delta: f32,
}

pub struct LightPanel {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    toggle: Rectangle,
    steppers: Vec<Stepper>,
    /// Indexes into `children`.
    state_label: usize,
    intensity_slider: usize,
    warmth_slider: Option<usize>,
}

impl LightPanel {
    /// Height of the panel for this device.
    pub fn height() -> i32 {
        let dpi = CURRENT_DEVICE.dpi;
        let rows = if CURRENT_DEVICE.has_natural_light() { 3 } else { 2 };
        rows * scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32 + scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32
    }

    pub fn new(screen: Rectangle, context: &Context) -> LightPanel {
        let dpi = CURRENT_DEVICE.dpi;
        let thickness = scale_by_dpi(THICKNESS_MEDIUM, dpi) as i32;
        let natural = CURRENT_DEVICE.has_natural_light();
        let rows = if natural { 3 } else { 2 };
        let rect = rect![screen.min.x, screen.max.y - Self::height(), screen.max.x, screen.max.y];
        let top = rect.min.y + thickness;
        let row_h = (rect.max.y - top) / rows;
        let padding = row_h / 3;
        let label_w = rect.width() as i32 * 3 / 10;
        let ctl_x = rect.min.x + label_w;
        let row = |i: i32| rect![rect.min.x, top + i * row_h, rect.max.x, top + (i + 1) * row_h];
        let levels = context.frontlight.levels();
        let mut children: Vec<Box<dyn View>> = Vec::new();
        let mut names = vec!["Light", "Brightness"];
        if natural {
            names.push("Warmth");
        }
        for (i, name) in names.iter().enumerate() {
            let r = row(i as i32);
            children.push(Box::new(Label::new(rect![r.min.x, r.min.y, ctl_x, r.max.y], name.to_string(), Align::Left(padding))));
        }
        let state_label = children.len();
        let r0 = row(0);
        let toggle = rect![ctl_x, r0.min.y, rect.max.x, r0.max.y];
        children.push(Box::new(Label::new(toggle, state_text(context.settings.frontlight).to_string(), Align::Left(0))));

        let mut steppers = Vec::new();
        let mut add_slider = |i: i32, id: SliderId, value: f32, step: f32, children: &mut Vec<Box<dyn View>>| -> usize {
            let r = row(i);
            let minus = rect![ctl_x, r.min.y, ctl_x + row_h, r.max.y];
            let plus = rect![rect.max.x - row_h, r.min.y, rect.max.x, r.max.y];
            steppers.push(Stepper { rect: minus, id, delta: -step });
            steppers.push(Stepper { rect: plus, id, delta: step });
            let at = children.len();
            children.push(Box::new(Slider::new(rect![minus.max.x, r.min.y, plus.min.x, r.max.y], id, value, 0.0, 100.0)));
            at
        };
        let intensity_slider = add_slider(1, SliderId::LightIntensity, levels.intensity, 5.0, &mut children);
        let warmth_slider = if natural { Some(add_slider(2, SliderId::LightWarmth, levels.warmth, 10.0, &mut children)) } else { None };
        LightPanel { id: ID_FEEDER.next(), rect, children, toggle, steppers, state_label, intensity_slider, warmth_slider }
    }

    /// Applies a new value to the light, switching it on if it was off.
    fn apply(&mut self, id: SliderId, value: f32, bus: &mut Bus, rq: &mut RenderQueue, context: &mut Context) {
        if !context.settings.frontlight {
            context.settings.frontlight = true;
            let levels = context.settings.frontlight_levels;
            context.frontlight.set_warmth(levels.warmth);
            context.frontlight.set_intensity(levels.intensity);
            self.set_state_label(true, rq);
        }
        match id {
            SliderId::LightIntensity => context.frontlight.set_intensity(value),
            SliderId::LightWarmth => context.frontlight.set_warmth(value),
            _ => return,
        }
        context.settings.frontlight_levels = context.frontlight.levels();
        bus.push_back(Event::FrontlightChanged);
    }

    fn set_state_label(&mut self, on: bool, rq: &mut RenderQueue) {
        if let Some(l) = self.children[self.state_label].downcast_mut::<Label>() {
            l.update(state_text(on), rq);
        }
    }

    fn slider_index(&self, id: SliderId) -> Option<usize> {
        match id {
            SliderId::LightIntensity => Some(self.intensity_slider),
            SliderId::LightWarmth => self.warmth_slider,
            _ => None,
        }
    }
}

fn state_text(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

impl View for LightPanel {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, bus: &mut Bus, rq: &mut RenderQueue, context: &mut Context) -> bool {
        match *evt {
            Event::Slider(id @ (SliderId::LightIntensity | SliderId::LightWarmth), value, _) => {
                self.apply(id, value, bus, rq, context);
                true
            }
            Event::Gesture(GestureEvent::Tap(c)) => {
                if !self.rect.includes(c) {
                    bus.push_back(Event::Close(ViewId::Frontlight));
                    return true;
                }
                if self.toggle.includes(c) {
                    let on = !context.settings.frontlight;
                    context.set_frontlight(on);
                    self.set_state_label(on, rq);
                    if on {
                        let levels = context.frontlight.levels();
                        if let Some(s) = self.children[self.intensity_slider].downcast_mut::<Slider>() {
                            s.update(levels.intensity, rq);
                        }
                    }
                    bus.push_back(Event::FrontlightChanged);
                    return true;
                }
                let hit = self.steppers.iter().find(|s| s.rect.includes(c)).map(|s| (s.id, s.delta));
                if let Some((id, delta)) = hit {
                    let levels = context.frontlight.levels();
                    let cur = if id == SliderId::LightIntensity { levels.intensity } else { levels.warmth };
                    let v = (cur + delta).clamp(0.0, 100.0);
                    if let Some(i) = self.slider_index(id) {
                        if let Some(s) = self.children[i].downcast_mut::<Slider>() {
                            s.update(v, rq);
                        }
                    }
                    self.apply(id, v, bus, rq, context);
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
        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        for s in &self.steppers {
            let text = if s.delta < 0.0 { "\u{2212}" } else { "+" };
            let plan = font.plan(text, None, None);
            let x = s.rect.min.x + (s.rect.width() as i32 - plan.width) / 2;
            let y = s.rect.min.y + (s.rect.height() as i32 + font.x_heights.0 as i32) / 2;
            font.render(fb, BLACK, &plan, Point::new(x, y));
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
        Some(ViewId::Frontlight)
    }
}
