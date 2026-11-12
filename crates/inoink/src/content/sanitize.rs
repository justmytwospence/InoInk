//! HTML5 parse + allow-list serialization to XHTML.
//!
//! Plato's HTML engine reads XML, so the output must be well-formed: void elements are
//! self-closed and only `&amp; &lt; &gt; &quot;` entities are emitted. Scripts, styles, forms,
//! frames, event handlers and every non-http(s) URL are removed.

use super::math::{self, MathMode};
use super::{asset_name, ArticleMeta, AssetMap, AssetRef, AssetState, PreparedArticle, MAX_HTML, RETRY_SCHEME};
use html5ever::tendril::TendrilSink;
use html5ever::{local_name, ns, parse_fragment, ParseOpts, QualName};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use url::Url;

const MAX_DEPTH: usize = 48;
/// Candidates wider than this are not preferred from a `srcset`.
const PREFERRED_WIDTH: u32 = 1600;

const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "embed", "form", "input", "button", "select", "textarea", "noscript",
    "template", "link", "meta", "base", "head", "title", "svg", "math", "video", "audio", "canvas", "applet", "map", "dialog", "option",
    "optgroup", "datalist", "source", "track", "param",
];

const BLOCK_TO_DIV: &[&str] = &[
    "div", "section", "article", "header", "footer", "main", "aside", "nav", "details", "summary", "address", "center", "hgroup",
    "fieldset", "figure", "picture", "legend",
];
const PASS_THROUGH: &[&str] = &[
    "p",
    "blockquote",
    "pre",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "caption",
    "em",
    "strong",
    "b",
    "i",
    "u",
    "s",
    "del",
    "ins",
    "sub",
    "sup",
    "small",
    "mark",
    "cite",
    "abbr",
    "q",
    "kbd",
    "samp",
    "var",
    "code",
    "time",
    "tt",
    "big",
];
const INLINE_RENAME: &[(&str, &str)] = &[
    ("strike", "s"),
    ("dfn", "em"),
    ("font", "span"),
    ("label", "span"),
    ("bdi", "span"),
    ("bdo", "span"),
    ("data", "span"),
    ("wbr", "span"),
    ("ruby", "span"),
    ("rt", "span"),
    ("rp", "span"),
    ("output", "span"),
];

pub(crate) fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            '\u{fffe}' | '\u{ffff}' => {}
            c => out.push(c),
        }
    }
}

/// Text-node escaping: quotes need no escaping outside attributes.
fn esc_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            '\u{fffe}' | '\u{ffff}' => {}
            c => out.push(c),
        }
    }
}

fn attr(node: &Handle, name: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        for a in attrs.borrow().iter() {
            if &*a.name.local == name {
                return Some(a.value.to_string());
            }
        }
    }
    None
}

fn tag(node: &Handle) -> Option<String> {
    if let NodeData::Element { name, .. } = &node.data {
        Some(name.local.to_string())
    } else {
        None
    }
}

/// Resolves `raw` against `base`, accepting only http and https.
pub fn resolve_url(base: Option<&Url>, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let u = match base {
        Some(b) => b.join(raw).ok()?,
        None => Url::parse(raw).ok()?,
    };
    match u.scheme() {
        "http" | "https" if u.host_str().is_some() => Some(u),
        _ => None,
    }
}

/// Picks one URL from a `srcset` value: the widest candidate not above [`PREFERRED_WIDTH`],
/// otherwise the narrowest above it. Candidates without a descriptor count as 1x (width 0).
pub fn pick_srcset(srcset: &str) -> Option<String> {
    let mut cands: Vec<(u32, String)> = Vec::new();
    // URLs may contain commas, so split on descriptor boundaries: "url 480w, url2 800w".
    for part in srcset.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut it = part.split_whitespace();
        let Some(url) = it.next() else { continue };
        let width = it
            .next()
            .and_then(|d| {
                d.strip_suffix('w')
                    .and_then(|n| n.parse::<u32>().ok())
                    .or_else(|| d.strip_suffix('x').and_then(|n| n.parse::<f32>().ok()).map(|x| (x * 800.0) as u32))
            })
            .unwrap_or(0);
        cands.push((width, url.to_string()));
    }
    let within: Vec<&(u32, String)> = cands.iter().filter(|(w, _)| *w <= PREFERRED_WIDTH).collect();
    if let Some((_, u)) = within.iter().max_by_key(|(w, _)| *w) {
        return Some(u.clone());
    }
    cands.iter().min_by_key(|(w, _)| *w).map(|(_, u)| u.clone())
}

