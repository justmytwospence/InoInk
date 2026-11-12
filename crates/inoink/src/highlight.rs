//! Highlights and annotations: Inoreader's read-only annotations and (later) quote anchoring.

use serde_json::Value;

/// One annotation as returned by Inoreader with `annotations=1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteAnnotation {
    pub id: i64,
    pub start: i64,
    pub end: i64,
    pub text: String,
    pub note: String,
    pub added_on: i64,
}

fn num(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The annotations of an article object, or `None` when the object carries no `annotations`
/// array (it was fetched without them, so nothing may be concluded). Entries without an id
/// are skipped.
pub fn parse_annotations(item: &Value) -> Option<Vec<RemoteAnnotation>> {
    let arr = item["annotations"].as_array()?;
    Some(
        arr.iter()
            .filter_map(|a| {
                Some(RemoteAnnotation {
                    id: num(&a["id"])?,
                    start: num(&a["start"]).unwrap_or(0),
                    end: num(&a["end"]).unwrap_or(0),
                    text: a["text"].as_str().unwrap_or("").to_string(),
                    note: a["note"].as_str().unwrap_or("").to_string(),
                    added_on: num(&a["added_on"]).unwrap_or(0),
                })
            })
            .collect(),
    )
}
