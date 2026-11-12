//! Formula detection helpers and on-device typesetting with Typst + MiTeX.
//!
//! Formulas are found in the sanitizer, their LaTeX source recovered, and each one is
//! rendered by `render` into a grayscale PNG that is cached like any downloaded image.

use markup5ever_rcdom::{Handle, NodeData};
use std::io::Cursor;
use std::sync::{Mutex, OnceLock};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// Prefix of the pseudo-URL under which a formula is stored as an asset.
pub const MATH_SCHEME: &str = "inoink-math:";
/// Characters of TeX accepted per formula.
pub const MAX_TEX: usize = 4000;
/// Formula text size; 1 em of the output.
const TEXT_PT: f64 = 10.0;
const PX_PER_PT: f64 = 8.0;
const MAX_PIXELS: u64 = 8_000_000;
const MAX_CONVERTED: usize = 50_000;
pub(crate) const MATH_FONT: &[u8] = include_bytes!("../../assets/math/NewCMMath-Regular.otf");
const MITEX_MOD: &str = include_str!("../../assets/math/mitex/specs/mod.typ");
const MITEX_PRELUDE: &str = include_str!("../../assets/math/mitex/specs/prelude.typ");
const MITEX_STANDARD: &str = include_str!("../../assets/math/mitex/specs/latex/standard.typ");

const MAIN_TYP: &str = r#"#import "/mitex/specs/mod.typ": mitex-scope
#set page(width: auto, height: auto, margin: 0pt, fill: white)
#set text(size: 10pt, font: "New Computer Modern Math", top-edge: "bounds", bottom-edge: "bounds")
#math.equation(block: read("/mode.txt") == "d", eval("$" + read("/formula.typ") + "$", scope: mitex-scope))
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathMode {
    Inline,
    Display,
}

pub fn math_url(tex: &str, mode: MathMode) -> String {
    format!("{MATH_SCHEME}{}:{tex}", if mode == MathMode::Display { 'd' } else { 'i' })
}

pub fn is_math_url(url: &str) -> bool {
    url.starts_with(MATH_SCHEME)
}

pub fn parse_math_url(url: &str) -> Option<(MathMode, &str)> {
    let rest = url.strip_prefix(MATH_SCHEME)?;
    if let Some(t) = rest.strip_prefix("i:") {
        Some((MathMode::Inline, t))
    } else {
        rest.strip_prefix("d:").map(|t| (MathMode::Display, t))
    }
}

#[derive(Debug)]
pub struct RenderedMath {
    pub png: Vec<u8>,
    pub width_em: f32,
    pub depth_em: f32,
}

struct Engine {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
}

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let fonts: Vec<Font> = Font::iter(Bytes::new(MATH_FONT)).collect();
        let book = LazyHash::new(FontBook::from_fonts(&fonts));
        let main = Source::new(
            FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new("main.typ").expect("valid path"))),
            MAIN_TYP.to_string(),
        );
        Engine { library: LazyHash::new(Library::default()), book, fonts, main }
    })
}

struct MathWorld<'a> {
    engine: &'a Engine,
    formula: String,
    display: bool,
}

impl World for MathWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.engine.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.engine.book
    }
    fn main(&self) -> FileId {
        self.engine.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.engine.main.id() {
            return Ok(self.engine.main.clone());
        }
        let bytes = self.file(id)?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Source::new(id, text))
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let p = id.get();
        let path = p.vpath().get_without_slash().to_string();
        if !matches!(p.root(), VirtualRoot::Project) {
            return Err(FileError::NotFound(path.into()));
        }
        match path.as_str() {
            "formula.typ" => Ok(Bytes::from_string(self.formula.clone())),
            "mode.txt" => Ok(Bytes::from_string(if self.display { "d" } else { "i" }.to_string())),
            "mitex/specs/mod.typ" => Ok(Bytes::from_string(MITEX_MOD.to_string())),
            "mitex/specs/prelude.typ" => Ok(Bytes::from_string(MITEX_PRELUDE.to_string())),
            "mitex/specs/latex/standard.typ" => Ok(Bytes::from_string(MITEX_STANDARD.to_string())),
            _ => Err(FileError::NotFound(path.into())),
        }
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.engine.fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Typesets `tex` into a grayscale PNG. The error strings are short and only logged.
pub fn render(tex: &str, mode: MathMode) -> std::result::Result<RenderedMath, String> {
    let t = tex.trim();
    if t.is_empty() || t.chars().count() > MAX_TEX {
        return Err("formula is empty or too long".into());
    }
    let conv = mitex::convert_math(t, None).map_err(|e| format!("not understood: {e}"))?;
    if conv.len() > MAX_CONVERTED {
        return Err("formula is too long".into());
    }
    static RENDER_LOCK: Mutex<()> = Mutex::new(());
    let _guard = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let world = MathWorld { engine: engine(), formula: conv, display: mode == MathMode::Display };
    let result = render_world(&world, mode);
    typst::comemo::evict(10);
    result
}

