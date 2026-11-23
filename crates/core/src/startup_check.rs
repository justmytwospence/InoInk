//! The app's half of the start-up self-check (the launcher checks the installed files first, see
//! contrib/inoink/inoink-dispatch.sh): the device check, and the launcher's own notice.

use std::env;
use std::path::Path;
use inoink::device_check::{take_notice, Device};
use crate::device::CURRENT_DEVICE;

/// Logs how this model and firmware compare with the tested ones, and returns the notice page for
/// an untested combination the first time InoInk starts on it.
pub fn device_notice(data_dir: &Path) -> Option<String> {
    let model = CURRENT_DEVICE.model.to_string();
    let codename = env::var("PRODUCT").unwrap_or_default();
    // INOINK_CHECK_FIRMWARE (exported from data/config.sh) tries the check as another firmware.
    let firmware = env::var("INOINK_CHECK_FIRMWARE").or_else(|_| env::var("FIRMWARE_VERSION")).unwrap_or_default();
    let device = Device { model: &model, codename: &codename, tier: CURRENT_DEVICE.tier(), firmware: &firmware };
    eprintln!("device check: {}", device.summary());
    take_notice(data_dir, &device)
}

/// The launcher's notice, set when it started the previous release because the active one is
/// damaged.
pub fn launcher_notice() -> Option<String> {
    env::var("INOINK_NOTICE").ok().map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}
