//! Input for InoInk: page-button presses and taps written to the same input devices the app reads,
//! as the hardware would send them.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
pub const SYN_REPORT: u16 = 0x00;
pub const ABS_MT_SLOT: u16 = 0x2f;
pub const ABS_MT_POSITION_X: u16 = 0x35;
pub const ABS_MT_POSITION_Y: u16 = 0x36;
pub const ABS_MT_TRACKING_ID: u16 = 0x39;
pub const ABS_MT_PRESSURE: u16 = 0x3a;
pub const KEY_BACKWARD: u16 = 193;
pub const KEY_FORWARD: u16 = 194;

/// The devices InoInk reads, in its order: it opens the first that exists of each list
/// (crates/plato/src/app.rs).
pub const TOUCH_INPUTS: [&str; 5] = [
    "/dev/input/by-path/platform-2-0010-event",
    "/dev/input/by-path/platform-1-0038-event",
    "/dev/input/by-path/platform-1-0010-event",
    "/dev/input/by-path/platform-0-0010-event",
    "/dev/input/event1",
];
pub const BUTTON_INPUTS: [&str; 4] = [
    "/dev/input/by-path/platform-gpio-keys-event",
    "/dev/input/by-path/platform-ntx_event0-event",
    "/dev/input/by-path/platform-mxckpd-event",
    "/dev/input/event0",
];

pub fn first_existing(list: &[&str]) -> Option<PathBuf> {
    list.iter().map(Path::new).find(|p| p.exists()).map(Path::to_path_buf)
}

/// One `struct input_event` in the machine's layout: a zero timeval (the kernel stamps the
/// event), type, code and value.
pub fn encode(kind: u16, code: u16, value: i32) -> Vec<u8> {
    let long = std::mem::size_of::<std::os::raw::c_long>();
    let mut out = vec![0u8; 2 * long];
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(&code.to_ne_bytes());
    out.extend_from_slice(&value.to_ne_bytes());
    out
}

/// How a touch panel's raw coordinates map to the screen, from InoInk's device table
/// (crates/core/src/device.rs): the axes are swapped and mirrored depending on the model and the
/// rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchMap {
    pub swap: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub width: u32,
    pub height: u32,
}

/// A model's portrait screen size, its mirroring scheme (centre, direction), the rotation that
/// is swapped and the rotation that shows portrait with the logo at the bottom. Only models
/// whose touch panels use the multi-touch protocol with tracking ids (Plato's MultiB and MultiC)
/// and whose screen is drawn in /dev/fb0 (not the Sage and the Elipsa, which draw elsewhere).
pub struct Model {
    pub name: &'static str,
    pub dims: (u32, u32),
    pub dpi: u32,
    pub mirroring: (i8, i8),
    pub swapping: i8,
    pub startup_rotation: i8,
    /// The framebuffer counts rotations the other way (Plato's transformed_rotation).
    pub reversed: bool,
}

impl Model {
    /// Plato's rotation for the framebuffer's `rotate` value.
    pub fn rotation(&self, fb_rotate: i8) -> i8 {
        if self.reversed {
            (4 - fb_rotate) % 4
        } else {
            fb_rotate
        }
    }
}

pub fn model(product: &str) -> Option<Model> {
    let m = |name, dims, dpi, mirroring, swapping, startup_rotation| {
        Some(Model { name, dims, dpi, mirroring, swapping, startup_rotation, reversed: false })
    };
    match product {
        "monza" | "monzaKobo" | "monzaTolino" => m("Libra Colour", (1264, 1680), 300, (2, 1), 1, 1),
        "spaBW" | "spaBWTPV" | "spaKoboBW" | "spaTolinoBW" => m("Clara BW", (1072, 1448), 300, (2, 1), 1, 3),
        "spaColour" | "spaKoboColour" | "spaTolinoColour" => m("Clara Colour", (1072, 1448), 300, (2, 1), 1, 3),
        "goldfinch" => m("Clara 2E", (1072, 1448), 300, (2, 1), 1, 3),
        "nova" => m("Clara HD", (1072, 1448), 300, (2, 1), 1, 3),
        "io" => m("Libra 2", (1264, 1680), 300, (3, 1), 1, 1),
        "storm" => m("Libra H2O", (1264, 1680), 300, (3, 1), 0, 0),
        "frost" => m("Forma", (1440, 1920), 300, (2, -1), 1, 1).map(|m| Model { reversed: true, ..m }),
        "condor" => m("Elipsa 2E", (1404, 1872), 227, (2, 1), 1, 1),
        _ => None,
    }
}