fn is_placeholder_src(s: &str) -> bool {
    let s = s.trim();
    s.is_empty() || s.starts_with("data:") || s.ends_with("blank.gif") || s.contains("spacer.gif") || s.contains("1x1")
}

/// The image URL an `<img>` points at, considering common lazy-loading attributes.
pub fn image_source(node: &Handle) -> Option<String> {
    for key in ["data-src", "data-lazy-src", "data-original", "data-lazy", "data-hi-res-src", "data-full-src"] {
        if let Some(v) = attr(node, key) {
            if !is_placeholder_src(&v) {
                return Some(v);
            }
        }
    }
    for key in ["data-srcset", "data-lazy-srcset", "srcset"] {
        if let Some(v) = attr(node, key) {
            if let Some(u) = pick_srcset(&v) {
                if !is_placeholder_src(&u) {
                    return Some(u);
                }
            }
        }
    }
    attr(node, "src").filter(|s| !is_placeholder_src(s))
}

fn safe_id(s: &str) -> Option<String> {
    let s = s.trim();
    if !s.is_empty() && s.len() <= 80 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')) {
        Some(s.to_string())
    } else {
        None
    }
}

const SKIP_MATH_CLASSES: &[&str] = &[
    "katex-html",
    "MathJax_Preview",
    "MathJax",
    "MathJax_Display",
    "MathJax_CHTML",
    "MathJax_SVG",
    "MathJax_SVG_Display",
    "MJX_Assistive_MathML",
];

fn has_class(node: &Handle, token: &str) -> bool {
    attr(node, "class").is_some_and(|c| c.split_whitespace().any(|t| t == token))
}

fn class_starts_with(node: &Handle, prefix: &str) -> bool {
    attr(node, "class").is_some_and(|c| c.split_whitespace().any(|t| t.starts_with(prefix)))
}

fn first_descendant(node: &Handle, local_name: &str) -> Option<Handle> {
    fn go(node: &Handle, name: &str, depth: usize) -> Option<Handle> {
        if depth > MAX_DEPTH {
            return None;
        }
        for c in node.children.borrow().iter() {
            if tag(c).as_deref() == Some(name) {
                return Some(c.clone());
            }
            if let Some(f) = go(c, name, depth + 1) {
                return Some(f);
            }
        }
        None
    }
    go(node, local_name, 0)
}

fn text_content(node: &Handle) -> String {
    fn go(node: &Handle, out: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if let NodeData::Text { contents } = &node.data {
            out.push_str(&contents.borrow());
        }
        for c in node.children.borrow().iter() {
            go(c, out, depth + 1);
        }
    }
    let mut s = String::new();
    go(node, &mut s, 0);
    s
}

struct Ctx<'a> {
    base: Option<Url>,
    assets: &'a AssetMap,
    out: String,
    text: String,
    refs: Vec<AssetRef>,
    placeholders: usize,
    in_pre: bool,
    /// Depth inside `code`, `kbd`, `samp` and `tt`, where TeX delimiters are literal.
    in_code: usize,
    /// False for comment fragments: images become their alt text.
    images: bool,
}

