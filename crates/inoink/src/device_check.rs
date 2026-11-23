//! The start-up device check: has InoInk been tested on this model and firmware? On an untested
//! combination the app shows a notice once, naming the model and firmware and how to report how
//! it went. Nothing is ever blocked.
//!
//! [`TESTED`] is the one list of tested devices; README > Status says the same, and a test keeps the
//! two in step. Which models are expected to work or experimental is decided by the device code
//! (`plato_core::device`), which passes its verdict in as a [`Tier`].

use std::fs;
use std::path::Path;

/// Model (as `plato_core::device::Model` names it) and firmware pairs InoInk has been run on, from
/// device reports. Kobo-branded devices only: a Tolino counts as a model of its own.
pub const TESTED: &[(&str, &str)] = &[("Libra Colour", "4.46.23852")];

/// Where the app remembers the last model and firmware it gave notice of, in the data directory.
pub const NOTICE_FILE: &str = "device-notice";

const REPORT: &str = "github.com/justmytwospence/InoInk/issues";

/// What the device code says about a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Linux 4.x or newer: expected to work.
    Expected,
    /// A Linux kernel older than the toolchain's minimum: InoInk may not start.
    Experimental,
    /// A codename this build does not know.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// The model and firmware are in [`TESTED`].
    Tested,
    /// The model was tested, on other firmware.
    UntestedFirmware,
    /// The model was never tested.
    UntestedModel,
    /// The firmware version could not be read (not a Kobo, or `.kobo/version` is missing).
    FirmwareUnknown,
}

/// The device InoInk runs on, as the launcher and the device code describe it.
#[derive(Clone, Copy, Debug)]
pub struct Device<'a> {
    /// The model name, e.g. "Libra Colour".
    pub model: &'a str,
    /// The codename (`PRODUCT`), e.g. "monza" or "monzaTolino".
    pub codename: &'a str,
    pub tier: Tier,
    /// `FIRMWARE_VERSION`, e.g. "4.46.23852".
    pub firmware: &'a str,
}

impl Device<'_> {
    fn firmware(&self) -> &str {
        self.firmware.trim()
    }

    fn is_tolino(&self) -> bool {
        self.codename.to_ascii_lowercase().contains("tolino")
    }

    pub fn support(&self) -> Support {
        let firmware = self.firmware();
        if firmware.is_empty() {
            return Support::FirmwareUnknown;
        }
        let kobo = self.tier != Tier::Unknown && !self.is_tolino();
        let model_tested = kobo && TESTED.iter().any(|(m, _)| *m == self.model);
        if !model_tested {
            Support::UntestedModel
        } else if TESTED.iter().any(|(m, f)| *m == self.model && *f == firmware) {
            Support::Tested
        } else {
            Support::UntestedFirmware
        }
    }

    /// A line for the log.
    pub fn summary(&self) -> String {
        let verdict = match self.support() {
            Support::Tested => "tested",
            Support::UntestedFirmware => "untested firmware",
            Support::UntestedModel => "untested model",
            Support::FirmwareUnknown => "firmware unknown",
        };
        let tier = match self.tier {
            Tier::Expected => "",
            Tier::Experimental => ", experimental model",
            Tier::Unknown => ", unknown model",
        };
        format!("{} ({}) firmware {}: {verdict}{tier}", self.model, self.codename, self.firmware())
    }

    /// Firmware 5 and newer: the official NickelMenu does not run there, and firmware updates
    /// replace the system, NickelMenu included.
    fn new_firmware(&self) -> bool {
        self.firmware().split('.').next().and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| m >= 5)
    }

    /// The notice for an untested model or firmware, as an HTML page; `None` when there is nothing
    /// to say.
    pub fn notice_html(&self) -> Option<String> {
        let support = self.support();
        if matches!(support, Support::Tested | Support::FirmwareUnknown) {
            return None;
        }
        let model = escape(self.model);
        let firmware = escape(self.firmware());
        let title = if support == Support::UntestedFirmware { "Untested firmware" } else { "Untested e-reader" };
        let mut body = String::new();
        let situation = match (support, self.tier) {
            (_, Tier::Unknown) => format!(
                "InoInk does not know this e-reader (codename {}, firmware {firmware}). It treats it as a \
                 recent 300&#160;dpi Kobo, which may not match its screen or buttons.",
                escape(self.codename)
            ),
            (Support::UntestedFirmware, _) => {
                let tested: Vec<&str> = TESTED.iter().filter(|(m, _)| *m == self.model).map(|(_, f)| *f).collect();
                format!(
                    "InoInk has been tested on the Kobo {model} with firmware {}, but not yet with firmware \
                     {firmware}, which this Kobo runs.",
                    tested.join(", ")
                )
            }
            (_, Tier::Experimental) => format!(
                "InoInk has not been tested on the Kobo {model} (firmware {firmware}). This model is \
                 experimental: its Linux is older than the minimum InoInk is built for, so parts of InoInk \
                 may not work."
            ),
            _ if self.is_tolino() => format!(
                "InoInk has not been tested on a Tolino yet. This one is built like the Kobo {model} and \
                 runs firmware {firmware}."
            ),
            _ => format!(
                "InoInk has not been tested on the Kobo {model} yet (firmware {firmware}). It uses the same \
                 code as on the tested models."
            ),
        };
        body.push_str(&format!("<p>{situation}</p>\n"));
        if self.tier == Tier::Expected && !self.new_firmware() {
            body.push_str("<p>It is expected to work.</p>\n");
        }
        if self.new_firmware() {
            body.push_str(
                "<p>Firmware 5 and newer is experimental for InoInk. A firmware update there replaces the \
                 system, NickelMenu and the InoInk menu entry included: see \u{201c}After a Kobo firmware \
                 update\u{201d} in InoInk's setup guide.</p>\n",
            );
        }
        body.push_str(&format!(
            "<p><b>Please report how it goes</b>, working or not. On a computer, open {REPORT}, choose \
             New issue, then <i>Device report</i>, and give the model and firmware above (Main menu, \
             Status, Device shows them again). If something went wrong, attach \
             <tt>.adds/inoink/data/inoink.log</tt> from the Kobo.</p>\n\
             <p>This note shows once for this model and firmware. To continue to your articles, tap \
             the middle of the screen, then the arrow at the top left.</p>\n"
        ));
        Some(format!("<html>\n<head><title>{title}</title></head>\n<body>\n<h2>{title}</h2>\n{body}</body>\n</html>\n"))
    }
}

