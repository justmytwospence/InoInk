//! Just enough JSON for one flat object per line: strings, numbers and booleans.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    #[cfg(test)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// An object that keeps its keys in insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Obj(pub Vec<(String, Value)>);

impl Obj {
    pub fn new() -> Obj {
        Obj(Vec::new())
    }
    pub fn s(mut self, k: &str, v: impl Into<String>) -> Obj {
        self.0.push((k.into(), Value::Str(v.into())));
        self
    }
    pub fn n(mut self, k: &str, v: impl Into<f64>) -> Obj {
        self.0.push((k.into(), Value::Num(v.into())));
        self
    }
    pub fn b(mut self, k: &str, v: bool) -> Obj {
        self.0.push((k.into(), Value::Bool(v)));
        self
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.iter().find(|(key, _)| key == k).map(|(_, v)| v)
    }
    pub fn str(&self, k: &str) -> Option<&str> {
        self.get(k).and_then(Value::as_str)
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl std::fmt::Display for Obj {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut out = String::from("{");
        for (i, (k, v)) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            quote(k, &mut out);
            out.push(':');
            match v {
                Value::Str(s) => quote(s, &mut out),
                Value::Num(n) if n.is_finite() => {
                    if n.fract() == 0.0 && n.abs() < 1e15 {
                        out.push_str(&format!("{}", *n as i64));
                    } else {
                        out.push_str(&format!("{n}"));
                    }
                }
                Value::Num(_) | Value::Null => out.push_str("null"),
                Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            }
        }
        out.push('}');
        f.write_str(&out)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.s.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                            out.extend(ch.to_string().as_bytes());
                        }
                        _ => return None,
                    }
                }
                c => out.push(c),
            }
        }
    }
    fn value(&mut self) -> Option<Value> {
        self.ws();
        let rest = &self.s[self.i..];
        for (word, v) in [(&b"true"[..], Value::Bool(true)), (b"false", Value::Bool(false)), (b"null", Value::Null)] {
            if rest.starts_with(word) {
                self.i += word.len();
                return Some(v);
            }
        }
        if rest.first() == Some(&b'"') {
            return self.string().map(Value::Str);
        }
        let len = rest.iter().take_while(|c| c.is_ascii_digit() || b"+-.eE".contains(c)).count();
        let n = std::str::from_utf8(&rest[..len]).ok()?.parse().ok()?;
        self.i += len;
        Some(Value::Num(n))
    }
}

/// Parses one flat object; nested objects and arrays are not supported.
pub fn parse(line: &str) -> Option<Obj> {
    let mut p = Parser { s: line.as_bytes(), i: 0 };
    p.eat(b'{')?;
    let mut obj = Obj::new();
    if p.eat(b'}').is_some() {
        return Some(obj);
    }
    loop {
        let k = p.string()?;
        p.eat(b':')?;
        let v = p.value()?;
        obj.0.push((k, v));
        if p.eat(b',').is_some() {
            continue;
        }
        p.eat(b'}')?;
        p.ws();
        return (p.i == p.s.len()).then_some(obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let o = Obj::new().s("type", "sample").s("note", "a \"q\"\n\u{1}é").n("ms", 412.5).n("n", 3).b("ok", true);
        let line = o.to_string();
        assert_eq!(line, r#"{"type":"sample","note":"a \"q\"\n\u0001é","ms":412.5,"n":3,"ok":true}"#);
        assert_eq!(parse(&line), Some(o));
        assert_eq!(parse(r#" { "a" : null , "b":-1e3 } "#).unwrap().get("b"), Some(&Value::Num(-1000.0)));
        assert_eq!(parse("{}"), Some(Obj::new()));
        for bad in ["", "{", r#"{"a":}"#, r#"{"a":1} x"#, r#"{"a":[1]}"#, r#"{"a":"\q"}"#] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
