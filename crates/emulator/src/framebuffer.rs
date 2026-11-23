use std::cell::Cell;
use std::rc::Rc;

use plato_core::anyhow::{format_err, Error};
use plato_core::chrono::Local;
use plato_core::color::Color;
use plato_core::framebuffer::{Framebuffer, Pixmap, UpdateMode};
use plato_core::geom::{Point, Rectangle};
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::WindowCanvas;

use crate::DEFAULT_ROTATION;

/// Physical window dimensions and full-resolution logical framebuffer dimensions.
pub(crate) type SharedGeometry = Rc<Cell<((u32, u32), (u32, u32))>>;

pub(crate) fn window_size(width: u32, height: u32, scale: f64) -> (u32, u32) {
    (
        (f64::from(width) * scale).round() as u32,
        (f64::from(height) * scale).round() as u32,
    )
}

pub(crate) fn window_to_device(x: i32, y: i32, window: (u32, u32), logical: (u32, u32)) -> Point {
    let coordinate = |value: i32, physical: u32, device: u32| {
        (f64::from(value) * f64::from(device) / f64::from(physical))
            .floor()
            .clamp(0.0, f64::from(device.saturating_sub(1))) as i32
    };
    Point {
        x: coordinate(x, window.0, logical.0),
        y: coordinate(y, window.1, logical.1),
    }
}

// Average each destination pixel over its exact source footprint. Coordinates are
// integer numerators in units of the destination dimension.
fn downsample_rgb(source: &Pixmap, width: u32, height: u32) -> Pixmap {
    assert!(source.samples == 3 && width > 0 && height > 0);
    assert!(width <= source.width && height <= source.height);
    let mut result = Pixmap::new(width, height, 3);
    let (sw, sh, dw, dh) = (
        source.width as u64,
        source.height as u64,
        width as u64,
        height as u64,
    );
    let denominator = sw * sh;
    for y in 0..dh {
        let (top, bottom) = (y * sh, (y + 1) * sh);
        for x in 0..dw {
            let (left, right) = (x * sw, (x + 1) * sw);
            let mut sums = [0u64; 3];
            for sy in top / dh..(bottom + dh - 1) / dh {
                let wy = bottom.min((sy + 1) * dh) - top.max(sy * dh);
                for sx in left / dw..(right + dw - 1) / dw {
                    let wx = right.min((sx + 1) * dw) - left.max(sx * dw);
                    let index = ((sy * sw + sx) * 3) as usize;
                    for (channel, sum) in sums.iter_mut().enumerate() {
                        *sum += u64::from(source.data()[index + channel]) * wx * wy;
                    }
                }
            }
            result.set_pixel(
                x as u32,
                y as u32,
                Color::Rgb(
                    ((sums[0] + denominator / 2) / denominator) as u8,
                    ((sums[1] + denominator / 2) / denominator) as u8,
                    ((sums[2] + denominator / 2) / denominator) as u8,
                ),
            );
        }
    }
    result
}

pub(crate) struct FBCanvas {
    canvas: WindowCanvas,
    pixmap: Pixmap,
    scale: f64,
    rotation: i8,
    geometry: SharedGeometry,
}

impl FBCanvas {
    pub(crate) fn new(
        canvas: WindowCanvas,
        width: u32,
        height: u32,
        scale: f64,
        geometry: SharedGeometry,
    ) -> Self {
        geometry.set((canvas.window().size(), (width, height)));
        Self {
            canvas,
            pixmap: Pixmap::new(width, height, 3),
            scale,
            rotation: DEFAULT_ROTATION,
            geometry,
        }
    }
}

