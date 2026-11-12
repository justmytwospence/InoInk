//! Per-zone quota accounting from `X-Reader-Zone*` headers, with a labelled fallback estimate.

use serde::{Deserialize, Serialize};

pub const FALLBACK_LIMIT: u32 = 100;
pub const MIN_RESERVE: u32 = 5;
/// Used when a 429 carries no reset information.
pub const DEFAULT_PAUSE_SECS: i64 = 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Zone {
    One,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Automatic sync: leaves a reserve for explicit actions.
    Background,
    /// The user asked for it: may use the reserve.
    Explicit,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct ZoneState {
    limit: Option<u32>,
    usage: Option<u32>,
    /// Requests we sent since the last header-reported usage (or since reset).
    sent_since: u32,
    /// True once real numbers have been seen.
    observed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    one: ZoneState,
    two: ZoneState,
    /// Unix time at which usage resets, when known.
    reset_at: Option<i64>,
    /// Requests are refused until this time after a 429.
    paused_until: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remaining {
    pub remaining: u32,
    pub limit: u32,
    /// True when the numbers are our fallback estimate rather than server-reported.
    pub estimated: bool,
}

impl Quota {
    fn zone(&self, z: Zone) -> &ZoneState {
        match z {
            Zone::One => &self.one,
            Zone::Two => &self.two,
        }
    }
    fn zone_mut(&mut self, z: Zone) -> &mut ZoneState {
        match z {
            Zone::One => &mut self.one,
            Zone::Two => &mut self.two,
        }
    }

    pub fn from_json(s: &str) -> Quota {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Forget usage once the reset time has passed.
    pub fn tick(&mut self, now: i64) {
        if let Some(r) = self.reset_at {
            if now >= r {
                for z in [Zone::One, Zone::Two] {
                    let s = self.zone_mut(z);
                    s.usage = Some(0);
                    s.sent_since = 0;
                }
                self.reset_at = None;
            }
        }
        if let Some(p) = self.paused_until {
            if now >= p {
                self.paused_until = None;
            }
        }
    }

    pub fn remaining(&self, z: Zone) -> Remaining {
        let s = self.zone(z);
        let limit = s.limit.unwrap_or(FALLBACK_LIMIT);
        let used = s.usage.unwrap_or(0) + s.sent_since;
        Remaining { remaining: limit.saturating_sub(used), limit, estimated: !s.observed }
    }

    /// Requests counted against `z` in the current period (server-reported plus ours since).
    pub fn used(&self, z: Zone) -> u32 {
        let s = self.zone(z);
        s.usage.unwrap_or(0) + s.sent_since
    }

    /// Unix time at which usage resets, when the server said so.
    pub fn reset_at(&self) -> Option<i64> {
        self.reset_at
    }

    pub fn reserve(&self, z: Zone) -> u32 {
        let limit = self.remaining(z).limit;
        MIN_RESERVE.max(limit / 10)
    }

    /// Seconds until requests may resume, if currently paused or exhausted for this priority.
    pub fn blocked_for(&self, z: Zone, prio: Priority, now: i64) -> Option<u64> {
        if let Some(p) = self.paused_until {
            if p > now {
                return Some((p - now) as u64);
            }
        }
        let r = self.remaining(z);
        let floor = match prio {
            Priority::Background => self.reserve(z),
            Priority::Explicit => 0,
        };
        if r.remaining <= floor {
            let wait = self.reset_at.map(|t| (t - now).max(1) as u64).unwrap_or(DEFAULT_PAUSE_SECS as u64);
            return Some(wait);
        }
        None
    }

    pub fn note_sent(&mut self, z: Zone) {
        self.zone_mut(z).sent_since += 1;
    }

    /// Updates from response headers (names compared case-insensitively by the caller).
    pub fn observe(&mut self, headers: &[(String, String)], now: i64) {
        let get = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.trim().parse::<u32>().ok());
        for (z, lim, use_) in
            [(Zone::One, "x-reader-zone1-limit", "x-reader-zone1-usage"), (Zone::Two, "x-reader-zone2-limit", "x-reader-zone2-usage")]
        {
            let (l, u) = (get(lim), get(use_));
            if l.is_some() || u.is_some() {
                let s = self.zone_mut(z);
                if l.is_some() {
                    s.limit = l;
                }
                if u.is_some() {
                    s.usage = u;
                    s.sent_since = 0;
                }
                s.observed = true;
            }
        }
        if let Some(reset) = get("x-reader-limits-reset-after") {
            self.reset_at = Some(now + reset as i64);
        }
    }

    /// A 429: refuse everything until the reset (or `retry_after`).
    pub fn pause(&mut self, secs: Option<u64>, now: i64) {
        let secs = secs.map(|s| s as i64).or(self.reset_at.map(|r| (r - now).max(1))).unwrap_or(DEFAULT_PAUSE_SECS);
        self.paused_until = Some(now + secs);
        self.reset_at = Some(now + secs);
    }

    pub fn paused_until(&self) -> Option<i64> {
        self.paused_until
    }
}
