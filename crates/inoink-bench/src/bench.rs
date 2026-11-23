//! The scenarios: each sends input the way a reader would, then times the screen until it settles.

use std::io::{self, Write};
use std::time::Duration;

use crate::config::Config;
use crate::input::{Input, KEY_BACKWARD, KEY_FORWARD};
use crate::json::Obj;
use crate::screen::{diff, Change, Rect, Screen, Signature};

pub trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

pub struct SystemClock(pub std::time::Instant);

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d)
    }
}

/// What the bench reads about InoInk and the device besides the screen.
pub trait Probe {
    /// Resident and peak memory of the app, in KiB.
    fn memory(&mut self) -> Option<(u64, u64)>;
    fn wifi_up(&mut self) -> bool;
    /// Bytes received and sent by the Wi-Fi interface.
    fn net_bytes(&mut self) -> Option<(u64, u64)>;
    /// The app's own `startup:` lines of this launch, as (step, ms); None while they are not all
    /// there; Err when InoInk did not start (for example, it was already running).
    fn startup(&mut self) -> std::result::Result<Option<Vec<(String, u64)>>, String>;
}

/// Where results go: one JSON object per line, flushed at once so a run cut short keeps them.
pub trait Sink {
    fn line(&mut self, obj: Obj) -> io::Result<()>;
}

impl<W: Write> Sink for W {
    fn line(&mut self, obj: Obj) -> io::Result<()> {
        writeln!(self, "{obj}")?;
        self.flush()
    }
}

#[derive(Debug)]
pub enum Stop {
    /// Something on the screen was not what the run expected; it stops sending input.
    Unexpected(String),
    Io(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Stop {
        Stop::Io(e)
    }
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Stop::Unexpected(s) => f.write_str(s),
            Stop::Io(e) => write!(f, "{e}"),
        }
    }
}

type Result<T> = std::result::Result<T, Stop>;

fn stop<T>(msg: impl Into<String>) -> Result<T> {
    Err(Stop::Unexpected(msg.into()))
}

/// The timing of one action.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// From the input to the first change on the screen.
    pub first_ms: f64,
    /// From the input to the last change before the screen stayed quiet.
    pub settled_ms: f64,
    /// What differs between before and after.
    pub change: Change,
}

const POLL: Duration = Duration::from_millis(2);
const SMALL_CELL: u32 = 4;

pub struct Bench<'a> {
    pub screen: &'a mut dyn Screen,
    pub input: &'a mut dyn Input,
    pub clock: &'a dyn Clock,
    pub probe: &'a mut dyn Probe,
    pub out: &'a mut dyn Sink,
    pub cfg: Config,
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