fn render_world(world: &MathWorld<'_>, mode: MathMode) -> std::result::Result<RenderedMath, String> {
    let doc = typst::compile::<typst_layout::PagedDocument>(world)
        .output
        .map_err(|errs| format!("could not typeset: {}", errs.first().map(|d| d.message.to_string()).unwrap_or_default()))?;
    let page = doc.pages().first().ok_or_else(|| "nothing to typeset".to_string())?;
    let size = page.frame.size();
    let (w_pt, h_pt) = (size.x.to_pt(), size.y.to_pt());
    if (w_pt * PX_PER_PT).ceil() * (h_pt * PX_PER_PT).ceil() > MAX_PIXELS as f64 {
        return Err("formula is too large".into());
    }
    let pm = typst_render::render(page, &typst_render::RenderOptions { pixel_per_pt: PX_PER_PT.into(), render_bleed: false });
    let (w, h) = (pm.width(), pm.height());
    // The page is filled white, so the pixmap is opaque and premultiplied equals straight.
    let luma: Vec<u8> =
        pm.pixels().iter().map(|p| ((p.red() as u32 * 299 + p.green() as u32 * 587 + p.blue() as u32 * 114) / 1000) as u8).collect();
    let img = image::GrayImage::from_raw(w, h, luma).ok_or_else(|| "bad pixmap".to_string())?;
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("png: {e}"))?;

    let depth_pt = if mode == MathMode::Inline {
        page.frame
            .items()
            .find_map(|(pos, item)| match item {
                typst::layout::FrameItem::Group(g) if g.frame.has_baseline() => {
                    Some((h_pt - (pos.y.to_pt() + g.frame.baseline().to_pt())).max(0.0))
                }
                _ => None,
            })
            .unwrap_or(0.0)
    } else {
        0.0
    };
    Ok(RenderedMath { png, width_em: (w_pt / TEXT_PT) as f32, depth_em: (depth_pt / TEXT_PT) as f32 })
}

#[derive(Debug, PartialEq, Eq)]
pub enum TexSegment<'a> {
    Text(&'a str),
    Math(&'a str, MathMode),
}

/// Splits text into plain runs and TeX formulas (`\( \)`, `\[ \]`, `$$ $$` and careful `$ $`).
pub fn split_tex(text: &str) -> Vec<TexSegment<'_>> {
    let b = text.as_bytes();
    let mut out: Vec<TexSegment<'_>> = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i < b.len() {
        let found: Option<(usize, usize, usize, MathMode)> = match b[i] {
            b'\\' if i + 1 < b.len() => match b[i + 1] {
                b'$' => {
                    i += 2;
                    continue;
                }
                b'(' => find_close(text, i + 2, "\\)").map(|c| (i + 2, c, c + 2, MathMode::Inline)),
                b'[' => find_close(text, i + 2, "\\]").map(|c| (i + 2, c, c + 2, MathMode::Display)),
                _ => {
                    i += 2;
                    continue;
                }
            },
            b'$' if b.get(i + 1) == Some(&b'$') => find_close(text, i + 2, "$$").map(|c| (i + 2, c, c + 2, MathMode::Display)),
            b'$' => single_dollar(text, i).map(|c| (i + 1, c, c + 1, MathMode::Inline)),
            _ => None,
        };
        match found {
            Some((cs, ce, end, mode)) if valid_content(&text[cs..ce]) => {
                if text_start < i {
                    out.push(TexSegment::Text(&text[text_start..i]));
                }
                out.push(TexSegment::Math(text[cs..ce].trim(), mode));
                i = end;
                text_start = end;
            }
            _ => {
                // Skip the whole opener so "$$" is not retried as a single dollar.
                i += if b[i] == b'$' && b.get(i + 1) == Some(&b'$') { 2 } else { 1 };
            }
        }
    }
    if text_start < text.len() {
        out.push(TexSegment::Text(&text[text_start..]));
    }
    out
}

fn valid_content(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_TEX
}