/// The notice for this device, the first time InoInk starts on this model and firmware. Remembers
/// that it was given in `data_dir`; a notice that cannot be remembered is still given.
pub fn take_notice(data_dir: &Path, device: &Device) -> Option<String> {
    let html = device.notice_html()?;
    let path = data_dir.join(NOTICE_FILE);
    let key = format!("{} {} {}", device.codename, device.model, device.firmware());
    if fs::read_to_string(&path).is_ok_and(|seen| seen.trim() == key) {
        return None;
    }
    if let Err(e) = fs::write(&path, format!("{key}\n")) {
        eprintln!("device check: cannot remember the notice in {}: {e}", path.display());
    }
    Some(html)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device<'a>(model: &'a str, codename: &'a str, tier: Tier, firmware: &'a str) -> Device<'a> {
        Device { model, codename, tier, firmware }
    }

    #[test]
    fn support_levels() {
        let (m, f) = TESTED[0];
        assert_eq!(device(m, "monza", Tier::Expected, f).support(), Support::Tested);
        assert_eq!(device(m, "monza", Tier::Expected, &format!(" {f}\n")).support(), Support::Tested);
        assert_eq!(device(m, "monza", Tier::Expected, "4.99.1").support(), Support::UntestedFirmware);
        assert_eq!(device(m, "monzaTolino", Tier::Expected, f).support(), Support::UntestedModel);
        assert_eq!(device("Nia", "luna", Tier::Expected, f).support(), Support::UntestedModel);
        assert_eq!(device("Unknown Kobo", "futurebird", Tier::Unknown, f).support(), Support::UntestedModel);
        assert_eq!(device(m, "monza", Tier::Expected, "").support(), Support::FirmwareUnknown);
    }

    #[test]
    fn notices_name_model_firmware_and_how_to_report() {
        let (m, f) = TESTED[0];
        assert_eq!(device(m, "monza", Tier::Expected, f).notice_html(), None);
        assert_eq!(device(m, "monza", Tier::Expected, " ").notice_html(), None);

        let html = device(m, "monza", Tier::Expected, "4.99.1").notice_html().unwrap();
        assert!(html.contains("<h2>Untested firmware</h2>"));
        assert!(html.contains(&format!("with firmware {f}, but not yet with firmware 4.99.1")));
        assert!(html.contains(REPORT) && html.contains("Device report"));
        assert!(!html.contains("Firmware 5 and newer") && html.contains("expected to work"));
        assert!(html.contains("arrow at the top left"));

        let html = device("Nia", "luna", Tier::Expected, "4.38.21908").notice_html().unwrap();
        assert!(html.contains("<h2>Untested e-reader</h2>") && html.contains("Kobo Nia yet (firmware 4.38.21908)"));

        let html = device("Glo HD", "alyssum", Tier::Experimental, "4.38.21908").notice_html().unwrap();
        assert!(html.contains("This model is experimental") && !html.contains("expected to work"));

        let html = device(m, "monzaTolino", Tier::Expected, "5.18.264769").notice_html().unwrap();
        assert!(html.contains("on a Tolino") && html.contains("Firmware 5 and newer") && !html.contains("expected to work"));

        let html = device("Unknown Kobo", "fut<ure>", Tier::Unknown, "6.0.279225").notice_html().unwrap();
        assert!(html.contains("codename fut&lt;ure&gt;, firmware 6.0.279225") && html.contains("Firmware 5 and newer"));
    }

    #[test]
    fn notice_is_given_once_per_model_and_firmware() {
        let dir = tempfile::tempdir().unwrap();
        let first = device("Nia", "luna", Tier::Expected, "4.38.21908");
        assert!(take_notice(dir.path(), &first).is_some());
        assert!(take_notice(dir.path(), &first).is_none());
        // A firmware update brings it back.
        let updated = device("Nia", "luna", Tier::Expected, "4.39.22801");
        assert!(take_notice(dir.path(), &updated).is_some());
        assert!(take_notice(dir.path(), &updated).is_none());
        // A tested device never gets one and does not forget the last notice.
        let (m, f) = TESTED[0];
        assert!(take_notice(dir.path(), &device(m, "monza", Tier::Expected, f)).is_none());
        assert!(take_notice(dir.path(), &updated).is_none());
        // Unwritable data directory: the notice is still given.
        assert!(take_notice(&dir.path().join("missing"), &first).is_some());
    }

    /// README > Status names every tested model and firmware, and no others.
    #[test]
    fn readme_lists_the_tested_devices() {
        let readme = include_str!("../../../README.md");
        let line = readme.lines().find(|l| l.starts_with("- **Tested:**")).expect("README Status has a Tested line");
        for (model, firmware) in TESTED {
            assert!(line.contains(&format!("{model} on firmware {firmware}")), "README Tested line lacks {model} {firmware}: {line}");
        }
        let named = line.matches(" on firmware ").count();
        assert_eq!(named, TESTED.len(), "README Tested line names devices not in device_check::TESTED: {line}");
    }
}