impl TouchMap {
    /// The map for a model at a rotation (the framebuffer's `rotate`), with the screen size in
    /// that rotation.
    pub fn new(model: &Model, rotation: i8) -> TouchMap {
        let (mxy, dir) = model.mirroring;
        let mx = (4 + mxy + dir) % 4;
        let my = (4 + mxy - dir) % 4;
        let swap = rotation % 2 == model.swapping;
        let (w, h) = model.dims;
        // Portrait at the startup rotation and the one opposite; landscape otherwise.
        let portrait = (rotation - model.startup_rotation) % 2 == 0;
        let (width, height) = if portrait { (w, h) } else { (h, w) };
        TouchMap { swap, mirror_x: mxy == rotation || mx == rotation, mirror_y: mxy == rotation || my == rotation, width, height }
    }

    /// The raw ABS_MT_POSITION_X and ABS_MT_POSITION_Y that InoInk reads as screen point (x, y):
    /// the inverse of plato_core::input::parse_device_events.
    pub fn raw(&self, x: u32, y: u32) -> (i32, i32) {
        let vx = if self.mirror_x { self.width as i32 - 1 - x as i32 } else { x as i32 };
        let vy = if self.mirror_y { self.height as i32 - 1 - y as i32 } else { y as i32 };
        if self.swap {
            (vy, vx)
        } else {
            (vx, vy)
        }
    }

    /// The screen point InoInk reads from raw coordinates (the forward transform, for the tests).
    #[cfg(test)]
    pub fn screen(&self, raw_x: i32, raw_y: i32) -> (i32, i32) {
        let (sx, sy) = if self.swap { (raw_y, raw_x) } else { (raw_x, raw_y) };
        let x = if self.mirror_x { self.width as i32 - 1 - sx } else { sx };
        let y = if self.mirror_y { self.height as i32 - 1 - sy } else { sy };
        (x, y)
    }
}

/// Where the bench sends input.
pub trait Input {
    fn key(&mut self, code: u16) -> io::Result<()>;
    fn tap(&mut self, x: u32, y: u32) -> io::Result<()>;
}

/// Writes events to the input devices. The kernel drops an absolute value equal to the last one
/// it saw, so successive taps move by one pixel and change the pressure a little.
pub struct Devices<W: Write> {
    pub touch: W,
    pub buttons: W,
    pub map: TouchMap,
    taps: i32,
}

impl<W: Write> Devices<W> {
    pub fn new(touch: W, buttons: W, map: TouchMap) -> Self {
        Devices { touch, buttons, map, taps: 0 }
    }
}

fn write_all<W: Write>(w: &mut W, events: &[(u16, u16, i32)]) -> io::Result<()> {
    let mut buf = Vec::new();
    for &(k, c, v) in events {
        buf.extend(encode(k, c, v));
    }
    w.write_all(&buf)?;
    w.flush()
}

impl<W: Write> Input for Devices<W> {
    fn key(&mut self, code: u16) -> io::Result<()> {
        write_all(&mut self.buttons, &[(EV_KEY, code, 1), (EV_SYN, SYN_REPORT, 0)])?;
        // A short press: a long one is a hold (the next article or chapter).
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_all(&mut self.buttons, &[(EV_KEY, code, 0), (EV_SYN, SYN_REPORT, 0)])
    }

