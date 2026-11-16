use std::thread;
use std::time::Duration;
use crate::device::CURRENT_DEVICE;
use crate::framebuffer::{Framebuffer, UpdateMode};
use crate::geom::{Rectangle, CornerSpec, BorderSpec};
use crate::font::{Fonts, font_from_style, NORMAL_STYLE};
use crate::color::{BLACK, WHITE, TEXT_NORMAL};
use super::{View, Event, Hub, Bus, Id, ID_FEEDER, RenderQueue, RenderData, ViewId};
use super::common::locate;
use super::{SMALL_BAR_HEIGHT, THICKNESS_LARGE, BORDER_RADIUS_MEDIUM};
use crate::gesture::GestureEvent;
use crate::input::DeviceEvent;
use crate::unit::scale_by_dpi;
use crate::context::Context;

const NOTIFICATION_CLOSE_DELAY: Duration = Duration::from_secs(4);

pub struct Notification {
    id: Id,
    rect: Rectangle,
    children: Vec<Box<dyn View>>,
    text: String,
    max_width: i32,
    view_id: ViewId,
}

impl Notification {
    pub fn new(text: String, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) -> Notification {
        let id = ID_FEEDER.next();
        let view_id = ViewId::MessageNotif(id);
        let hub2 = hub.clone();

        thread::spawn(move || {
            thread::sleep(NOTIFICATION_CLOSE_DELAY);
            hub2.send(Event::Close(view_id)).ok();
        });

        let dpi = CURRENT_DEVICE.dpi;
        let (width, _) = context.display.dims;
        let small_height = scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32;

        let font = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi);
        let x_height = font.x_heights.0 as i32;
        let padding = font.em() as i32;

        let max_message_width = width as i32 - 5 * padding;
        let plan = font.plan(&text, Some(max_message_width), None);

        let dialog_width = plan.width + 3 * padding;
        let dialog_height = 7 * x_height;

        // Always in the same place: centred just below the top bar.
        let dx = (width as i32 - dialog_width) / 2;
        let dy = small_height + padding;

        let rect = rect![dx, dy,
                         dx + dialog_width, dy + dialog_height];

        rq.add(RenderData::new(id, rect, UpdateMode::Gui));

        Notification {
            id,
            rect,
            children: Vec::new(),
            text,
            max_width: max_message_width,
            view_id,
        }
    }
}

impl View for Notification {
    fn handle_event(&mut self, evt: &Event, _hub: &Hub, _bus: &mut Bus, _rq: &mut RenderQueue, _context: &mut Context) -> bool {
        match *evt {
            Event::Gesture(GestureEvent::Tap(center)) if self.rect.includes(center) => true,
            Event::Gesture(GestureEvent::Swipe { start, .. }) if self.rect.includes(start) => true,
            Event::Device(DeviceEvent::Finger { position, .. }) if self.rect.includes(position) => true,
            _ => false,
        }
    }

    fn render(&self, fb: &mut dyn Framebuffer, _rect: Rectangle, fonts: &mut Fonts) {
        let dpi = CURRENT_DEVICE.dpi;

        let border_radius = scale_by_dpi(BORDER_RADIUS_MEDIUM, dpi) as i32;
        let border_thickness = scale_by_dpi(THICKNESS_LARGE, dpi) as u16;

        fb.draw_rounded_rectangle_with_border(&self.rect,
                                              &CornerSpec::Uniform(border_radius),
                                              &BorderSpec { thickness: border_thickness,
                                                            color: BLACK },
                                              &WHITE);

        let font = font_from_style(fonts, &NORMAL_STYLE, dpi);
        let plan = font.plan(&self.text, Some(self.max_width), None);
        let x_height = font.x_heights.0 as i32;

        let dx = (self.rect.width() as i32 - plan.width) as i32 / 2;
        let dy = (self.rect.height() as i32 - x_height) / 2;
        let pt = pt!(self.rect.min.x + dx, self.rect.max.y - dy);

        font.render(fb, TEXT_NORMAL[1], &plan, pt);
    }

    fn resize(&mut self, _rect: Rectangle, _hub: &Hub, _rq: &mut RenderQueue, context: &mut Context) {
        let dpi = CURRENT_DEVICE.dpi;
        let (width, _) = context.display.dims;
        let small_height = scale_by_dpi(SMALL_BAR_HEIGHT, dpi) as i32;
        let padding = font_from_style(&mut context.fonts, &NORMAL_STYLE, dpi).em() as i32;
        let dialog_width = self.rect.width() as i32;
        let dialog_height = self.rect.height() as i32;
        let dx = (width as i32 - dialog_width) / 2;
        let dy = small_height + padding;
        self.rect = rect![dx, dy, dx + dialog_width, dy + dialog_height];
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
        Some(self.view_id)
    }
}

/// Shows `text` as the only notification of `view`: an earlier one still showing is replaced.
pub fn show_notification(view: &mut dyn View, text: String, hub: &Hub, rq: &mut RenderQueue, context: &mut Context) {
    while let Some(index) = locate::<Notification>(view) {
        rq.add(RenderData::expose(*view.child(index).rect(), UpdateMode::Gui));
        view.children_mut().remove(index);
    }
    let notif = Notification::new(text, hub, rq, context);
    view.children_mut().push(Box::new(notif) as Box<dyn View>);
}
