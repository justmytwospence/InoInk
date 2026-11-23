//! How a highlight is drawn over the page: shading (a tint on colour screens), an underline or a
//! dotted underline. Grayscale screens draw every colour as gray.

use crate::framebuffer::Framebuffer;
use crate::geom::Rectangle;
use inoink::highlight::{HighlightColor, HighlightStyle};

/// Gray shading of a plain highlight on grayscale screens.
pub const HIGHLIGHT_DRIFT: u8 = 0x22;
/// Gray shading of a highlight with a note: darker.
pub const ANNOTATION_DRIFT: u8 = 0x44;

/// The look of one highlight.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Mark {
    pub style: HighlightStyle,
    pub color: HighlightColor,
    /// It has a note (shading is darker).
    pub noted: bool,
}

/// How new highlights look: the last style and colour chosen. Without a chosen colour, yellow on
/// colour screens and gray on others.
pub fn defaults(settings: &crate::settings::InoInkSettings, colour: bool) -> (HighlightStyle, HighlightColor) {
    let fallback = if colour { HighlightColor::Yellow } else { HighlightColor::Gray };
    (settings.highlight_style, settings.highlight_color.unwrap_or(fallback))
}

/// The thickness of an underline under words `height` pixels tall.
pub fn line_thickness(height: i32) -> i32 {
    (height / 12).max(3)
}

/// Draws `mark` over `rects` (the words and the spaces between them, in screen coordinates),
/// clipped to `clip`. `colour` says whether the screen shows colour.
pub fn draw(fb: &mut dyn Framebuffer, rects: &[Rectangle], clip: &Rectangle, mark: Mark, colour: bool) {
    match mark.style {
        HighlightStyle::Shaded => {
            for r in rects.iter().filter_map(|r| r.intersection(clip)) {
                if colour && mark.color != HighlightColor::Gray {
                    fb.tint_region(&r, mark.color.tint());
                    if mark.noted {
                        fb.shift_region(&r, HIGHLIGHT_DRIFT);
                    }
                } else {
                    fb.shift_region(&r, if mark.noted { ANNOTATION_DRIFT } else { HIGHLIGHT_DRIFT });
                }
            }
        }
        HighlightStyle::Underline | HighlightStyle::Dotted => {
            // One thickness for the whole passage, from its tallest word.
            let height = rects.iter().map(|r| r.height() as i32).max().unwrap_or(0);
            let t = line_thickness(height);
            let ink = if colour { mark.color.ink() } else { [0, 0, 0] };
            let color = crate::color::Color::Rgb(ink[0], ink[1], ink[2]);
            for r in rects {
                let line = rect![r.min.x, r.max.y - t, r.max.x, r.max.y];
                if mark.style == HighlightStyle::Underline {
                    if let Some(l) = line.intersection(clip) {
                        fb.draw_rectangle(&l, color);
                    }
                } else {
                    // Dots aligned to the page, so neighbouring words share one pattern.
                    let period = 3 * t;
                    let mut x = line.min.x - line.min.x.rem_euclid(period);
                    while x < line.max.x {
                        let dot = rect![x.max(line.min.x), line.min.y, (x + t).min(line.max.x), line.max.y];
                        if let Some(d) = dot.intersection(clip).filter(|d| d.min.x < d.max.x) {
                            fb.draw_rectangle(&d, color);
                        }
                        x += period;
                    }
                }
            }
        }
    }
}