fn find_close(text: &str, from: usize, close: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = from;
    while i < b.len() {
        if b[i] == b'\\' && !close.starts_with('\\') {
            i += 2;
            continue;
        }
        if text[i..].starts_with(close) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The closing `$` for a single-dollar formula opened at `open`, by Pandoc's rule plus a TeX test.
fn single_dollar(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let next = text[open + 1..].chars().next()?;
    if next.is_whitespace() {
        return None;
    }
    let mut i = open + 1;
    let close = loop {
        if i >= b.len() {
            return None;
        }
        match b[i] {
            b'\\' => i += 2,
            b'$' => break i,
            _ => i += 1,
        }
    };
    let content = &text[open + 1..close];
    let last = content.chars().next_back()?;
    if last.is_whitespace() || b.get(close + 1).is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let texish = content.chars().any(|c| matches!(c, '\\' | '^' | '_' | '{' | '}' | '='))
        || (content.len() <= 2 && content.chars().all(|c| c.is_ascii_alphabetic()));
    texish.then_some(close)
}

const MAX_MATHML_DEPTH: usize = 32;

fn local(node: &Handle) -> Option<String> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.to_string()),
        _ => None,
    }
}

fn attr(node: &Handle, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        return attrs.borrow().iter().find(|a| &*a.name.local == key).map(|a| a.value.to_string());
    }
    None
}

fn all_text(node: &Handle, out: &mut String, depth: usize) {
    if depth > MAX_MATHML_DEPTH {
        return;
    }
    match &node.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        _ => {
            for c in node.children.borrow().iter() {
                all_text(c, out, depth + 1);
            }
        }
    }
}

fn elem_children(node: &Handle) -> Vec<Handle> {
    node.children.borrow().iter().filter(|c| matches!(c.data, NodeData::Element { .. })).cloned().collect()
}

fn escape_tex(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        match c {
            '\\' => out.push_str("\\backslash "),
            '{' | '}' | '#' | '%' | '&' | '$' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '^' => out.push_str("\\^{}"),
            c => out.push(c),
        }
    }
    if space && !out.is_empty() {
        out.push(' ');
    }
    out
}

fn operator(s: &str) -> String {
    let t = s.trim();
    let mapped = match t {
        "\u{2211}" => "\\sum ",
        "\u{220f}" => "\\prod ",
        "\u{222b}" => "\\int ",
        "\u{2192}" => "\\to ",
        "\u{2264}" => "\\leq ",
        "\u{2265}" => "\\geq ",
        "\u{2260}" => "\\neq ",
        "\u{b1}" => "\\pm ",
        "\u{d7}" => "\\times ",
        "\u{22c5}" => "\\cdot ",
        "\u{221e}" => "\\infty ",
        "\u{2208}" => "\\in ",
        "\u{2212}" => "- ",
        "\u{2061}" | "\u{2062}" | "\u{2063}" => "",
        _ => return escape_tex(t),
    };
    mapped.to_string()
}

fn accent(s: &str) -> Option<&'static str> {
    Some(match s.trim() {
        "^" | "\u{2c6}" | "\u{302}" => "\\hat",
        "~" | "\u{2dc}" | "\u{303}" => "\\tilde",
        "\u{af}" | "\u{203e}" | "\u{304}" => "\\bar",
        "\u{2192}" | "\u{20d7}" => "\\vec",
        "\u{2d9}" | "\u{307}" => "\\dot",
        "\u{a8}" | "\u{308}" => "\\ddot",
        _ => return None,
    })
}

/// Converts a MathML subtree to TeX for a defined subset; `None` for anything else.
pub fn mathml_to_tex(node: &Handle) -> Option<String> {
    let mut out = String::new();
    conv(node, &mut out, 0)?;
    (out.chars().count() <= MAX_TEX).then_some(out)
}

fn conv_str(node: &Handle, depth: usize) -> Option<String> {
    let mut s = String::new();
    conv(node, &mut s, depth)?;
    Some(s)
}

fn conv_children(node: &Handle, out: &mut String, depth: usize) -> Option<()> {
    for c in node.children.borrow().iter() {
        match &c.data {
            NodeData::Text { contents } => {
                if !contents.borrow().trim().is_empty() {
                    return None;
                }
            }
            NodeData::Element { .. } => conv(c, out, depth + 1)?,
            _ => {}
        }
    }
    Some(())
}

fn nth(kids: &[Handle], i: usize, depth: usize) -> Option<String> {
    conv_str(kids.get(i)?, depth + 1)
}

