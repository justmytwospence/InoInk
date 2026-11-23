//! inoink-bench: times InoInk on a Kobo (page turns, opening the list and articles, memory, a sync
//! with nothing new) by sending it input the way the hardware does and watching the framebuffer.
//! See doc/INOINK-BENCHMARKS.md. Inspired by Kobalt's device benchmarks
//! (https://github.com/nicoverbruggen/kobalt, MIT).
// The framebuffer reader is Linux only; elsewhere only the tests and `report` run.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod bench;
mod config;
mod input;
mod json;
mod report;
mod screen;

use std::cell::OnceCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bench::{Bench, Probe, Sink, Stop, SystemClock};
use config::Config;
use input::{Model, TouchMap};
use json::Obj;

const USAGE: &str = "Usage:
  inoink-bench run [--config FILE] [--out DIR] [--root DIR]
      on the Kobo, next to InoInk (started by bench.sh); results go to DIR (default ROOT/bench/results)
  inoink-bench report FILE...
      a Markdown summary of result files, for doc/INOINK-ACCEPTANCE.md
  inoink-bench defaults PRODUCT
      the default settings for a model, as a bench.conf";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("report") if args.len() > 1 => report(&args[1..]),
        Some("defaults") if args.len() == 2 => defaults(&args[1]),
        Some("--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("inoink-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn report(files: &[String]) -> Result<(), String> {
    let texts = files
        .iter()
        .map(|f| std::fs::read_to_string(f).map(|t| (f.clone(), t)).map_err(|e| format!("{f}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    print!("{}", report::report(&texts)?);
    Ok(())
}

fn model_for(product: &str) -> Result<Model, String> {
    input::model(product).ok_or_else(|| format!("no touch layout for the Kobo model \"{product}\"; supported: the Libra Colour and other recent models (doc/INOINK-BENCHMARKS.md)"))
}

fn defaults(product: &str) -> Result<(), String> {
    let m = model_for(product)?;
    let c = Config::defaults(m.dims.0, m.dims.1, m.dpi);
    let p = |p: (u32, u32)| format!("{},{}", p.0, p.1);
    println!("# inoink-bench settings for the {} in portrait. Remove a line to use its default.", m.name);
    println!("scenarios = {}", c.scenarios.join(","));
    println!("samples = {}\npage_turns = {}\nsync_samples = {}", c.samples, c.page_turns, c.sync_samples);
    println!("quiet_ms = {}\ntimeout_ms = {}\nsync_timeout_ms = {}", c.quiet_ms, c.timeout_ms, c.sync_timeout_ms);
    println!("startup_timeout_ms = {}\nstartup_quiet_ms = {}\ncell = {}", c.startup_timeout_ms, c.startup_quiet_ms, c.cell);
    println!("tap_article = {}\ntap_sync = {}\nsync_icon = {}", p(c.tap_article), p(c.tap_sync), c.sync_icon);
    println!("tap_bars = {}\ntap_back = {}", p(c.tap_bars), p(c.tap_back));
    println!("ignore = {}", c.ignore.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(";"));
    Ok(())
}

/// UTC date and time without a time crate: days since 1970 to a civil date.
fn utc(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The app's own `startup:` lines from its latest launch in `log`.
pub fn parse_startup(log: &str) -> Result<Option<Vec<(String, u64)>>, String> {
    let launch = log.rfind(" starting in ").map(|i| log[..i].rfind('\n').map_or(0, |n| n + 1));
    let Some(from) = launch else {
        return if log.contains("already running") {
            Err("InoInk was already running; quit it and start the benchmark again".into())
        } else {
            Ok(None)
        };
    };
    let mut steps = Vec::new();
    let mut done = false;
    for line in log[from..].lines() {
        if let Some(rest) = line.strip_prefix("startup: ") {
            if let Some((step, at)) = rest.rsplit_once(" at ") {
                if let Some(ms) = at.strip_suffix(" ms").and_then(|v| v.parse().ok()) {
                    done |= step == "first list frame sent";
                    steps.push((step.to_string(), ms));
                }
            }
        }
    }
    Ok(done.then_some(steps))
}

struct System {
    log: PathBuf,
    offset: u64,
    iface: String,
}

impl System {
    fn pid() -> Option<u32> {
        std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            (read_trim(e.path().join("comm"))? == "inoink").then_some(pid)
        })
    }
}

fn status_kib(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|l| l.strip_prefix(key)?.trim().strip_suffix("kB")?.trim().parse().ok())
}

impl Probe for System {
    fn memory(&mut self) -> Option<(u64, u64)> {
        let status = std::fs::read_to_string(format!("/proc/{}/status", Self::pid()?)).ok()?;
        Some((status_kib(&status, "VmRSS:")?, status_kib(&status, "VmHWM:")?))
    }
    fn wifi_up(&mut self) -> bool {
        read_trim(format!("/sys/class/net/{}/operstate", self.iface)).as_deref() == Some("up")
    }
    fn net_bytes(&mut self) -> Option<(u64, u64)> {
        let dev = std::fs::read_to_string("/proc/net/dev").ok()?;
        dev.lines().find_map(|l| {
            let (name, rest) = l.split_once(':')?;
            if name.trim() != self.iface {
                return None;
            }
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some((*f.first()?, *f.get(8)?))
        })
    }
    fn startup(&mut self) -> Result<Option<Vec<(String, u64)>>, String> {
        let Ok(mut f) = File::open(&self.log) else {
            return Ok(None);
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        // The launcher starts a new log when the old one grows large.
        if len < self.offset {
            self.offset = 0;
        }
        let mut text = String::new();
        f.seek(SeekFrom::Start(self.offset)).and_then(|_| f.read_to_string(&mut text)).map_err(|e| e.to_string())?;
        parse_startup(&text)
    }
}

/// The screen's rotation and touch map, read once InoInk is up (it sets the rotation as it
/// starts) and shared by the screen and the input.
type Geometry = Rc<OnceCell<TouchMap>>;

fn geometry<'a>(cell: &'a Geometry, model: &Model) -> &'a TouchMap {
    cell.get_or_init(|| {
        let rotate = read_trim("/sys/class/graphics/fb0/rotate").and_then(|r| r.parse().ok()).unwrap_or(model.startup_rotation);
        TouchMap::new(model, model.rotation(rotate))
    })
}

struct LazyScreen {
    model: &'static Model,
    geometry: Geometry,
    #[cfg(target_os = "linux")]
    fb: Option<screen::Framebuffer>,
}

impl screen::Screen for LazyScreen {
    fn size(&self) -> (u32, u32) {
        let map = geometry(&self.geometry, self.model);
        (map.width, map.height)
    }
    #[cfg(target_os = "linux")]
    fn signature(&mut self, region: screen::Rect, cell: u32) -> io::Result<screen::Signature> {
        if self.fb.is_none() {
            let (w, h) = self.size();
            self.fb = Some(screen::Framebuffer::open(w, h)?);
        }
        self.fb.as_mut().unwrap().signature(region, cell)
    }
    #[cfg(not(target_os = "linux"))]
    fn signature(&mut self, _: screen::Rect, _: u32) -> io::Result<screen::Signature> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "runs on a Kobo only"))
    }
}