impl Framebuffer for FBCanvas {
    fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        self.pixmap.set_pixel(x, y, color);
    }

    fn set_blended_pixel(&mut self, x: u32, y: u32, color: Color, alpha: f32) {
        self.pixmap.set_blended_pixel(x, y, color, alpha);
    }

    fn invert_region(&mut self, rect: &Rectangle) {
        self.pixmap.invert_region(rect);
    }

    fn tint_region(&mut self, rect: &Rectangle, tint: [u8; 3]) {
        self.pixmap.tint_region(rect, tint);
    }

    fn shift_region(&mut self, rect: &Rectangle, drift: u8) {
        self.pixmap.shift_region(rect, drift);
    }

    fn update(&mut self, _rect: &Rectangle, _mode: UpdateMode) -> Result<u32, Error> {
        let (output_width, output_height) = self
            .canvas
            .output_size()
            .map_err(|e| format_err!("can't get emulator output size: {}", e))?;
        if output_width == 0 || output_height == 0 {
            return Err(format_err!("emulator output size must be nonzero"));
        }
        let (width, height) = (
            output_width.min(self.width()),
            output_height.min(self.height()),
        );
        let reduced = if (width, height) != self.dims() {
            Some(downsample_rgb(&self.pixmap, width, height))
        } else {
            None
        };
        let upload = reduced.as_ref().unwrap_or(&self.pixmap);
        let creator = self.canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormatEnum::RGB24, width, height)
            .map_err(|e| format_err!("can't create framebuffer texture: {}", e))?;
        // SDL's safe wrappers panic on these errors; use the fallible C calls instead.
        // SAFETY: the texture is a live, owned SDL object on this thread.
        let result = unsafe {
            sdl2::sys::SDL_SetTextureScaleMode(
                texture.raw(),
                sdl2::sys::SDL_ScaleMode::SDL_ScaleModeNearest,
            )
        };
        if result != 0 {
            return Err(format_err!(
                "can't set nearest-neighbor scaling: {}",
                sdl2::get_error()
            ));
        }
        // The final copy only enlarges (or preserves) the coverage-averaged image.
        texture
            .update(None, upload.data(), width as usize * 3)
            .map_err(|e| format_err!("can't upload framebuffer texture: {}", e))?;
        // SAFETY: the canvas owns a valid renderer for the duration of this call.
        if unsafe { sdl2::sys::SDL_RenderClear(self.canvas.raw()) } != 0 {
            return Err(format_err!(
                "can't clear emulator window: {}",
                sdl2::get_error()
            ));
        }
        self.canvas
            .copy(&texture, None, None)
            .map_err(|e| format_err!("can't copy framebuffer texture: {}", e))?;
        self.canvas.present();
        Ok(Local::now().timestamp_subsec_millis())
    }

    fn wait(&self, _token: u32) -> Result<i32, Error> {
        Ok(1)
    }

    fn save(&self, path: &str) -> Result<(), Error> {
        self.pixmap.save(path)
    }

    fn rotation(&self) -> i8 {
        self.rotation
    }

    fn set_rotation(&mut self, n: i8) -> Result<(u32, u32), Error> {
        let (mut width, mut height) = self.dims();
        if n.rem_euclid(2) != self.rotation.rem_euclid(2) {
            std::mem::swap(&mut width, &mut height);
        }
        let (physical_width, physical_height) = window_size(width, height, self.scale);
        self.canvas
            .window_mut()
            .set_size(physical_width, physical_height)
            .map_err(|e| format_err!("can't resize emulator window: {}", e))?;
        if (width, height) != self.dims() {
            self.pixmap = Pixmap::new(width, height, 3);
        }
        self.rotation = n;
        self.geometry
            .set((self.canvas.window().size(), (width, height)));
        Ok((width, height))
    }

    fn set_monochrome(&mut self, _enable: bool) {}

    fn set_dithered(&mut self, _enable: bool) {}

    fn set_inverted(&mut self, _enable: bool) {}

    fn monochrome(&self) -> bool {
        false
    }

    fn dithered(&self) -> bool {
        false
    }

    fn inverted(&self) -> bool {
        false
    }

    fn width(&self) -> u32 {
        self.pixmap.width
    }

    fn height(&self) -> u32 {
        self.pixmap.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plato_core::color::WHITE;

    #[test]
    fn area_averages_rgb_and_preserves_identity_and_solid_colors() {
        let mut source = Pixmap::new(2, 2, 3);
        source.set_pixel(0, 0, Color::Rgb(0, 1, 2));
        source.set_pixel(1, 0, Color::Rgb(1, 2, 3));
        source.set_pixel(0, 1, Color::Rgb(2, 3, 4));
        source.set_pixel(1, 1, Color::Rgb(3, 4, 5));
        assert_eq!(
            downsample_rgb(&source, 1, 1).get_pixel(0, 0),
            Color::Rgb(2, 3, 4)
        );
        assert_eq!(downsample_rgb(&source, 2, 2).data(), source.data());
        for value in [0, 255] {
            let mut solid = Pixmap::new(9, 9, 3);
            for y in 0..9 {
                for x in 0..9 {
                    solid.set_pixel(x, y, Color::Rgb(value, value, value));
                }
            }
            assert!(downsample_rgb(&solid, 4, 4)
                .data()
                .iter()
                .all(|&v| v == value));
        }
    }

    #[test]
    fn area_preserves_lines_and_rectangle_phases() {
        for phase in [0, 1] {
            for vertical in [false, true] {
                let mut source = Pixmap::new(8, 8, 3);
                for i in 0..8 {
                    let (x, y) = if vertical { (phase, i) } else { (i, phase) };
                    source.set_pixel(x, y, Color::Rgb(0, 0, 0));
                }
                let reduced = downsample_rgb(&source, 4, 4);
                for i in 0..4 {
                    let (x, y) = if vertical { (0, i) } else { (i, 0) };
                    assert_eq!(reduced.get_pixel(x, y), Color::Rgb(128, 128, 128));
                }
            }
            let mut source = Pixmap::new(8, 8, 3);
            let (lo, hi) = if phase == 0 { (0, 6) } else { (1, 7) };
            for i in lo..=hi {
                for (x, y) in [(i, lo), (i, hi), (lo, i), (hi, i)] {
                    source.set_pixel(x, y, Color::Rgb(0, 0, 0));
                }
            }
            let reduced = downsample_rgb(&source, 4, 4);
            for (x, y) in [(1, 0), (1, 3), (0, 1), (3, 1)] {
                assert_eq!(reduced.get_pixel(x, y), Color::Rgb(128, 128, 128));
            }
        }
    }

    #[test]
    fn area_weights_noninteger_and_strong_reduction() {
        let mut source = Pixmap::new(9, 9, 3);
        for y in 0..9 {
            source.set_pixel(2, y, Color::Rgb(0, 0, 0));
        }
        let reduced = downsample_rgb(&source, 4, 4);
        for y in 0..4 {
            // Source column 2 overlaps the first footprint by 1/4 pixel and
            // the second by 3/4 pixel, each footprint being 9/4 pixels wide.
            assert_eq!(reduced.get_pixel(0, y), Color::Rgb(227, 227, 227));
            assert_eq!(reduced.get_pixel(1, y), Color::Rgb(170, 170, 170));
            assert_eq!(reduced.get_pixel(2, y), Color::Rgb(255, 255, 255));
        }
        let mut strong = Pixmap::new(10, 10, 3);
        for x in 0..10 {
            strong.set_pixel(x, 3, Color::Rgb(0, 0, 0));
        }
        assert_eq!(
            downsample_rgb(&strong, 1, 1).get_pixel(0, 0),
            Color::Rgb(230, 230, 230)
        );
    }

    #[test]
    fn scaled_window_dimensions_round() {
        assert_eq!(window_size(1264, 1680, 0.5), (632, 840));
        assert_eq!(window_size(1264, 1680, 1.0), (1264, 1680));
        assert_eq!(window_size(1264, 1680, 0.4), (506, 672));
    }

    #[test]
    fn pointer_mapping_scales_floors_and_clamps() {
        let logical = (1264, 1680);
        for scale in [0.5, 1.0] {
            let window = window_size(logical.0, logical.1, scale);
            assert_eq!(
                window_to_device(0, 0, window, logical),
                Point { x: 0, y: 0 }
            );
            assert_eq!(
                window_to_device(window.0 as i32 / 2, window.1 as i32 / 2, window, logical),
                Point { x: 632, y: 840 }
            );
            assert_eq!(
                window_to_device(window.0 as i32, window.1 as i32, window, logical),
                Point { x: 1263, y: 1679 }
            );
            assert_eq!(
                window_to_device(window.0 as i32, 0, window, logical),
                Point { x: 1263, y: 0 }
            );
            assert_eq!(
                window_to_device(0, window.1 as i32, window, logical),
                Point { x: 0, y: 1679 }
            );
            assert_eq!(
                window_to_device(-10, -5, window, logical),
                Point { x: 0, y: 0 }
            );
            assert_eq!(
                window_to_device(i32::MAX, i32::MAX, window, logical),
                Point { x: 1263, y: 1679 }
            );
        }
        assert_eq!(
            window_to_device(1, 1, (506, 672), logical),
            Point { x: 2, y: 2 }
        );
        assert_eq!(
            window_to_device(631, 839, (632, 840), logical),
            Point { x: 1262, y: 1678 }
        );
        assert_eq!(
            window_to_device(1263, 1679, logical, logical),
            Point { x: 1263, y: 1679 }
        );
    }

    // Keep all SDL work in one test: SDL initialization and video are process-global.
    #[test]
    fn dummy_framebuffer_pixels_screenshots_and_rotation() -> Result<(), Error> {
        std::env::set_var("SDL_VIDEODRIVER", "dummy");
        let sdl = sdl2::init().map_err(Error::msg)?;
        let video = sdl.video().map_err(Error::msg)?;
        let window = video
            .window("Framebuffer test", 632, 840)
            .hidden()
            .build()?;
        let canvas = window.into_canvas().software().build()?;
        let geometry = Rc::new(Cell::new(((0, 0), (0, 0))));
        let mut fb = FBCanvas::new(canvas, 1264, 1680, 0.5, Rc::clone(&geometry));
        assert_eq!(geometry.get(), ((632, 840), (1264, 1680)));
        assert_eq!(fb.dims(), (1264, 1680));
        assert_eq!(fb.rotation(), DEFAULT_ROTATION);
        assert_eq!(fb.pixmap.get_pixel(0, 0), Color::Rgb(255, 255, 255));
        fb.set_pixel(0, 0, Color::Rgb(20, 40, 60));
        fb.set_blended_pixel(0, 0, Color::Rgb(100, 120, 140), 0.5);
        assert_eq!(fb.pixmap.get_pixel(0, 0), Color::Rgb(60, 80, 100));
        let pixel = Rectangle::from((1, 1));
        fb.invert_region(&pixel);
        assert_eq!(fb.pixmap.get_pixel(0, 0), Color::Rgb(195, 175, 155));
        fb.shift_region(&pixel, 170);
        assert_eq!(fb.pixmap.get_pixel(0, 0), Color::Rgb(25, 5, 0));
        fb.set_blended_pixel(1, 0, Color::Rgb(1, 2, 3), 1.0);
        fb.set_pixel(1263, 1679, Color::Rgb(7, 8, 9));
        for y in 10..14 {
            for x in 10..14 {
                fb.set_pixel(
                    x,
                    y,
                    if x % 2 == 0 {
                        Color::Rgb(11, 22, 33)
                    } else {
                        Color::Rgb(111, 122, 133)
                    },
                );
            }
        }
        for x in 20..28 {
            fb.set_pixel(x, 20, Color::Rgb(0, 0, 0));
            fb.set_pixel(x, 23, Color::Rgb(0, 0, 0));
        }
        let token = fb.update(&fb.rect(), UpdateMode::Full)?;
        assert_eq!(fb.wait(token)?, 1);
        // Verify the area averages on the actual canvas, including both line phases.
        let displayed = fb
            .canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .map_err(Error::msg)?;
        assert_eq!(displayed.len(), 632 * 840 * 3);
        let sample = &displayed[3 * (5 * 632 + 5)..3 * (5 * 632 + 6)];
        assert_eq!(sample, [61, 72, 83]);
        for y in [10, 11] {
            assert_eq!(
                &displayed[3 * (y * 632 + 10)..3 * (y * 632 + 11)],
                &[128, 128, 128]
            );
        }

        let path =
            std::env::temp_dir().join(format!("inoink-framebuffer-{}.png", std::process::id()));
        fb.save(path.to_str().unwrap())?;
        let saved = Pixmap::from_png(&path)?;
        std::fs::remove_file(path)?;
        assert_eq!(saved.dims(), (1264, 1680));
        assert_eq!(saved.samples, 3);
        assert_eq!(saved.get_pixel(0, 0), Color::Rgb(25, 5, 0));
        assert_eq!(saved.get_pixel(1263, 1679), Color::Rgb(7, 8, 9));

        assert_eq!(fb.set_rotation(3)?, (1264, 1680));
        assert_eq!(fb.rotation(), 3);
        assert_eq!(fb.pixmap.get_pixel(0, 0), Color::Rgb(25, 5, 0));
        assert_eq!(fb.set_rotation(0)?, (1680, 1264));
        assert_eq!(fb.rotation(), 0);
        assert_eq!(geometry.get(), ((840, 632), (1680, 1264)));
        assert!(fb.pixmap.data().iter().all(|&byte| byte == WHITE.gray()));
        let (window, logical) = geometry.get();
        assert_eq!(
            window_to_device(420, 316, window, logical),
            Point { x: 840, y: 632 }
        );
        assert_eq!(
            window_to_device(840, 632, window, logical),
            Point { x: 1679, y: 1263 }
        );
        fb.update(&fb.rect(), UpdateMode::Gui)?;
        assert_eq!(fb.set_rotation(2)?, (1680, 1264));
        assert_eq!(fb.rotation(), 2);
        assert_eq!(geometry.get(), ((840, 632), (1680, 1264)));
        assert_eq!(fb.set_rotation(1)?, (1264, 1680));
        assert_eq!(geometry.get(), ((632, 840), (1264, 1680)));
        // A second canvas exercises identity and enlargement without another SDL init.
        let window = video.window("Scaling test", 4, 4).hidden().build()?;
        let canvas = window.into_canvas().software().build()?;
        let mut other = FBCanvas::new(canvas, 4, 4, 1.0, Rc::new(Cell::new(((0, 0), (0, 0)))));
        other.set_pixel(1, 2, Color::Rgb(17, 39, 61));
        other.update(&other.rect(), UpdateMode::Full)?;
        let pixels = other
            .canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .map_err(Error::msg)?;
        assert_eq!(&pixels[3 * (2 * 4 + 1)..3 * (2 * 4 + 2)], &[17, 39, 61]);
        other.canvas.window_mut().set_size(8, 8)?;
        other.update(&other.rect(), UpdateMode::Full)?;
        let pixels = other
            .canvas
            .read_pixels(None, PixelFormatEnum::RGB24)
            .map_err(Error::msg)?;
        for y in 4..6 {
            for x in 2..4 {
                assert_eq!(&pixels[3 * (y * 8 + x)..3 * (y * 8 + x + 1)], &[17, 39, 61]);
            }
        }
        fb.set_inverted(true);
        fb.set_monochrome(true);
        fb.set_dithered(true);
        assert!(!fb.inverted() && !fb.monochrome() && !fb.dithered());
        Ok(())
    }
}
