use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Microseconds since the epoch.
pub fn now_usec() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

fn named_entities() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static TABLE: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| entities::ENTITIES.iter().map(|e| (e.entity, e.characters)).collect())
}

/// Decodes HTML character references (named and numeric) in plain text. Unknown references
/// are kept as they are.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut cursor = text;
    let mut buf = String::with_capacity(text.len());
    while let Some(start) = cursor.find('&') {
        buf.push_str(&cursor[..start]);
        cursor = &cursor[start..];
        let Some(end) = cursor.find(';') else {
            break;
        };
        let reference = &cursor[..=end];
        if let Some(repl) = named_entities().get(reference) {
            buf.push_str(repl);
        } else if let Some(num) = reference[1..].strip_prefix('#') {
            let num = &num[..num.len() - 1];
            let parsed = match num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => num.parse::<u32>().ok(),
            };
            match parsed.and_then(char::from_u32) {
                Some(ch) => buf.push(ch),
                None => buf.push_str(reference),
            }
        } else {
            // Not a reference: keep the ampersand and look again after it.
            buf.push('&');
            cursor = &cursor[1..];
            continue;
        }
        cursor = &cursor[end + 1..];
    }
    buf.push_str(cursor);
    buf
}

/// At most `max` characters, cut at the last space before the limit with an ellipsis appended
/// (or inside the word when there is no space). Unchanged when it already fits.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max).collect();
    let cut = match prefix.rfind(' ').filter(|i| *i > 0) {
        Some(i) => &prefix[..i],
        None => &prefix[..],
    };
    format!("{}\u{2026}", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_entities() {
        assert_eq!(decode_entities("Fine&mdash;Eventually."), "Fine\u{2014}Eventually.");
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&#38;"), "&");
        assert_eq!(decode_entities("&#x2014;"), "\u{2014}");
        assert_eq!(decode_entities("a &zZz; b"), "a &zZz; b");
        assert_eq!(decode_entities("AT&T"), "AT&T");
        assert_eq!(decode_entities("AT&T &amp; co"), "AT&T & co");
        assert_eq!(decode_entities("plain"), "plain");
    }

    #[test]
    fn shortens_at_words() {
        assert_eq!(shorten("short", 40), "short");
        assert_eq!(shorten("alpha beta gamma", 12), "alpha beta\u{2026}");
        assert_eq!(shorten("abcdefghij", 4), "abcd\u{2026}");
    }
}
