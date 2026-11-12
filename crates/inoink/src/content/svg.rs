//! SVG image files are rasterized with resvg, without any file or network access.

use super::images::{ImageLimits, Normalized};
use super::math::MATH_FONT;
use crate::error::{Error, Result};
use image::ImageFormat;
use std::io::Cursor;

/// True when `bytes` look like an SVG document.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let t = head.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<svg") || ((t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE")) && t.contains("<svg"))
}

/// Rasterizes an SVG to PNG without any file or network access.
pub fn rasterize(bytes: &[u8], limits: &ImageLimits) -> Result<Normalized> {
    let mut opt = resvg::usvg::Options {
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    opt.fontdb_mut().load_font_data(MATH_FONT.to_vec());
    opt.font_family = "New Computer Modern Math".into();
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).map_err(|_| Error::Unsupported("unreadable SVG image".into()))?;
    if !tree.root().has_children() {
        return Err(Error::Unsupported("empty SVG image".into()));
    }
    let (w, h) = (tree.size().width(), tree.size().height());
    if w < 1.0 || h < 1.0 {
        return Err(Error::Unsupported("empty SVG image".into()));
    }
    let scale = 2.0f32.min(limits.max_width as f32 / w).min(limits.max_height as f32 / h);
    let (pw, ph) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
    if pw == 0 || ph == 0 || u64::from(pw) * u64::from(ph) > limits.max_pixels {
        return Err(Error::Unsupported("image is too large".into()));
    }
    let mut pm = resvg::tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| Error::Unsupported("image is too large".into()))?;
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    let rgb: Vec<u8> = pm.pixels().iter().flat_map(|p| [p.red(), p.green(), p.blue()]).collect();
    let img = image::RgbImage::from_raw(pw, ph, rgb).ok_or_else(|| Error::Unsupported("image is too large".into()))?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| Error::Unsupported(format!("png: {e}")))?;
    Ok(Normalized { bytes: out, ext: "png", width: pw, height: ph })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="black"/><text x="5" y="30">Hi</text><image href="/etc/passwd" width="10" height="10"/><image href="https://evil.example/x.png" width="10" height="10"/></svg>"#;

    fn limits() -> ImageLimits {
        ImageLimits::default()
    }

    #[test]
    fn rasterizes_without_touching_files_or_network() {
        let n = rasterize(SVG.as_bytes(), &limits()).unwrap();
        assert_eq!((n.width, n.height, n.ext), (200, 100, "png"));
        let img = image::load_from_memory(&n.bytes).unwrap().to_luma8();
        assert!(img.get_pixel(100, 50)[0] < 64);
    }

    #[test]
    fn empty_or_script_only_svg_is_rejected() {
        assert!(rasterize(b"<svg xmlns='http://www.w3.org/2000/svg'><script>1</script></svg>", &limits()).is_err());
    }

    #[test]
    fn detects_svg() {
        assert!(is_svg(b"<?xml version=\"1.0\"?><svg></svg>"));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(b"<html><body></body></html>"));
    }
}
