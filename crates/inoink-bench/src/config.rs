//! The run's settings: `key = value` lines, `#` comments. Every key has a default derived from the
//! screen size and InoInk's layout (crates/core/src/view/inoink/home.rs, top_bar.rs), so a missing
//! or empty file is a complete configuration.

use crate::screen::Rect;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Scenarios to run after startup, in this order: sync, list, article.
    pub scenarios: Vec<String>,
    /// Article opens (and list page turns).
    pub samples: u32,
    pub page_turns: u32,
    pub sync_samples: u32,
    /// The screen counts as settled after this long without a change.
    pub quiet_ms: u64,
    /// The longest an action may take before the run stops.
    pub timeout_ms: u64,
    pub sync_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    /// Before the first scenario, the screen must stay unchanged this long (the startup sync).
    pub startup_quiet_ms: u64,
    /// Pixels per signature cell on the whole screen.
    pub cell: u32,
    pub tap_article: (u32, u32),
    pub tap_sync: (u32, u32),
    pub sync_icon: Rect,
    pub tap_bars: (u32, u32),
    pub tap_back: (u32, u32),
    /// Changes here never count (the clock and the battery).
    pub ignore: Vec<Rect>,
}

pub const SCENARIOS: [&str; 3] = ["sync", "list", "article"];

impl Config {
    /// Defaults for a portrait screen of `w` x `h` pixels at `dpi`.
    pub fn defaults(w: u32, h: u32, dpi: u32) -> Config {
        let px = |v: f32| (v * dpi as f32 / 300.0) as u32;
        let (small, big, thick) = (px(121.0), px(163.0), px(2.0).max(1));
        // The top bar is a small bar less half the separator; its icons sit in slots of 4/5 of
        // its height from the right: menu, light, Wi-Fi, then sync.
        let side = small - thick / 2;
        let slot = side * 4 / 5;
        let list_top = 2 * small + (thick - thick / 2);
        Config {
            scenarios: SCENARIOS.iter().map(|s| s.to_string()).collect(),
            samples: 5,
            page_turns: 10,
            sync_samples: 3,
            quiet_ms: 800,
            timeout_ms: 15_000,
            sync_timeout_ms: 120_000,
            startup_timeout_ms: 240_000,
            startup_quiet_ms: 8_000,
            cell: 24,
            tap_article: (w * 2 / 5, list_top + big / 2),
            tap_sync: (w - 3 * slot - slot / 2, side / 2),
            sync_icon: Rect::new(w - 4 * slot, 0, w - 3 * slot, side),
            tap_bars: (w / 2, h / 2),
            tap_back: (w * 3 / 10, side / 2),
            ignore: vec![Rect::new(0, 0, w / 3, side)],
        }
    }

    pub fn parse(mut self, text: &str) -> Result<Config, String> {
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=').ok_or_else(|| format!("line {}: expected key = value", n + 1))?;
            let (k, v) = (k.trim(), v.trim());
            let bad = || format!("line {}: invalid value for {k}: {v}", n + 1);
            let num = || v.parse::<u64>().map_err(|_| bad());
            let point = || -> Result<(u32, u32), String> {
                let p: Vec<u32> = v.split(',').map(|s| s.trim().parse().ok()).collect::<Option<_>>().ok_or_else(bad)?;
                match p[..] {
                    [x, y] => Ok((x, y)),
                    _ => Err(bad()),
                }
            };
            match k {
                "scenarios" => {
                    let list: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    if list.is_empty() || list.iter().any(|s| !SCENARIOS.contains(&s.as_str())) {
                        return Err(bad());
                    }
                    self.scenarios = list;
                }
                "samples" => self.samples = num()?.clamp(1, 100) as u32,
                "page_turns" => self.page_turns = num()?.min(200) as u32,
                "sync_samples" => self.sync_samples = num()?.min(20) as u32,
                "quiet_ms" => self.quiet_ms = num()?.max(100),
                "timeout_ms" => self.timeout_ms = num()?.max(1000),
                "sync_timeout_ms" => self.sync_timeout_ms = num()?.max(1000),
                "startup_timeout_ms" => self.startup_timeout_ms = num()?.max(1000),
                "startup_quiet_ms" => self.startup_quiet_ms = num()?,
                "cell" => self.cell = num()?.clamp(4, 128) as u32,
                "tap_article" => self.tap_article = point()?,
                "tap_sync" => self.tap_sync = point()?,
                "tap_bars" => self.tap_bars = point()?,
                "tap_back" => self.tap_back = point()?,
                "sync_icon" => self.sync_icon = Rect::parse(v).ok_or_else(bad)?,
                "ignore" => {
                    self.ignore = if v.is_empty() || v == "none" {
                        Vec::new()
                    } else {
                        v.split(';').map(|r| Rect::parse(r).ok_or_else(bad)).collect::<Result<_, _>>()?
                    }
                }
                _ => return Err(format!("line {}: unknown setting {k}", n + 1)),
            }
        }
        Ok(self)
    }

    pub fn runs(&self, scenario: &str) -> bool {
        self.scenarios.iter().any(|s| s == scenario)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libra_colour_defaults_match_the_list_layout() {
        let c = Config::defaults(1264, 1680, 300);
        // The sync icon is the fourth slot from the right of the 120-pixel top bar (the preview's
        // 01-newsfeed screenshot has it centred at x 928); the first row starts below the tabs.
        assert_eq!(c.tap_sync, (928, 60));
        assert_eq!(c.sync_icon, Rect::new(880, 0, 976, 120));
        assert_eq!(c.tap_article, (505, 324));
        assert_eq!(c.tap_back, (379, 60));
        assert_eq!(c.ignore, vec![Rect::new(0, 0, 421, 120)]);
    }

    #[test]
    fn the_shipped_file_shows_the_libra_colour_defaults() {
        let file = include_str!("../device/bench.cfg");
        let defaults = Config::defaults(1264, 1680, 300);
        // Every setting is there, commented out, with its default.
        let uncommented: String = file
            .lines()
            .filter_map(|l| l.strip_prefix("# "))
            .filter(|l| l.split_once(" = ").is_some_and(|(k, _)| !k.contains(' ')))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(uncommented.lines().count(), 16);
        assert_eq!(defaults.clone().parse(&uncommented).unwrap(), defaults);
        assert_eq!(defaults.clone().parse(file).unwrap(), defaults);
    }

    #[test]
    fn settings_override_defaults() {
        let text = "# comment\nsamples = 3\ntap_sync = 10, 20 # inline\nignore = 0,0,5,5;10,10,20,20\nscenarios = article, list\n";
        let c = Config::defaults(1264, 1680, 300).parse(text).unwrap();
        assert_eq!(c.samples, 3);
        assert_eq!(c.tap_sync, (10, 20));
        assert_eq!(c.ignore.len(), 2);
        assert!(c.runs("list") && !c.runs("sync"));
        let d = Config::defaults(1264, 1680, 300);
        assert!(d.clone().parse("ignore = none").unwrap().ignore.is_empty());
        for bad in ["samples", "samples = x", "wat = 1", "tap_sync = 1", "scenarios = dance", "sync_icon = 1,2,3"] {
            assert!(d.clone().parse(bad).is_err(), "{bad}");
        }
    }
}