impl Ctx<'_> {
    fn text_break(&mut self) {
        if !self.text.ends_with(' ') && !self.text.ends_with('\n') && !self.text.is_empty() {
            self.text.push(' ');
        }
    }

    fn note_asset(&mut self, url: &Url) -> String {
        let name = asset_name(url.as_str());
        if !self.refs.iter().any(|r| r.name == name) {
            self.refs.push(AssetRef { name: name.clone(), url: url.to_string() });
        }
        name
    }

    fn image(&mut self, node: &Handle) {
        let alt = attr(node, "alt").unwrap_or_default();
        let Some(src) = image_source(node).filter(|_| self.images) else {
            // No usable source: keep the alt text so nothing is silently lost.
            if !alt.trim().is_empty() {
                self.out.push_str("<em>");
                esc(alt.trim(), &mut self.out);
                self.out.push_str("</em>");
                self.text.push_str(alt.trim());
            }
            return;
        };
        let Some(url) = resolve_url(self.base.as_ref(), &src) else {
            return;
        };
        let name = self.note_asset(&url);
        match self.assets.get(&name) {
            Some(AssetState::Done(file)) => {
                self.out.push_str("<img src=\"assets/");
                esc(file, &mut self.out);
                self.out.push_str("\" alt=\"");
                esc(&alt, &mut self.out);
                self.out.push_str("\"/>");
            }
            state => {
                self.placeholders += 1;
                let label = if matches!(state, Some(AssetState::Failed)) { "Image unavailable" } else { "Image not downloaded" };
                self.out.push_str("<p class=\"inoink-missing\"><a href=\"");
                self.out.push_str(RETRY_SCHEME);
                self.out.push_str(&name);
                self.out.push_str("\">[");
                self.out.push_str(label);
                self.out.push_str(" \u{2014} retry]</a>");
                if !alt.trim().is_empty() {
                    self.out.push(' ');
                    esc(alt.trim(), &mut self.out);
                }
                self.out.push_str("</p>");
            }
        }
    }

    fn emit_text(&mut self, t: &str) {
        esc_text(t, &mut self.out);
        if self.in_pre {
            self.text.push_str(t);
        } else {
            for w in t.split_whitespace() {
                if !self.text.is_empty() && !self.text.ends_with(' ') && !self.text.ends_with('\n') {
                    self.text.push(' ');
                }
                self.text.push_str(w);
            }
        }
    }

    /// A formula: an image once it is typeset, its TeX source in the meantime.
    fn math(&mut self, tex: &str, mode: MathMode) {
        self.text_break();
        self.text.push_str(tex);
        self.text.push(' ');
        let display = mode == MathMode::Display;
        let formula = if self.images {
            let url = math::math_url(tex, mode);
            let name = asset_name(&url);
            if !self.refs.iter().any(|r| r.name == name) {
                self.refs.push(AssetRef { name: name.clone(), url });
            }
            match self.assets.get(&name) {
                Some(AssetState::Formula { file, width_em, depth_em }) => Some((file.clone(), *width_em, *depth_em)),
                _ => None,
            }
        } else {
            None
        };
        match formula {
            Some((file, w, d)) => {
                let (class, style) = if display {
                    ("inoink-math-display", format!("display:block;width:{w:.3}em"))
                } else {
                    ("inoink-math", format!("display:inline;width:{w:.3}em;vertical-align:-{d:.3}em"))
                };
                self.out.push_str(&format!("<img class=\"{class}\" src=\"assets/"));
                esc(&file, &mut self.out);
                self.out.push_str("\" alt=\"");
                esc(tex, &mut self.out);
                self.out.push_str(&format!("\" style=\"{style}\"/>"));
            }
            None => {
                if display {
                    self.out.push_str("<br/>");
                }
                self.out.push_str("<code class=\"inoink-tex\">");
                esc_text(tex, &mut self.out);
                self.out.push_str("</code>");
                if display {
                    self.out.push_str("<br/>");
                }
            }
        }
    }

    fn mathml(&mut self, node: &Handle, display: bool) {
        match math::tex_source(node).filter(|t| !t.trim().is_empty()) {
            Some(t) => self.math(t.trim(), if display { MathMode::Display } else { MathMode::Inline }),
            None => self.mathml_text(node, 0),
        }
    }

    /// Linear fallback for MathML that cannot be converted to TeX: nothing is dropped.
    fn mathml_text(&mut self, node: &Handle, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match &node.data {
            NodeData::Text { contents } => {
                let t = contents.borrow().to_string();
                self.emit_text(&t);
            }
            NodeData::Element { .. } => {
                let n = tag(node).unwrap_or_default();
                let kids: Vec<Handle> =
                    node.children.borrow().iter().filter(|c| matches!(c.data, NodeData::Element { .. })).cloned().collect();
                match n.as_str() {
                    "annotation" | "annotation-xml" => {}
                    "msup" if kids.len() == 2 => {
                        self.mathml_text(&kids[0], depth + 1);
                        self.out.push_str("<sup>");
                        self.mathml_text(&kids[1], depth + 1);
                        self.out.push_str("</sup>");
                    }
                    "msub" if kids.len() == 2 => {
                        self.mathml_text(&kids[0], depth + 1);
                        self.out.push_str("<sub>");
                        self.mathml_text(&kids[1], depth + 1);
                        self.out.push_str("</sub>");
                    }
                    "mfrac" if kids.len() == 2 => {
                        self.out.push('(');
                        self.mathml_text(&kids[0], depth + 1);
                        self.out.push_str(")/(");
                        self.mathml_text(&kids[1], depth + 1);
                        self.out.push(')');
                    }
                    "msqrt" => {
                        self.out.push_str("\u{221a}(");
                        self.children_with(node, depth, Self::mathml_text);
                        self.out.push(')');
                    }
                    _ => self.children_with(node, depth, Self::mathml_text),
                }
            }
            _ => {}
        }
    }

    fn children_with(&mut self, node: &Handle, depth: usize, f: fn(&mut Self, &Handle, usize)) {
        let kids: Vec<Handle> = node.children.borrow().iter().cloned().collect();
        for k in kids {
            f(self, &k, depth + 1);
        }
    }

    fn walk(&mut self, node: &Handle, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match &node.data {
            NodeData::Text { contents } => {
                let t = contents.borrow().to_string();
                if !self.in_pre && self.in_code == 0 {
                    let segs = math::split_tex(&t);
                    if segs.iter().any(|s| matches!(s, math::TexSegment::Math(..))) {
                        for seg in segs {
                            match seg {
                                math::TexSegment::Text(x) => self.emit_text(x),
                                math::TexSegment::Math(tex, mode) => self.math(tex, mode),
                            }
                        }
                        return;
                    }
                }
                self.emit_text(&t);
            }
            NodeData::Element { .. } => {
                let name = tag(node).unwrap_or_default();
                let n = name.as_str();
                if n == "script" && attr(node, "type").is_some_and(|t| t.trim().to_ascii_lowercase().starts_with("math/tex")) {
                    let ty = attr(node, "type").unwrap_or_default().to_ascii_lowercase();
                    let tex = text_content(node);
                    if self.images && !tex.trim().is_empty() {
                        self.math(tex.trim(), if ty.contains("mode=display") { MathMode::Display } else { MathMode::Inline });
                    }
                    return;
                }
                if n == "mjx-container" {
                    if let Some(m) = first_descendant(node, "math") {
                        self.mathml(&m, attr(node, "display").as_deref() == Some("true"));
                    }
                    return;
                }
                if has_class(node, "katex-display") {
                    if let Some(m) = first_descendant(node, "math") {
                        self.mathml(&m, true);
                    }
                    return;
                }
                if has_class(node, "katex") {
                    if let Some(m) = first_descendant(node, "math") {
                        self.mathml(&m, attr(&m, "display").as_deref() == Some("block"));
                    }
                    return;
                }
                if SKIP_MATH_CLASSES.iter().any(|c| has_class(node, c)) {
                    return;
                }
                if n == "img" && class_starts_with(node, "mwe-math-fallback-image") {
                    return;
                }
                if n == "math" {
                    self.mathml(node, attr(node, "display").as_deref() == Some("block"));
                    return;
                }
                if DROP_WITH_CONTENT.contains(&n) {
                    return;
                }
                match n {
                    "img" => self.image(node),
                    "br" => {
                        self.out.push_str("<br/>");
                        self.text.push('\n');
                    }
                    "hr" => {
                        self.out.push_str("<hr/>");
                        self.text_break();
                    }
                    "a" => {
                        let href = attr(node, "href");
                        let safe = href.as_deref().and_then(|h| {
                            let h = h.trim();
                            if let Some(frag) = h.strip_prefix('#') {
                                safe_id(frag).map(|f| format!("#{f}"))
                            } else {
                                resolve_url(self.base.as_ref(), h).map(|u| u.to_string())
                            }
                        });
                        match safe {
                            Some(h) => {
                                self.out.push_str("<a href=\"");
                                esc(&h, &mut self.out);
                                self.out.push_str("\">");
                                self.children(node, depth);
                                self.out.push_str("</a>");
                            }
                            None => self.children(node, depth),
                        }
                    }
                    "figcaption" => {
                        self.out.push_str("<p class=\"inoink-caption\">");
                        self.children(node, depth);
                        self.out.push_str("</p>");
                        self.text_break();
                    }
                    "td" | "th" => {
                        self.out.push('<');
                        self.out.push_str(n);
                        for k in ["colspan", "rowspan"] {
                            if let Some(v) = attr(node, k).and_then(|v| v.trim().parse::<u32>().ok()) {
                                if (1..=100).contains(&v) {
                                    self.out.push_str(&format!(" {k}=\"{v}\""));
                                }
                            }
                        }
                        self.out.push('>');
                        self.children(node, depth);
                        self.out.push_str("</");
                        self.out.push_str(n);
                        self.out.push('>');
                        self.text.push(' ');
                    }
                    "pre" => {
                        let prev = self.in_pre;
                        self.in_pre = true;
                        self.simple(node, "pre", depth);
                        self.in_pre = prev;
                    }
                    _ if BLOCK_TO_DIV.contains(&n) => {
                        self.simple(node, "div", depth);
                    }
                    "code" | "kbd" | "samp" | "tt" => {
                        self.in_code += 1;
                        self.simple(node, n, depth);
                        self.in_code -= 1;
                    }
                    _ if PASS_THROUGH.contains(&n) => {
                        self.simple(node, n, depth);
                    }
                    _ => {
                        if let Some((_, to)) = INLINE_RENAME.iter().find(|(from, _)| *from == n) {
                            self.simple(node, to, depth);
                        } else {
                            // Unknown element: drop the tag, keep the content.
                            self.children(node, depth);
                        }
                    }
                }
            }
            NodeData::Document => self.children(node, depth),
            _ => {}
        }
    }

    fn simple(&mut self, node: &Handle, name: &str, depth: usize) {
        let block = !matches!(
            name,
            "em" | "strong"
                | "b"
                | "i"
                | "u"
                | "s"
                | "del"
                | "ins"
                | "sub"
                | "sup"
                | "small"
                | "mark"
                | "cite"
                | "abbr"
                | "q"
                | "kbd"
                | "samp"
                | "var"
                | "code"
                | "time"
                | "tt"
                | "big"
                | "span"
        );
        if block {
            self.text_break();
        }
        self.out.push('<');
        self.out.push_str(name);
        if let Some(id) = attr(node, "id").and_then(|i| safe_id(&i)) {
            self.out.push_str(" id=\"");
            esc(&id, &mut self.out);
            self.out.push('"');
        }
        self.out.push('>');
        self.children(node, depth);
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
        if block {
            self.text_break();
        }
    }

    fn children(&mut self, node: &Handle, depth: usize) {
        let kids: Vec<Handle> = node.children.borrow().iter().cloned().collect();
        for k in kids {
            self.walk(&k, depth + 1);
        }
    }
}