fn conv(node: &Handle, out: &mut String, depth: usize) -> Option<()> {
    if depth > MAX_MATHML_DEPTH || out.len() > MAX_TEX * 4 {
        return None;
    }
    let name = local(node)?;
    let kids = elem_children(node);
    let mut text = String::new();
    match name.as_str() {
        "math" | "mrow" | "mstyle" | "mpadded" | "menclose" | "merror" => conv_children(node, out, depth)?,
        "semantics" => conv(kids.first()?, out, depth + 1)?,
        "annotation" | "annotation-xml" | "mphantom" | "mspace" | "maligngroup" | "malignmark" | "none" => {}
        "mi" => {
            all_text(node, &mut text, 0);
            let t = text.trim();
            let variant = attr(node, "mathvariant").unwrap_or_default();
            if t.chars().count() == 1 && (variant.is_empty() || variant == "italic") {
                out.push_str(&escape_tex(t));
            } else {
                let cmd = match variant.as_str() {
                    "bold" => "\\mathbf",
                    "double-struck" => "\\mathbb",
                    "script" => "\\mathcal",
                    "fraktur" => "\\mathfrak",
                    _ => "\\mathrm",
                };
                out.push_str(&format!("{cmd}{{{}}}", escape_tex(t)));
            }
        }
        "mn" => {
            all_text(node, &mut text, 0);
            out.push_str(&escape_tex(text.trim()));
        }
        "mo" => {
            all_text(node, &mut text, 0);
            out.push_str(&operator(&text));
        }
        "mtext" | "ms" => {
            all_text(node, &mut text, 0);
            out.push_str(&format!("\\text{{{}}}", escape_tex(&text)));
        }
        "msup" => out.push_str(&format!("{{{}}}^{{{}}}", nth(&kids, 0, depth)?, nth(&kids, 1, depth)?)),
        "msub" => out.push_str(&format!("{{{}}}_{{{}}}", nth(&kids, 0, depth)?, nth(&kids, 1, depth)?)),
        "msubsup" => out.push_str(&format!("{{{}}}_{{{}}}^{{{}}}", nth(&kids, 0, depth)?, nth(&kids, 1, depth)?, nth(&kids, 2, depth)?)),
        "mfrac" => out.push_str(&format!("\\frac{{{}}}{{{}}}", nth(&kids, 0, depth)?, nth(&kids, 1, depth)?)),
        "msqrt" => {
            let mut inner = String::new();
            conv_children(node, &mut inner, depth)?;
            out.push_str(&format!("\\sqrt{{{inner}}}"));
        }
        "mroot" => out.push_str(&format!("\\sqrt[{}]{{{}}}", nth(&kids, 1, depth)?, nth(&kids, 0, depth)?)),
        "mover" => {
            let base = nth(&kids, 0, depth)?;
            let over = kids.get(1)?;
            let acc = if local(over).as_deref() == Some("mo") {
                let mut t = String::new();
                all_text(over, &mut t, 0);
                accent(&t)
            } else {
                None
            };
            match acc {
                Some(cmd) => out.push_str(&format!("{cmd}{{{base}}}")),
                None => out.push_str(&format!("\\overset{{{}}}{{{base}}}", conv_str(over, depth + 1)?)),
            }
        }
        "munder" => out.push_str(&format!("\\underset{{{}}}{{{}}}", nth(&kids, 1, depth)?, nth(&kids, 0, depth)?)),
        "munderover" => out.push_str(&format!("{{{}}}_{{{}}}^{{{}}}", nth(&kids, 0, depth)?, nth(&kids, 1, depth)?, nth(&kids, 2, depth)?)),
        "mtable" => {
            let mut rows = Vec::new();
            for r in &kids {
                let rname = local(r)?;
                if rname != "mtr" && rname != "mlabeledtr" {
                    return None;
                }
                let mut cells = Vec::new();
                for (i, c) in elem_children(r).iter().enumerate() {
                    if rname == "mlabeledtr" && i == 0 {
                        continue;
                    }
                    if local(c).as_deref() != Some("mtd") {
                        return None;
                    }
                    let mut s = String::new();
                    conv_children(c, &mut s, depth + 1)?;
                    cells.push(s);
                }
                rows.push(cells.join(" & "));
            }
            out.push_str(&format!("\\begin{{matrix}}{}\\end{{matrix}}", rows.join(" \\\\ ")));
        }
        "mfenced" => {
            let open = attr(node, "open").unwrap_or_else(|| "(".into());
            let close = attr(node, "close").unwrap_or_else(|| ")".into());
            let sep = attr(node, "separators").and_then(|s| s.chars().find(|c| !c.is_whitespace())).unwrap_or(',');
            let mut parts = Vec::new();
            for k in &kids {
                parts.push(conv_str(k, depth + 1)?);
            }
            out.push_str(&format!("\\left{}{}\\right{}", delim(&open), parts.join(&sep.to_string()), delim(&close)));
        }
        _ => return None,
    }
    Some(())
}

