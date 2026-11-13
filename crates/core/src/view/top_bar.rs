use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::view::{View, Event, EntryId, Hub, Bus, Id, ID_FEEDER, RenderQueue, RenderData, ViewId, Align};
use crate::view::icon::Icon;
use crate::view::clock::Clock;
use crate::view::battery::Battery;
use crate::view::label::Label;
use crate::geom::{Rectangle};
use crate::font::Fonts;
use crate::context::Context;

/// The Wi-Fi icon for the current state: online, switched on but not connected yet, or off.
pub fn wifi_icon_name(context: &Context) -> &'static str {
    if context.online {
        "wifi"
    } else if context.settings.wifi {
        "wifi-connecting"
    } else {
        "wifi-off"
    }
}

#[derive(Debug)]
pub struct TopBar {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
}

impl TopBar {
    pub fn new(rect: Rectangle, root_event: Event, title: String, context: &mut Context) -> TopBar {
        let id = ID_FEEDER.next();
        let mut children = Vec::new();

        let side = rect.height() as i32;
        let icon_name = match root_event {
            Event::Back => "back",
            _ => "search",
        };

        let root_icon = Icon::new(icon_name,
                                  rect![rect.min, rect.min+side],
                                  root_event);
        children.push(Box::new(root_icon) as Box<dyn View>);

        let mut clock_rect = rect![rect.max - pt!(5*side, side),
                                   rect.max - pt!(4*side, 0)];
        let clock_label = Clock::new(&mut clock_rect, context);
        let title_rect = rect![rect.min.x + side, rect.min.y,
                               clock_rect.min.x, rect.max.y];
        let title_label = Label::new(title_rect, title, Align::Center)
                                .event(Some(Event::ToggleNear(ViewId::TitleMenu, title_rect)));
        children.push(Box::new(title_label) as Box<dyn View>);
        children.push(Box::new(clock_label) as Box<dyn View>);

        let capacity = context.battery.capacity().map_or(0.0, |v| v[0]);
        let status = context.battery.status().map_or(crate::battery::Status::Discharging, |v| v[0]);
        let battery_widget = Battery::new(rect![rect.max - pt!(3*side, side),
                                                rect.max - pt!(2*side, 0)],
                                          capacity,
                                          status);
        children.push(Box::new(battery_widget) as Box<dyn View>);

        let name = if context.settings.frontlight { "frontlight" } else { "frontlight-disabled" };
        let frontlight_icon = Icon::new(name,
                                        rect![rect.max - pt!(2*side, side),
                                              rect.max - pt!(side, 0)],
                                        Event::Show(ViewId::Frontlight));
        children.push(Box::new(frontlight_icon) as Box<dyn View>);

        let menu_rect = rect![rect.max-side, rect.max];
        let menu_icon = Icon::new("menu",
                                  menu_rect,
                                  Event::ToggleNear(ViewId::MainMenu, menu_rect));
        children.push(Box::new(menu_icon) as Box<dyn View>);

        // Wi-Fi state, after the menu so the other children keep their indexes.
        let wifi_icon = Icon::new(wifi_icon_name(context),
                                  rect![rect.max - pt!(4*side, side),
                                        rect.max - pt!(3*side, 0)],
                                  Event::Select(EntryId::ToggleWifi));
        children.push(Box::new(wifi_icon) as Box<dyn View>);

        TopBar {
            id,
            rect,
            children,
        }
    }

    /// The article-list layout: the clock and battery, which are only read, on the left; every
    /// button together on the right (search, then `actions`, Wi-Fi, light, menu). The title sits
    /// between them.
    pub fn with_home_layout(mut self, actions: Vec<(&str, Event)>) -> TopBar {
        let r = self.rect;
        let side = r.height() as i32;
        // The icons on the right are drawn centred, so a narrower slot only trims blank space
        // and leaves the title more room.
        let slot = side * 4 / 5;
        let mut put = |child: &mut Box<dyn View>, x0: i32, x1: i32| {
            *child.rect_mut() = rect![x0, r.min.y, x1, r.max.y];
        };
        let clock_w = self.children[2].rect().width() as i32;
        let x = r.min.x + side / 4;
        put(&mut self.children[2], x, x + clock_w);
        put(&mut self.children[3], x + clock_w, x + clock_w + side);
        let left_end = x + clock_w + side;
        let mut x = r.max.x;
        for i in [5usize, 4, 6] {
            put(&mut self.children[i], x - slot, x);
            x -= slot;
        }
        for (name, event) in actions.into_iter().rev() {
            self.children.push(Box::new(Icon::new(name, rect![x - slot, r.min.y, x, r.max.y], event)) as Box<dyn View>);
            x -= slot;
        }
        put(&mut self.children[0], x - slot, x);
        x -= slot;
        let title_rect = rect![left_end, r.min.y, x.max(left_end), r.max.y];
        self.children[1] = Box::new(Label::new(title_rect, String::new(), Align::Center).bold().event(Some(Event::ToggleNear(ViewId::TitleMenu, title_rect))));
        // Nothing else covers the margin before the clock: keep it white.
        self.children.push(Box::new(crate::view::filler::Filler::new(rect![r.min.x, r.min.y, r.min.x + side / 4, r.max.y], crate::color::WHITE)) as Box<dyn View>);
        // The menu opens from the menu icon, wherever it now is.
        let menu_rect = *self.children[5].rect();
        self.children[5] = Box::new(Icon::new("menu", menu_rect, Event::ToggleNear(ViewId::MainMenu, menu_rect)));
        self
    }