impl Bench<'_> {
    fn full(&self) -> Rect {
        let (w, h) = self.screen.size();
        Rect::new(0, 0, w, h)
    }

    fn sig(&mut self) -> Result<Signature> {
        let (r, c) = (self.full(), self.cfg.cell);
        Ok(self.screen.signature(r, c)?)
    }

    fn changed(&self, a: &Signature, b: &Signature) -> Change {
        diff(a, b, &self.cfg.ignore)
    }

    /// Waits until the screen has not changed for `quiet`, at most `max`. Returns whether it did.
    fn wait_quiet(&mut self, quiet: Duration, max: Duration) -> Result<bool> {
        let start = self.clock.now();
        let mut last = self.sig()?;
        let mut since = start;
        loop {
            let now = self.clock.now();
            if now - since >= quiet {
                return Ok(true);
            }
            if now - start >= max {
                return Ok(false);
            }
            self.clock.sleep(Duration::from_millis(50));
            let s = self.sig()?;
            if self.changed(&last, &s).any() {
                last = s;
                since = self.clock.now();
            }
        }
    }

    /// Times the screen after an input sent at `t0`: the first change from `before`, then the
    /// last change before it stays quiet. None when nothing changes within `wait`.
    fn measure(&mut self, t0: Duration, before: &Signature, wait: Duration) -> Result<Option<Timing>> {
        let quiet = Duration::from_millis(self.cfg.quiet_ms);
        let limit = Duration::from_millis(self.cfg.timeout_ms);
        let mut first = None;
        let mut last = before.clone();
        let mut last_change = t0;
        loop {
            let s = self.sig()?;
            let now = self.clock.now();
            if self.changed(&last, &s).any() {
                last = s;
                last_change = now;
                first.get_or_insert(now);
            }
            match first {
                None if now - t0 >= wait => return Ok(None),
                Some(f) if now - last_change >= quiet => {
                    let change = self.changed(before, &last);
                    return Ok(Some(Timing { first_ms: ms(f - t0), settled_ms: ms(last_change - t0), change }));
                }
                Some(_) if now - t0 >= limit => return stop(format!("the screen kept changing for {} s", limit.as_secs())),
                _ => {}
            }
            self.clock.sleep(POLL);
        }
    }

    fn act(&mut self, what: impl FnOnce(&mut dyn Input) -> io::Result<()>, wait: Duration) -> Result<Option<Timing>> {
        let before = self.sig()?;
        what(self.input)?;
        // From the end of the input: InoInk acts when the finger lifts or the button is released.
        let t0 = self.clock.now();
        self.measure(t0, &before, wait)
    }

    fn key(&mut self, code: u16) -> Result<Option<Timing>> {
        let wait = Duration::from_millis(self.cfg.timeout_ms);
        self.act(|i| i.key(code), wait)
    }

    fn tap(&mut self, p: (u32, u32)) -> Result<Option<Timing>> {
        let wait = Duration::from_millis(self.cfg.timeout_ms);
        self.act(|i| i.tap(p.0, p.1), wait)
    }

    fn sample(&mut self, scenario: &str, n: u32, t: &Timing) -> Result<()> {
        let o = Obj::new()
            .s("type", "sample")
            .s("scenario", scenario)
            .n("n", n)
            .n("first_ms", t.first_ms)
            .n("settled_ms", t.settled_ms)
            .n("changed", (t.change.fraction * 1000.0).round() / 1000.0);
        Ok(self.out.line(o)?)
    }

    fn skip(&mut self, scenario: &str, reason: &str) -> Result<()> {
        Ok(self.out.line(Obj::new().s("type", "skip").s("scenario", scenario).s("reason", reason))?)
    }

    fn memory(&mut self, scenario: &str) -> Result<()> {
        match self.probe.memory() {
            Some((rss, hwm)) => Ok(self.out.line(
                Obj::new().s("type", "sample").s("scenario", scenario).n("n", 1).n("rss_kib", rss as f64).n("hwm_kib", hwm as f64),
            )?),
            None => self.skip(scenario, "InoInk is not running"),
        }
    }

    /// The whole run. Results are written as they come; an unexpected screen stops the run.
    pub fn run(&mut self) -> Result<()> {
        // Always: InoInk must have shown its list before anything else.
        self.startup()?;
        self.memory("memory.list")?;
        if self.cfg.runs("sync") {
            self.sync()?;
        }
        if self.cfg.runs("list") {
            self.list()?;
        }
        if self.cfg.runs("article") {
            self.article()?;
        }
        self.memory("memory.end")
    }

    fn startup(&mut self) -> Result<()> {
        let start = self.clock.now();
        let limit = Duration::from_millis(self.cfg.startup_timeout_ms);
        loop {
            match self.probe.startup() {
                Ok(Some(steps)) => {
                    for (step, value) in steps {
                        let scenario = format!("startup.{}", step.replace(' ', "-"));
                        self.out.line(Obj::new().s("type", "sample").s("scenario", &scenario).n("n", 1).n("ms", value as f64))?;
                    }
                    break;
                }
                Ok(None) if self.clock.now() - start < limit => self.clock.sleep(Duration::from_millis(250)),
                Ok(None) => return stop("InoInk did not finish starting (no \"startup: first list frame sent\" in its log)"),
                Err(reason) => {
                    self.skip("startup", &reason)?;
                    break;
                }
            }
        }
        // Let the startup sync and its redraws finish.
        let quiet = Duration::from_millis(self.cfg.startup_quiet_ms);
        if !self.wait_quiet(quiet, limit)? {
            return stop("the screen did not settle after startup");
        }
        Ok(())
    }

    /// Sync with nothing new: tap the sync button and time its "syncing" form.
    fn sync(&mut self) -> Result<()> {
        if self.cfg.sync_samples == 0 {
            return Ok(());
        }
        if !self.probe.wifi_up() {
            return self.skip("sync.idle", "Wi-Fi is off");
        }
        let icon = self.cfg.sync_icon;
        let (w, _) = self.screen.size();
        let top = Rect::new(0, 0, w, icon.y1);
        let p = self.cfg.tap_sync;
        let mut retried = false;
        let mut n = 1;
        while n <= self.cfg.sync_samples {
            self.wait_quiet(Duration::from_millis(2000), Duration::from_secs(60))?;
            let before_icon = self.screen.signature(icon, SMALL_CELL)?;
            let before = self.sig()?;
            let bytes = self.probe.net_bytes();
            self.input.tap(p.0, p.1)?;
            let t0 = self.clock.now();
            // The icon must change first, and nothing below the top bar.
            let mut started = None;
            while started.is_none() {
                let now = self.clock.now();
                let s = self.screen.signature(icon, SMALL_CELL)?;
                if diff(&before_icon, &s, &[]).any() {
                    started = Some(now);
                    break;
                }
                let full = self.sig()?;
                let c = self.changed(&before, &full);
                if c.bbox.is_some_and(|b| !top.contains(&b)) {
                    return stop(format!("tapping sync at {},{} changed the screen at {}: check tap_sync", p.0, p.1, c.bbox.unwrap()));
                }
                if now - t0 >= Duration::from_secs(5) {
                    break;
                }
                self.clock.sleep(POLL);
            }
            let Some(started) = started else {
                if retried {
                    return stop(format!(
                        "tapping sync at {},{} did not change the sync icon at {}: check tap_sync and sync_icon",
                        p.0, p.1, icon
                    ));
                }
                // A sync may still be running (its icon does not move): wait for it to end, once.
                retried = true;
                let limit = Duration::from_millis(self.cfg.sync_timeout_ms);
                let t = self.clock.now();
                while self.screen.signature(icon, SMALL_CELL)? == before_icon && self.clock.now() - t < limit {
                    self.clock.sleep(Duration::from_millis(50));
                }
                continue;
            };
            // The sync is over when the icon is back to its plain form.
            let limit = Duration::from_millis(self.cfg.sync_timeout_ms);
            let done = loop {
                let now = self.clock.now();
                if self.screen.signature(icon, SMALL_CELL)? == before_icon {
                    break now;
                }
                if now - t0 >= limit {
                    return stop(format!("the sync did not end within {} s", limit.as_secs()));
                }
                self.clock.sleep(Duration::from_millis(10));
            };
            let mut o = Obj::new()
                .s("type", "sample")
                .s("scenario", "sync.idle")
                .n("n", n)
                .n("first_ms", ms(started - t0))
                .n("total_ms", ms(done - t0));
            if let (Some((r0, t0b)), Some((r1, t1b))) = (bytes, self.probe.net_bytes()) {
                o = o.n("rx_bytes", r1.saturating_sub(r0) as f64).n("tx_bytes", t1b.saturating_sub(t0b) as f64);
            }
            self.out.line(o)?;
            n += 1;
        }
        Ok(())
    }

    /// Page buttons on the list: forward and back in turn, so it ends on the first page.
    fn list(&mut self) -> Result<()> {
        let n = self.cfg.samples * 2;
        for i in 0..n {
            let code = if i % 2 == 0 { KEY_FORWARD } else { KEY_BACKWARD };
            match self.key(code)? {
                None if i == 0 => {
                    return self
                        .skip("list.page", "the page button changed nothing: the list has one page, or the buttons do not reach InoInk")
                }
                None => return stop("a page button changed nothing on the list"),
                Some(t) if t.change.fraction < 0.2 => {
                    return stop(format!("a page button changed only {:.0}% of the list", t.change.fraction * 100.0))
                }
                Some(t) => self.sample("list.page", i + 1, &t)?,
            }
        }
        Ok(())
    }

    /// Open the first article, turn its pages (the first time), and go back to the list.
    fn article(&mut self) -> Result<()> {
        for i in 1..=self.cfg.samples {
            self.wait_quiet(Duration::from_millis(1500), Duration::from_secs(30))?;
            let p = self.cfg.tap_article;
            match self.tap(p)? {
                None => return stop(format!("tapping the article at {},{} changed nothing: check tap_article", p.0, p.1)),
                Some(t) if t.change.fraction < 0.5 => {
                    return stop(format!(
                        "tapping the article at {},{} changed only {:.0}% of the screen: check tap_article",
                        p.0,
                        p.1,
                        t.change.fraction * 100.0
                    ))
                }
                Some(t) => self.sample("article.open", i, &t)?,
            }
            if i == 1 {
                self.page_turns()?;
                self.memory("memory.article")?;
            }
            // The bars first (a tap in the middle), then the title, which goes back.
            let p = self.cfg.tap_bars;
            match self.tap(p)? {
                None => return stop(format!("tapping the middle of the article at {},{} did not show its bars: check tap_bars", p.0, p.1)),
                Some(t) if t.change.fraction > 0.5 => {
                    return stop(format!(
                        "tapping the middle of the article at {},{} changed most of the screen (a link or an image?)",
                        p.0, p.1
                    ))
                }
                Some(_) => {}
            }
            let p = self.cfg.tap_back;
            match self.tap(p)? {
                None => return stop(format!("tapping the title at {},{} did not go back: check tap_back", p.0, p.1)),
                Some(t) if t.change.fraction < 0.5 => {
                    return stop(format!(
                        "tapping the title at {},{} changed only {:.0}% of the screen: check tap_back",
                        p.0,
                        p.1,
                        t.change.fraction * 100.0
                    ))
                }
                Some(t) => self.sample("article.back", i, &t)?,
            }
        }
        Ok(())
    }

    fn page_turns(&mut self) -> Result<()> {
        let n = self.cfg.page_turns;
        for i in 0..n {
            let code = if i % 2 == 0 { KEY_FORWARD } else { KEY_BACKWARD };
            match self.key(code)? {
                None if i == 0 => return self.skip("page.turn", "the page button changed nothing: the buttons do not reach InoInk"),
                None => return stop("a page button changed nothing in the article"),
                Some(t) if t.change.fraction < 0.15 => {
                    return stop(format!(
                        "a page turn changed only {:.0}% of the screen: put an article of several pages first in the list",
                        t.change.fraction * 100.0
                    ))
                }
                Some(t) => self.sample("page.turn", i + 1, &t)?,
            }
        }
        if n % 2 == 1 {
            self.key(KEY_BACKWARD)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::sign;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A pretend InoInk on a pretend clock: the screen is a few named states, and input moves
    /// between them after a delay.
    struct World {
        now: Duration,
        screen: &'static str,
        pending: Vec<(Duration, &'static str, Option<u32>)>,
        /// The page the app is on; `page` is the one on the screen.
        target: u32,
        syncing_until: Option<Duration>,
        page: u32,
        pages: u32,
        log: Vec<String>,
        wifi: bool,
    }

    impl World {
        fn new() -> Rc<RefCell<World>> {
            Rc::new(RefCell::new(World {
                now: Duration::ZERO,
                screen: "list",
                pending: Vec::new(),
                target: 0,
                syncing_until: None,
                page: 0,
                pages: 3,
                log: Vec::new(),
                wifi: true,
            }))
        }
        fn advance(&mut self) {
            self.pending.sort_by_key(|p| p.0);
            while let Some(&(at, s, page)) = self.pending.first() {
                if at > self.now {
                    break;
                }
                self.screen = s;
                if let Some(p) = page {
                    self.page = p;
                }
                self.pending.remove(0);
            }
            if self.syncing_until.is_some_and(|t| t <= self.now) {
                self.syncing_until = None;
            }
        }
        fn after(&mut self, ms: u64, s: &'static str, page: Option<u32>) {
            let at = self.now + Duration::from_millis(ms);
            self.pending.push((at, s, page));
        }
        fn pixel(&self, x: u32, y: u32) -> u32 {
            if y < 120 && (880..976).contains(&x) {
                return if self.syncing_until.is_some() { 7 } else { 1 };
            }
            if y < 120 {
                return 1;
            }
            match self.screen {
                "list" => 10 + self.page,
                "article" => 100 + self.page,
                // The bars cover the top and the bottom.
                "bars" if !(200..1500).contains(&y) => 300,
                "bars" => 100 + self.page,
                _ => 0,
            }
        }
    }

    struct FakeClock(Rc<RefCell<World>>);
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0.borrow().now
        }
        fn sleep(&self, d: Duration) {
            let mut w = self.0.borrow_mut();
            w.now += d;
            w.advance();
        }
    }

    struct FakeScreen(Rc<RefCell<World>>);
    impl Screen for FakeScreen {
        fn size(&self) -> (u32, u32) {
            (1264, 1680)
        }
        fn signature(&mut self, region: Rect, cell: u32) -> io::Result<Signature> {
            let mut w = self.0.borrow_mut();
            // Every look at the screen takes a millisecond.
            w.now += Duration::from_millis(1);
            w.advance();
            Ok(sign(region, cell, |x, y| w.pixel(x, y)))
        }
    }

    struct FakeInput(Rc<RefCell<World>>);
    impl Input for FakeInput {
        fn key(&mut self, code: u16) -> io::Result<()> {
            let mut w = self.0.borrow_mut();
            w.log.push(format!("key {code}"));
            let forward = code == KEY_FORWARD;
            if w.target + 1 < w.pages && forward || w.target > 0 && !forward {
                if forward {
                    w.target += 1
                } else {
                    w.target -= 1
                }
                let (s, t) = (w.screen, w.target);
                // Redraw: blank for a moment, then the new page.
                w.after(150, "blank", None);
                w.after(180, s, Some(t));
            }
            Ok(())
        }
        fn tap(&mut self, x: u32, y: u32) -> io::Result<()> {
            let mut w = self.0.borrow_mut();
            w.log.push(format!("tap {x},{y}"));
            match (w.screen, x, y) {
                ("list", 928, 60) if w.syncing_until.is_none() => {
                    w.syncing_until = Some(w.now + Duration::from_millis(1200));
                }
                ("list", 505, 324) => {
                    w.target = 0;
                    w.after(300, "article", Some(0));
                }
                ("article", 632, 840) => w.after(100, "bars", None),
                ("bars", 379, 60) => {
                    w.target = 0;
                    w.after(250, "list", Some(0));
                }
                _ => {}
            }
            Ok(())
        }
    }

    struct FakeProbe(Rc<RefCell<World>>, bool);
    impl Probe for FakeProbe {
        fn memory(&mut self) -> Option<(u64, u64)> {
            Some((30_000, 41_000))
        }
        fn wifi_up(&mut self) -> bool {
            self.0.borrow().wifi
        }
        fn net_bytes(&mut self) -> Option<(u64, u64)> {
            let t = self.0.borrow().now.as_millis() as u64;
            Some((t * 10, t))
        }
        fn startup(&mut self) -> std::result::Result<Option<Vec<(String, u64)>>, String> {
            if self.1 {
                return Err("InoInk was already running".into());
            }
            Ok(Some(vec![("database open".into(), 210), ("first list frame sent".into(), 1800)]))
        }
    }

    fn run(world: &Rc<RefCell<World>>, cfg: Config, already_running: bool) -> (std::result::Result<(), String>, Vec<Obj>) {
        let mut screen = FakeScreen(world.clone());
        let mut input = FakeInput(world.clone());
        let clock = FakeClock(world.clone());
        let mut probe = FakeProbe(world.clone(), already_running);
        let mut out: Vec<u8> = Vec::new();
        let result = {
            let mut b = Bench { screen: &mut screen, input: &mut input, clock: &clock, probe: &mut probe, out: &mut out, cfg };
            b.run().map_err(|e| e.to_string())
        };
        let lines = String::from_utf8(out).unwrap().lines().map(|l| crate::json::parse(l).unwrap()).collect();
        (result, lines)
    }

    fn cfg() -> Config {
        let mut c = Config::defaults(1264, 1680, 300);
        c.samples = 2;
        c.page_turns = 3;
        c.sync_samples = 2;
        c.startup_quiet_ms = 1000;
        c.timeout_ms = 2000;
        c.sync_timeout_ms = 30_000;
        // Coarser cells keep the simulated runs quick; the fake screen changes in large areas.
        c.cell = 64;
        c
    }

    fn samples<'a>(lines: &'a [Obj], scenario: &str) -> Vec<&'a Obj> {
        lines.iter().filter(|o| o.str("type") == Some("sample") && o.str("scenario") == Some(scenario)).collect()
    }

    #[test]
    fn a_full_run_times_every_scenario() {
        let world = World::new();
        let (result, lines) = run(&world, cfg(), false);
        assert_eq!(result, Ok(()), "{lines:?}");
        assert_eq!(samples(&lines, "startup.first-list-frame-sent")[0].get("ms").unwrap().as_f64(), Some(1800.0));
        let sync = samples(&lines, "sync.idle");
        assert_eq!(sync.len(), 2);
        let total = sync[0].get("total_ms").unwrap().as_f64().unwrap();
        assert!((1199.0..1260.0).contains(&total), "{total}");
        assert!(sync[0].get("rx_bytes").unwrap().as_f64().unwrap() > 0.0);
        // List pages: forward and back, twice each.
        let pages = samples(&lines, "list.page");
        assert_eq!(pages.len(), 4);
        for p in &pages {
            let first = p.get("first_ms").unwrap().as_f64().unwrap();
            let settled = p.get("settled_ms").unwrap().as_f64().unwrap();
            assert!((150.0..160.0).contains(&first), "{first}");
            assert!((180.0..190.0).contains(&settled), "{settled}");
        }
        assert_eq!(samples(&lines, "article.open").len(), 2);
        let open = samples(&lines, "article.open")[0].get("settled_ms").unwrap().as_f64().unwrap();
        assert!((300.0..310.0).contains(&open), "{open}");
        assert_eq!(samples(&lines, "page.turn").len(), 3);
        assert_eq!(samples(&lines, "article.back").len(), 2);
        assert_eq!(samples(&lines, "memory.article")[0].get("hwm_kib").unwrap().as_f64(), Some(41000.0));
        assert_eq!(samples(&lines, "memory.end").len(), 1);
        // An odd number of turns ends with one more back, so the article closes on its first page.
        assert_eq!(world.borrow().screen, "list");
        assert_eq!(world.borrow().log.iter().filter(|l| l.starts_with("key")).count(), 4 + 4);
    }

    #[test]
    fn wrong_coordinates_stop_the_run_before_more_input() {
        let world = World::new();
        let mut c = cfg();
        c.tap_sync = (10, 600);
        let (result, _) = run(&world, c, false);
        assert!(result.unwrap_err().contains("did not change the sync icon"));
        assert_eq!(world.borrow().log, vec!["tap 10,600", "tap 10,600"], "one retry, then nothing");

        let world = World::new();
        let mut c = cfg();
        c.scenarios = vec!["article".into()];
        c.tap_bars = (1, 1);
        let (result, lines) = run(&world, c, false);
        assert!(result.unwrap_err().contains("did not show its bars"));
        assert_eq!(samples(&lines, "article.open").len(), 1);
    }

    #[test]
    fn a_sync_already_running_is_waited_for_once() {
        let world = World::new();
        world.borrow_mut().syncing_until = Some(Duration::from_secs(20));
        let mut c = cfg();
        c.scenarios = vec!["sync".into()];
        let (result, lines) = run(&world, c, false);
        assert_eq!(result, Ok(()));
        assert_eq!(samples(&lines, "sync.idle").len(), 2);
        assert_eq!(world.borrow().log.len(), 3, "the first tap fell into the running sync");
        assert!(world.borrow().now > Duration::from_secs(20));
    }

    #[test]
    fn skips_are_recorded() {
        let world = World::new();
        world.borrow_mut().wifi = false;
        world.borrow_mut().pages = 1;
        let mut c = cfg();
        c.scenarios = vec!["sync".into(), "list".into()];
        let (result, lines) = run(&world, c, true);
        assert_eq!(result, Ok(()));
        let skips: Vec<_> = lines.iter().filter(|o| o.str("type") == Some("skip")).map(|o| o.str("scenario").unwrap()).collect();
        assert_eq!(skips, vec!["startup", "sync.idle", "list.page"]);
    }
}
