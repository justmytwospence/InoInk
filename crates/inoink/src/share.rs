//! Sending links to other services (Karakeep for now). Shares wait in a local outbox and are
//! sent whenever the network is up, independently of the Inoreader quota. The service address
//! and key are provisioned from the computer, encrypted to the Kobo's key, like the Inoreader
//! sign-in, and never appear in logs.

use crate::api::{Body, HttpRequest, Method, Transport};
use crate::cache::atomic_write;
use crate::credentials::{decrypt_with_device_key, restrict};
use crate::error::{Error, Redacted, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

pub const KARAKEEP: &str = "karakeep";
const SHARE_FILE: &str = "share.json";
const SHARE_TRANSFER: &str = "provision/share.age";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct KarakeepConfig {
    pub url: String,
    pub api_key: String,
}

impl fmt::Debug for KarakeepConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KarakeepConfig").field("url", &self.url).field("api_key", &Redacted(&self.api_key)).finish()
    }
}

/// The services links can be sent to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareConfig {
    #[serde(default)]
    pub karakeep: Option<KarakeepConfig>,
}

pub fn load_config(data_dir: &Path) -> Result<ShareConfig> {
    match std::fs::read(data_dir.join(SHARE_FILE)) {
        Ok(b) => serde_json::from_slice(&b).map_err(|_| Error::Data("the share settings file is damaged".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ShareConfig::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save_config(data_dir: &Path, cfg: &ShareConfig) -> Result<()> {
    let path = data_dir.join(SHARE_FILE);
    atomic_write(&path, &serde_json::to_vec(cfg)?)?;
    restrict(&path);
    Ok(())
}

/// Computer side: encrypts the share settings for a device's public key.
pub fn encrypt_for(recipient: &str, cfg: &ShareConfig) -> Result<Vec<u8>> {
    let r = age::x25519::Recipient::from_str(recipient.trim()).map_err(|_| Error::InvalidRequest("not a device public key".into()))?;
    age::encrypt(&r, &serde_json::to_vec(cfg)?).map_err(|e| Error::Data(format!("encryption failed: {e}")))
}

/// Computer side: writes the encrypted transfer file for the device to import.
pub fn write_transfer(data_dir: &Path, encrypted: &[u8]) -> Result<()> {
    atomic_write(&data_dir.join(SHARE_TRANSFER), encrypted)
}

/// Device side: imports and deletes a pending transfer file. A service missing from the file
/// keeps its existing settings.
pub fn import_provisioned(data_dir: &Path) -> Result<bool> {
    let transfer = data_dir.join(SHARE_TRANSFER);
    let bytes = match std::fs::read(&transfer) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let result = (|| {
        let plain = decrypt_with_device_key(data_dir, &bytes)?;
        let incoming: ShareConfig = serde_json::from_slice(&plain).map_err(|_| Error::Data("the share transfer file is damaged".into()))?;
        let mut cfg = load_config(data_dir).unwrap_or_default();
        if incoming.karakeep.is_some() {
            cfg.karakeep = incoming.karakeep;
        }
        save_config(data_dir, &cfg)
    })();
    let _ = std::fs::remove_file(&transfer);
    result.map(|_| true)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShareOutcome {
    Sent,
    /// A transient problem: try again later.
    Retry(String),
    /// Refused for good: tell the user.
    Failed(String),
}

pub trait Sharer: Send + Sync {
    fn share(&self, target: &str, url: &str, title: &str) -> ShareOutcome;
}

/// Sends links over HTTP.
pub struct HttpSharer {
    cfg: ShareConfig,
    transport: Arc<dyn Transport>,
}

impl HttpSharer {
    pub fn new(cfg: ShareConfig, transport: Arc<dyn Transport>) -> HttpSharer {
        HttpSharer { cfg, transport }
    }

    fn karakeep(&self, k: &KarakeepConfig, url: &str, title: &str) -> ShareOutcome {
        let mut body = serde_json::json!({"type": "link", "url": url});
        if !title.trim().is_empty() {
            body["title"] = serde_json::Value::String(title.trim().to_string());
        }
        let req = HttpRequest {
            method: Method::Post,
            url: format!("{}/api/v1/bookmarks", k.url.trim_end_matches('/')),
            headers: vec![("Authorization".into(), format!("Bearer {}", k.api_key)), ("Accept".into(), "application/json".into())],
            body: Some(Body::Json(body.to_string())),
        };
        match self.transport.send(&req) {
            // 201 when created, 200 when Karakeep already had the link.
            Ok(r) if r.status == 200 || r.status == 201 => ShareOutcome::Sent,
            Ok(r) if r.status == 401 || r.status == 403 => ShareOutcome::Failed("Karakeep refused the API key".into()),
            Ok(r) if (400..500).contains(&r.status) => ShareOutcome::Failed(format!("Karakeep refused the link (HTTP {})", r.status)),
            Ok(r) => ShareOutcome::Retry(format!("Karakeep answered HTTP {}", r.status)),
            Err(e) => ShareOutcome::Retry(e.to_string()),
        }
    }
}

impl Sharer for HttpSharer {
    fn share(&self, target: &str, url: &str, title: &str) -> ShareOutcome {
        match (target, &self.cfg.karakeep) {
            (KARAKEEP, Some(k)) => self.karakeep(k, url, title),
            _ => ShareOutcome::Failed("this share target is not set up".into()),
        }
    }
}