/// The DOM must outlive every handle into it (dropping it clears the tree).
pub(crate) fn parse(html: &str) -> RcDom {
    let ctx = QualName::new(None, ns!(html), local_name!("div"));
    parse_fragment(RcDom::default(), ParseOpts::default(), ctx, vec![], false).one(html)
}

/// Sanitizes a fragment (a comment) without images. Returns (XHTML fragment, plain text).
pub fn sanitize_fragment(raw: &str, base_url: Option<&str>) -> (String, String) {
    let base = base_url.and_then(|b| Url::parse(b).ok());
    let raw = if raw.len() > MAX_HTML { &raw[..raw.floor_char_boundary(MAX_HTML)] } else { raw };
    let dom = parse(raw);
    let root = dom.document.children.borrow().first().cloned().unwrap_or_else(|| dom.document.clone());
    let assets = AssetMap::new();
    let mut cx = Ctx {
        base,
        assets: &assets,
        out: String::new(),
        text: String::new(),
        refs: Vec::new(),
        placeholders: 0,
        in_pre: false,
        in_code: 0,
        images: false,
    };
    cx.children(&root, 0);
    (cx.out, cx.text)
}

/// A complete XHTML document: the title as heading, a meta line, then already sanitized `body`.
pub fn document(title: &str, meta: &str, body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 512);
    html.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>");
    esc(title, &mut html);
    html.push_str("</title></head><body><h1>");
    esc(title, &mut html);
    html.push_str("</h1><p class=\"inoink-meta\">");
    esc(meta, &mut html);
    html.push_str("</p><hr/>");
    html.push_str(body);
    html.push_str("</body></html>\n");
    html
}