/// Input devices, opened on first use with the touch map of the screen at that time.
struct LazyInput {
    geometry: Geometry,
    model: &'static Model,
    devices: Option<input::Devices<File>>,
}

impl LazyInput {
    fn get(&mut self) -> io::Result<&mut input::Devices<File>> {
        if self.devices.is_none() {
            let missing = |what| io::Error::new(io::ErrorKind::NotFound, format!("no {what} input device"));
            let touch = input::first_existing(&input::TOUCH_INPUTS).ok_or_else(|| missing("touch"))?;
            let buttons = input::first_existing(&input::BUTTON_INPUTS).ok_or_else(|| missing("button"))?;
            let open = |p: &Path| OpenOptions::new().write(true).open(p);
            let map = *geometry(&self.geometry, self.model);
            self.devices = Some(input::Devices::new(open(&touch)?, open(&buttons)?, map));
        }
        Ok(self.devices.as_mut().unwrap())
    }
}

impl input::Input for LazyInput {
    fn key(&mut self, code: u16) -> io::Result<()> {
        self.get()?.key(code)
    }
    fn tap(&mut self, x: u32, y: u32) -> io::Result<()> {
        self.get()?.tap(x, y)
    }
}

/// Removes the lock when the run ends, however it ends.
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Results(File);
impl Sink for Results {
    fn line(&mut self, obj: Obj) -> io::Result<()> {
        use std::io::Write;
        writeln!(self.0, "{obj}")?;
        // On the Kobo's FAT storage: keep every line even if the device reboots.
        self.0.sync_data()
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut config = None;
    let mut out = None;
    let mut root = PathBuf::from("/mnt/onboard/.adds/inoink");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
        match a.as_str() {
            "--config" => config = Some(PathBuf::from(v)),
            "--out" => out = Some(PathBuf::from(v)),
            "--root" => root = PathBuf::from(v),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    let product = std::env::var("PRODUCT").map_err(|_| "PRODUCT is not set: start the benchmark from NickelMenu".to_string())?;
    let model: &'static Model = Box::leak(Box::new(model_for(&product)?));
    let mut cfg = Config::defaults(model.dims.0, model.dims.1, model.dpi);
    if let Some(path) = &config {
        if let Ok(text) = std::fs::read_to_string(path) {
            cfg = cfg.parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }

    let lock = PathBuf::from("/tmp/inoink-bench.lock");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|_| "a benchmark is already running (/tmp/inoink-bench.lock)".to_string())?;
    let _lock = Lock(lock);

    let log = root.join("data/inoink.log");
    let offset = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let mut probe = System { log, offset, iface: std::env::var("INTERFACE").unwrap_or_else(|_| "wlan0".into()) };

    let started = SystemTime::now();
    let release = read_trim(root.join("active-release")).unwrap_or_else(|| "unknown".into());
    let kobo = read_trim("/mnt/onboard/.kobo/version").unwrap_or_default();
    let field = |i: usize| kobo.split(',').nth(i).unwrap_or("?").trim().to_string();
    let dir = out.unwrap_or_else(|| root.join("bench/results"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp: String = utc(started).chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let path = dir.join(format!("{stamp}-{release}.jsonl"));
    let file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut results = Results(file);
    eprintln!("inoink-bench: writing {}", path.display());
    let header = Obj::new()
        .s("type", "header")
        .s("tool", "inoink-bench")
        .n("format", 1)
        .s("release", &release)
        .s("product", &product)
        .s("model", model.name)
        .s("model_number", field(5).trim_start_matches(['0', '-']))
        .s("firmware", field(2))
        .s("started", utc(started))
        .n("samples", cfg.samples)
        .n("page_turns", cfg.page_turns)
        .n("quiet_ms", cfg.quiet_ms as f64)
        .n("cell", cfg.cell)
        .s("scenarios", cfg.scenarios.join(","));
    results.line(header).map_err(|e| e.to_string())?;

    let shared: Geometry = Rc::new(OnceCell::new());
    let mut screen = LazyScreen {
        model,
        geometry: shared.clone(),
        #[cfg(target_os = "linux")]
        fb: None,
    };
    let mut input = LazyInput { geometry: shared, model, devices: None };
    let clock = SystemClock(std::time::Instant::now());
    let outcome = {
        let mut b = Bench { screen: &mut screen, input: &mut input, clock: &clock, probe: &mut probe, out: &mut results, cfg };
        b.run()
    };
    let end = match &outcome {
        Ok(()) => Obj::new().s("type", "end").b("ok", true),
        Err(e) => {
            let kind = if matches!(e, Stop::Io(_)) { "io" } else { "unexpected" };
            results.line(Obj::new().s("type", "error").s("kind", kind).s("message", e.to_string())).ok();
            Obj::new().s("type", "end").b("ok", false)
        }
    };
    let elapsed = SystemTime::now().duration_since(started).unwrap_or(Duration::ZERO);
    results.line(end.n("seconds", elapsed.as_secs() as f64)).map_err(|e| e.to_string())?;
    outcome.map_err(|e| format!("stopped: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(utc(UNIX_EPOCH + Duration::from_secs(951_782_400 + 3661)), "2000-02-29T01:01:01Z");
        assert_eq!(utc(UNIX_EPOCH + Duration::from_secs(1_798_761_599)), "2026-12-31T23:59:59Z");
    }

    #[test]
    fn startup_lines_of_the_latest_launch() {
        let log =
            "--- Mon starting in /a\nstartup: list built at 900 ms\nstartup: first list frame sent at 1000 ms\n--- exited with status 0\n\
                   --- Tue starting in /b\nlaunch: x\nstartup: opening screen shown at 120 ms\nstartup: database open at 300 ms\n";
        assert_eq!(parse_startup(log), Ok(None));
        let log = format!("{log}noise\nstartup: first list frame sent at 2100 ms\n");
        assert_eq!(
            parse_startup(&log),
            Ok(Some(vec![("opening screen shown".into(), 120), ("database open".into(), 300), ("first list frame sent".into(), 2100)]))
        );
        assert_eq!(parse_startup(""), Ok(None));
        assert!(parse_startup("--- Wed already running (pid 4); launch ignored\n").is_err());
    }

    #[test]
    fn memory_lines() {
        let status = "Name:\tinoink\nVmHWM:\t   40960 kB\nVmRSS:\t   31744 kB\n";
        assert_eq!(status_kib(status, "VmRSS:"), Some(31744));
        assert_eq!(status_kib(status, "VmHWM:"), Some(40960));
        assert_eq!(status_kib(status, "VmSwap:"), None);
    }
}
