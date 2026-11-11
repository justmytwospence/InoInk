//! Brightness by sliding a finger up or down along the left edge, as in the Kobo firmware.

use crate::geom::{Point, Rectangle};
use crate::gesture::GestureEvent;
use crate::input::FingerStatus;

#[derive(Default)]
pub struct EdgeSlide {
    finger: Option<i32>,
    start: Point,
    start_intensity: f32,
    last: f32,
    active: bool,
    swallow: bool,
}

impl EdgeSlide {
    /// Feeds one touch event. Returns a new intensity (0 to 100) when the slide moves it.
    pub fn feed(&mut self, id: i32, status: FingerStatus, position: Point, screen: Rectangle, intensity: f32) -> Option<f32> {
        let strip = (screen.width() as i32 / 10).max(1);
        let threshold = (screen.height() as i32 / 40).max(1);
        match status {
            FingerStatus::Down => {
                self.swallow = false;
                if self.finger.is_some() {
                    self.finger = None;
                    self.active = false;
                } else if position.x < screen.min.x + strip {
                    self.finger = Some(id);
                    self.start = position;
                    self.start_intensity = intensity;
                    self.last = intensity;
                    self.active = false;
                }
                None
            }
            FingerStatus::Motion if self.finger == Some(id) => {
                let dy = self.start.y - position.y;
                let dx = (position.x - self.start.x).abs();
                if !self.active {
                    if dy.abs() >= threshold && dy.abs() > 2 * dx {
                        self.active = true;
                    } else if dx >= threshold {
                        self.finger = None;
                    }
                }
                if self.active {
                    let v = (self.start_intensity + dy as f32 * 100.0 / (0.75 * screen.height() as f32)).clamp(0.0, 100.0).round();
                    if v != self.last {
                        self.last = v;
                        return Some(v);
                    }
                }
                None
            }
            FingerStatus::Up if self.finger == Some(id) => {
                if self.active {
                    self.swallow = true;
                }
                self.finger = None;
                self.active = false;
                None
            }
            _ => None,
        }
    }

    /// True for the gesture a slide produces, so it does not also turn a page.
    pub fn swallows(&mut self, ge: &GestureEvent) -> bool {
        let finger_gesture = matches!(
            ge,
            GestureEvent::Tap(..)
                | GestureEvent::Swipe { .. }
                | GestureEvent::SlantedSwipe { .. }
                | GestureEvent::Arrow { .. }
                | GestureEvent::Corner { .. }
                | GestureEvent::HoldFingerShort(..)
                | GestureEvent::HoldFingerLong(..)
        );
        if !finger_gesture || !(self.active || self.swallow) {
            return false;
        }
        if !self.active {
            self.swallow = false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Dir;

    fn screen() -> Rectangle {
        rect![0, 0, 1000, 2000]
    }

    #[test]
    fn vertical_slide_changes_intensity_and_clamps() {
        let mut s = EdgeSlide::default();
        s.feed(0, FingerStatus::Down, pt!(20, 1000), screen(), 50.0);
        let mut last = None;
        for y in (200..1000).step_by(50).rev() {
            if let Some(v) = s.feed(0, FingerStatus::Motion, pt!(22, y), screen(), 50.0) {
                last = Some(v);
            }
        }
        assert_eq!(last, Some(100.0));
        let mid = s.feed(0, FingerStatus::Motion, pt!(22, 1100), screen(), 50.0).unwrap();
        assert!(mid < 50.0 && mid > 0.0);
        assert_eq!(s.feed(0, FingerStatus::Motion, pt!(22, 1900), screen(), 50.0), Some(0.0));
    }

    #[test]
    fn horizontal_motion_cancels() {
        let mut s = EdgeSlide::default();
        s.feed(0, FingerStatus::Down, pt!(20, 1000), screen(), 50.0);
        assert_eq!(s.feed(0, FingerStatus::Motion, pt!(90, 1010), screen(), 50.0), None);
        assert_eq!(s.feed(0, FingerStatus::Motion, pt!(90, 500), screen(), 50.0), None);
    }

    #[test]
    fn slide_outside_the_strip_does_nothing() {
        let mut s = EdgeSlide::default();
        s.feed(0, FingerStatus::Down, pt!(500, 1000), screen(), 50.0);
        assert_eq!(s.feed(0, FingerStatus::Motion, pt!(500, 200), screen(), 50.0), None);
    }

    #[test]
    fn the_gesture_after_a_slide_is_swallowed_once() {
        let mut s = EdgeSlide::default();
        s.feed(0, FingerStatus::Down, pt!(20, 1000), screen(), 50.0);
        s.feed(0, FingerStatus::Motion, pt!(20, 600), screen(), 50.0);
        s.feed(0, FingerStatus::Up, pt!(20, 600), screen(), 50.0);
        let swipe = GestureEvent::Swipe { dir: Dir::North, start: pt!(20, 1000), end: pt!(20, 600) };
        assert!(s.swallows(&swipe));
        assert!(!s.swallows(&swipe));
    }

    #[test]
    fn a_plain_tap_is_not_swallowed() {
        let mut s = EdgeSlide::default();
        s.feed(0, FingerStatus::Down, pt!(20, 1000), screen(), 50.0);
        s.feed(0, FingerStatus::Up, pt!(20, 1000), screen(), 50.0);
        assert!(!s.swallows(&GestureEvent::Tap(pt!(20, 1000))));
    }
}