fn delim(s: &str) -> String {
    match s.trim() {
        "{" => "\\{".into(),
        "}" => "\\}".into(),
        "" => ".".into(),
        t => escape_tex(t),
    }
}

/// The LaTeX source of a `<math>` element: its TeX annotation, its `alttext`, or a conversion.
pub fn tex_source(math: &Handle) -> Option<String> {
    fn find_annotation(node: &Handle, depth: usize) -> Option<String> {
        if depth > MAX_MATHML_DEPTH {
            return None;
        }
        if local(node).as_deref() == Some("annotation") {
            let enc = attr(node, "encoding").unwrap_or_default().trim().to_ascii_lowercase();
            if matches!(enc.as_str(), "application/x-tex" | "tex" | "application/x-latex") {
                let mut t = String::new();
                all_text(node, &mut t, 0);
                return Some(t);
            }
        }
        node.children.borrow().iter().find_map(|c| find_annotation(c, depth + 1))
    }
    if let Some(t) = find_annotation(math, 0).filter(|t| !t.trim().is_empty()) {
        return Some(t);
    }
    if let Some(t) = attr(math, "alttext").filter(|t| !t.trim().is_empty()) {
        return Some(t);
    }
    mathml_to_tex(math)
}

#[cfg(test)]
mod tests {
    use super::*;
    use TexSegment::{Math, Text};

    #[test]
    fn math_urls_round_trip() {
        let u = math_url("x^2", MathMode::Inline);
        assert_eq!(u, "inoink-math:i:x^2");
        assert!(is_math_url(&u));
        assert_eq!(parse_math_url(&u), Some((MathMode::Inline, "x^2")));
        assert_eq!(parse_math_url("inoink-math:d:a:b"), Some((MathMode::Display, "a:b")));
        assert_eq!(parse_math_url("https://x"), None);
    }

    #[test]
    fn split_tex_finds_formulas_but_not_prices() {
        assert_eq!(split_tex(r"a \(x\) b"), vec![Text("a "), Math("x", MathMode::Inline), Text(" b")]);
        assert_eq!(split_tex("$$x$$"), vec![Math("x", MathMode::Display)]);
        assert_eq!(split_tex(r"\[y\]"), vec![Math("y", MathMode::Display)]);
        assert_eq!(split_tex("$x$"), vec![Math("x", MathMode::Inline)]);
        assert_eq!(split_tex("$a^2+b^2$"), vec![Math("a^2+b^2", MathMode::Inline)]);
        for t in ["$5 and $10", "costs $5.", r"\$x\$", "$ x $", r"\(x"] {
            assert_eq!(split_tex(t), vec![Text(t)], "{t}");
        }
    }

    fn find(node: &Handle, name: &str) -> Option<Handle> {
        if local(node).as_deref() == Some(name) {
            return Some(node.clone());
        }
        node.children.borrow().iter().find_map(|c| find(c, name))
    }

    #[test]
    fn mathml_subset_converts() {
        let conv = |html: &str| {
            let dom = crate::content::sanitize::parse(html);
            let m = find(&dom.document, "math").expect("math element");
            mathml_to_tex(&m)
        };
        assert_eq!(conv("<math><msup><mi>x</mi><mn>2</mn></msup></math>").as_deref(), Some("{x}^{2}"));
        assert_eq!(conv("<math><mfrac><mi>a</mi><mi>b</mi></mfrac></math>").as_deref(), Some("\\frac{a}{b}"));
        assert_eq!(conv("<math><mi>sin</mi></math>").as_deref(), Some("\\mathrm{sin}"));
        assert_eq!(conv("<math><mmultiscripts/></math>"), None);
    }

    #[test]
    fn renders_inline_and_display() {
        let m = render("x^2+y^2", MathMode::Inline).unwrap();
        assert!(m.width_em > 1.0, "{}", m.width_em);
        assert!(m.depth_em > 0.0 && m.depth_em < 0.5, "{}", m.depth_em);
        let img = image::load_from_memory(&m.png).unwrap();
        let expect = (m.width_em * 80.0).round() as i64;
        assert!((img.width() as i64 - expect).abs() <= 2, "{} vs {expect}", img.width());
        let d = render(r"\frac{a}{b}", MathMode::Display).unwrap();
        assert_eq!(d.depth_em, 0.0);
    }

    #[test]
    fn bad_formulas_are_errors() {
        assert!(render(r"\mymacro{x}", MathMode::Inline).unwrap_err().starts_with("not understood"));
        assert!(render("", MathMode::Inline).is_err());
        assert!(render(&"x".repeat(5000), MathMode::Inline).is_err());
    }
}
