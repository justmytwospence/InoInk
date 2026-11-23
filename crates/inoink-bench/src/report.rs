//! The summary of one or more result files, as Markdown for doc/INOINK-ACCEPTANCE.md.

use crate::json::{parse, Obj, Value};
use std::collections::BTreeMap;

/// How each measured field reads in the table, and its unit.
fn measure(field: &str) -> Option<(&'static str, Unit)> {
    Some(match field {
        "ms" => ("time", Unit::Ms),
        "first_ms" => ("first change", Unit::Ms),
        "settled_ms" => ("settled", Unit::Ms),
        "total_ms" => ("until done", Unit::Ms),
        "rx_bytes" => ("received", Unit::KiB),
        "tx_bytes" => ("sent", Unit::KiB),
        "rss_kib" => ("resident", Unit::MiB),
        "hwm_kib" => ("peak", Unit::MiB),
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum Unit {
    Ms,
    KiB,
    MiB,
}

fn show(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Ms if v >= 100.0 => format!("{v:.0} ms"),
        Unit::Ms => format!("{v:.1} ms"),
        Unit::KiB => format!("{:.1} KiB", v / 1024.0),
        Unit::MiB => format!("{:.1} MiB", v / 1024.0),
    }
}

pub fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Order of the scenarios in the table: as they run.
fn rank(scenario: &str) -> usize {
    ["startup.", "memory.list", "sync.", "list.", "article.open", "page.", "memory.article", "article.back", "memory.end"]
        .iter()
        .position(|p| scenario.starts_with(p))
        .unwrap_or(99)
}

pub fn report(files: &[(String, String)]) -> Result<String, String> {
    let mut header: Option<Obj> = None;
    let mut runs = 0;
    let mut values: BTreeMap<(usize, String, String), Vec<f64>> = BTreeMap::new();
    let mut notes = Vec::new();
    for (name, text) in files {
        let mut ended = false;
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            let obj = parse(line).ok_or_else(|| format!("{name}:{}: not a result line", n + 1))?;
            match obj.str("type") {
                Some("header") => {
                    runs += 1;
                    if let Some(h) = &header {
                        for k in ["release", "product", "firmware"] {
                            if h.str(k) != obj.str(k) {
                                return Err(format!("{name}: a different {k} than the other files; summarize each separately"));
                            }
                        }
                    } else {
                        header = Some(obj);
                    }
                }
                Some("sample") => {
                    let scenario = obj.str("scenario").unwrap_or("?").to_string();
                    for (k, v) in &obj.0 {
                        if let (Some(_), Value::Num(x)) = (measure(k), v) {
                            values.entry((rank(&scenario), scenario.clone(), k.clone())).or_default().push(*x);
                        }
                    }
                }
                Some("skip") => notes.push(format!("{}: skipped, {}", obj.str("scenario").unwrap_or("?"), obj.str("reason").unwrap_or(""))),
                Some("error") => notes.push(format!("{name}: stopped, {}", obj.str("message").unwrap_or(""))),
                Some("end") => ended = true,
                _ => {}
            }
        }
        if !ended {
            notes.push(format!("{name}: the run did not finish"));
        }
    }
    let h = header.ok_or("no header line: not an inoink-bench result")?;
    let mut out = String::new();
    out.push_str(&format!(
        "InoInk {} on a {} ({}), firmware {}; {} run{}, first on {}.\n\n",
        h.str("release").unwrap_or("?"),
        h.str("model").unwrap_or("?"),
        h.str("product").unwrap_or("?"),
        h.str("firmware").unwrap_or("?"),
        runs,
        if runs == 1 { "" } else { "s" },
        h.str("started").unwrap_or("?"),
    ));
    out.push_str("| Scenario | Measure | Samples | Median | Range |\n|---|---|---:|---:|---:|\n");
    for ((_, scenario, field), mut v) in values {
        let (label, unit) = measure(&field).unwrap();
        let (lo, hi) = (v.iter().cloned().fold(f64::INFINITY, f64::min), v.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        let m = median(&mut v);
        out.push_str(&format!("| `{scenario}` | {label} | {} | {} | {} to {} |\n", v.len(), show(m, unit), show(lo, unit), show(hi, unit)));
    }
    if !notes.is_empty() {
        out.push('\n');
        for n in notes {
            out.push_str(&format!("- {n}\n"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN: &str = r#"{"type":"header","release":"0.9.1","product":"monza","model":"Libra Colour","firmware":"4.46.23852","started":"2026-11-20T10:00:00Z"}
{"type":"sample","scenario":"startup.first-list-frame-sent","n":1,"ms":1800}
{"type":"sample","scenario":"page.turn","n":1,"first_ms":90.5,"settled_ms":120,"changed":0.8}
{"type":"sample","scenario":"page.turn","n":2,"first_ms":95,"settled_ms":140,"changed":0.8}
{"type":"sample","scenario":"page.turn","n":3,"first_ms":99,"settled_ms":400,"changed":0.8}
{"type":"sample","scenario":"memory.end","n":1,"rss_kib":31744,"hwm_kib":40960}
{"type":"skip","scenario":"sync.idle","reason":"Wi-Fi is off"}
{"type":"end","ok":true}
"#;

    #[test]
    fn medians() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn table_in_run_order() {
        let r = report(&[("a.jsonl".into(), RUN.into())]).unwrap();
        assert!(r.starts_with("InoInk 0.9.1 on a Libra Colour (monza), firmware 4.46.23852; 1 run, first on 2026-11-20T10:00:00Z."));
        let rows: Vec<&str> = r.lines().filter(|l| l.starts_with("| `")).collect();
        assert_eq!(
            rows,
            vec![
                "| `startup.first-list-frame-sent` | time | 1 | 1800 ms | 1800 ms to 1800 ms |",
                "| `page.turn` | first change | 3 | 95.0 ms | 90.5 ms to 99.0 ms |",
                "| `page.turn` | settled | 3 | 140 ms | 120 ms to 400 ms |",
                "| `memory.end` | peak | 1 | 40.0 MiB | 40.0 MiB to 40.0 MiB |",
                "| `memory.end` | resident | 1 | 31.0 MiB | 31.0 MiB to 31.0 MiB |",
            ]
        );
        assert!(r.contains("- sync.idle: skipped, Wi-Fi is off"));
    }

    #[test]
    fn runs_pool_and_must_match() {
        let r = report(&[("a".into(), RUN.into()), ("b".into(), RUN.lines().take(3).collect::<Vec<_>>().join("\n"))]).unwrap();
        assert!(r.contains("2 runs"));
        assert!(r.contains("| `page.turn` | first change | 4 |"));
        assert!(r.contains("- b: the run did not finish"));
        let other = RUN.replace("0.9.1", "0.9.2");
        assert!(report(&[("a".into(), RUN.into()), ("b".into(), other)]).unwrap_err().contains("different release"));
        assert!(report(&[("a".into(), "nonsense".into())]).is_err());
        assert!(report(&[("a".into(), String::new())]).is_err());
    }
}