    pub fn update_root_icon(&mut self, name: &str, rq: &mut RenderQueue) {
        let icon = self.child_mut(0).downcast_mut::<Icon>().unwrap();
        if icon.name != name {
            icon.name = name.to_string();
            rq.add(RenderData::new(icon.id(), *icon.rect(), UpdateMode::Gui));
        }
    }

    pub fn update_title_label(&mut self, title: &str, rq: &mut RenderQueue) {
        let title_label = self.children[1].as_mut().downcast_mut::<Label>().unwrap();
        title_label.update(title, rq);
    }

    pub fn update_frontlight_icon(&mut self, rq: &mut RenderQueue, context: &mut Context) {
        let name = if context.settings.frontlight { "frontlight" } else { "frontlight-disabled" };
        let icon = self.child_mut(4).downcast_mut::<Icon>().unwrap();
        icon.name = name.to_string();
        rq.add(RenderData::new(icon.id(), *icon.rect(), UpdateMode::Gui));
    }

    pub fn update_wifi_icon(&mut self, rq: &mut RenderQueue, context: &mut Context) {
        let name = wifi_icon_name(context);
        if let Some(icon) = self.children.get_mut(6).and_then(|c| c.downcast_mut::<Icon>()) {
            if icon.name != name {
                icon.name = name.to_string();
                rq.add(RenderData::new(icon.id(), *icon.rect(), UpdateMode::Gui));
            }
        }
    }

    pub fn update_clock_label(&mut self, rq: &mut RenderQueue) {
        if let Some(clock_label) = self.children[2].downcast_mut::<Clock>() {
            clock_label.update(rq);
        }
    }

    pub fn update_battery_widget(&mut self, rq: &mut RenderQueue, context: &mut Context) {
        if let Some(battery_widget) = self.children[3].downcast_mut::<Battery>() {
            battery_widget.update(rq, context);
        }
    }

    pub fn reseed(&mut self, rq: &mut RenderQueue, context: &mut Context) {
        self.update_frontlight_icon(rq, context);
        self.update_wifi_icon(rq, context);
        self.update_clock_label(rq);
        self.update_battery_widget(rq, context);
    }
}

impl View for TopBar {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(center)) |
            Event::Gesture(GestureEvent::HoldFingerShort(center, ..)) if self.rect.includes(center) => true,
            Event::Gesture(GestureEvent::Swipe { start, end, .. }) if self.rect.includes(start) && self.rect.includes(end) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, _fb: &mut dyn Framebuffer, _rect: Rectangle, _fonts: &mut Fonts) {
    }

    fn resize(&mut self, rect: Rectangle, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
        let side = rect.height() as i32;
        self.children[0].resize(rect![rect.min, rect.min+side], hub, rq, context);
        let clock_width = self.children[2].rect().width() as i32;
        let clock_rect = rect![rect.max - pt!(4*side + clock_width, side),
                               rect.max - pt!(4*side, 0)];
        self.children[1].resize(rect![rect.min.x + side,
                                      rect.min.y,
                                      clock_rect.min.x,
                                      rect.max.y],
                                hub, rq, context);
        self.children[2].resize(clock_rect, hub, rq, context);
        self.children[3].resize(rect![rect.max - pt!(3*side, side),
                                      rect.max - pt!(2*side, 0)],
                                hub, rq, context);
        self.children[4].resize(rect![rect.max - pt!(2*side, side),
                                      rect.max - pt!(side, 0)],
                                hub, rq, context);
        self.children[5].resize(rect![rect.max-side, rect.max],
                                hub, rq, context);
        if let Some(wifi) = self.children.get_mut(6) {
            wifi.resize(rect![rect.max - pt!(4*side, side),
                              rect.max - pt!(3*side, 0)],
                        hub, rq, context);
        }
        self.rect = rect;
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