/// Sanitizes `raw` and wraps it in a complete XHTML document. `assets` says which images are
/// already downloaded; the rest become retryable placeholders.
pub fn prepare_article(raw: &str, base_url: Option<&str>, meta: &ArticleMeta, assets: &AssetMap) -> PreparedArticle {
    let base = base_url.and_then(|b| Url::parse(b).ok());
    let raw = if raw.len() > MAX_HTML { &raw[..raw.floor_char_boundary(MAX_HTML)] } else { raw };
    let dom = parse(raw);
    // The fragment parser wraps content in an <html> element under the document.
    let root = dom.document.children.borrow().first().cloned().unwrap_or_else(|| dom.document.clone());
    let mut cx = Ctx {
        base,
        assets,
        out: String::new(),
        text: String::new(),
        refs: Vec::new(),
        placeholders: 0,
        in_pre: false,
        in_code: 0,
        images: true,
    };
    cx.children(&root, 0);

    let mut parts: Vec<&str> = Vec::new();
    for p in [meta.source.as_str(), meta.author.as_str(), meta.date.as_str()] {
        if !p.trim().is_empty() {
            parts.push(p.trim());
        }
    }
    let mut body = cx.out;
    if let Some(label) = &meta.discussion {
        body.push_str("<p class=\"inoink-switch\"><a href=\"");
        body.push_str(super::discussion::COMMENTS_SCHEME);
        body.push_str("\">");
        esc(label, &mut body);
        body.push_str("</a></p>");
    }
    let html = document(&meta.title, &parts.join(" \u{b7} "), &body);
    PreparedArticle { html, text: cx.text.trim().to_string(), assets: cx.refs, placeholders: cx.placeholders }
}