    fn tap(&mut self, x: u32, y: u32) -> io::Result<()> {
        self.taps += 1;
        let n = self.taps;
        let x = (x + (n as u32 & 1)).min(self.map.width - 1);
        let (rx, ry) = self.map.raw(x, y);
        let down = [
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, 1000 + n),
            (EV_ABS, ABS_MT_POSITION_X, rx),
            (EV_ABS, ABS_MT_POSITION_Y, ry),
            (EV_ABS, ABS_MT_PRESSURE, 40 + (n & 1)),
            (EV_SYN, SYN_REPORT, 0),
        ];
        write_all(&mut self.touch, &down)?;
        std::thread::sleep(std::time::Duration::from_millis(60));
        write_all(&mut self.touch, &[(EV_ABS, ABS_MT_TRACKING_ID, -1), (EV_SYN, SYN_REPORT, 0)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libra_colour_portrait_matches_plato() {
        // rotation 1 on the Libra Colour: the axes are swapped and y is mirrored.
        let m = model("monza").unwrap();
        let map = TouchMap::new(&m, m.startup_rotation);
        assert_eq!(map, TouchMap { swap: true, mirror_x: false, mirror_y: true, width: 1264, height: 1680 });
        assert_eq!(map.raw(100, 200), (1479, 100));
    }

    #[test]
    fn raw_coordinates_round_trip_for_every_model_and_rotation() {
        for product in ["monza", "spaBW", "spaColour", "goldfinch", "nova", "io", "storm", "frost", "condor"] {
            let m = model(product).unwrap();
            for rotation in 0..4 {
                let map = TouchMap::new(&m, rotation);
                for (x, y) in [(0, 0), (10, 20), (map.width - 1, map.height - 1), (map.width / 2, 7)] {
                    let (rx, ry) = map.raw(x, y);
                    assert_eq!(map.screen(rx, ry), (x as i32, y as i32), "{product} rotation {rotation}");
                }
            }
        }
        assert!(model("trilogy").is_none() && model("cadmus").is_none());
        assert_eq!(model("frost").unwrap().rotation(1), 3);
        assert_eq!(model("monza").unwrap().rotation(1), 1);
    }

    #[test]
    fn events_are_laid_out_like_the_kernel_struct() {
        let e = encode(EV_KEY, KEY_FORWARD, 1);
        let long = std::mem::size_of::<std::os::raw::c_long>();
        assert_eq!(e.len(), 2 * long + 8);
        assert_eq!(&e[2 * long..2 * long + 2], &EV_KEY.to_ne_bytes());
        assert_eq!(&e[2 * long + 2..2 * long + 4], &KEY_FORWARD.to_ne_bytes());
        assert_eq!(&e[2 * long + 4..], &1i32.to_ne_bytes());
    }

    fn decode(bytes: &[u8]) -> Vec<(u16, u16, i32)> {
        let long = std::mem::size_of::<std::os::raw::c_long>();
        bytes
            .chunks(2 * long + 8)
            .map(|c| {
                let k = u16::from_ne_bytes([c[2 * long], c[2 * long + 1]]);
                let code = u16::from_ne_bytes([c[2 * long + 2], c[2 * long + 3]]);
                let v = i32::from_ne_bytes(c[2 * long + 4..2 * long + 8].try_into().unwrap());
                (k, code, v)
            })
            .collect()
    }

    #[test]
    fn successive_taps_never_repeat_a_value() {
        let m = model("monza").unwrap();
        let mut d = Devices::new(Vec::new(), Vec::new(), TouchMap::new(&m, 1));
        d.tap(500, 600).unwrap();
        d.tap(500, 600).unwrap();
        let ev = decode(&d.touch);
        let values = |code| ev.iter().filter(|e| e.0 == EV_ABS && e.1 == code).map(|e| e.2).collect::<Vec<_>>();
        let (xs, ys, ps) = (values(ABS_MT_POSITION_X), values(ABS_MT_POSITION_Y), values(ABS_MT_PRESSURE));
        assert_eq!(xs.len(), 2);
        assert!(xs[0] != xs[1] || ys[0] != ys[1]);
        assert_ne!(ps[0], ps[1]);
        assert_eq!(values(ABS_MT_TRACKING_ID), vec![1001, -1, 1002, -1]);
        // Both land where they were aimed, one pixel apart at most.
        let map = d.map;
        for (rx, ry) in xs.iter().zip(&ys) {
            let (x, y) = map.screen(*rx, *ry);
            assert!((500..=501).contains(&x) && y == 600);
        }
        d.key(KEY_FORWARD).unwrap();
        assert_eq!(decode(&d.buttons), vec![(EV_KEY, KEY_FORWARD, 1), (EV_SYN, 0, 0), (EV_KEY, KEY_FORWARD, 0), (EV_SYN, 0, 0)]);
    }
}
