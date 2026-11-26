//! Sending links to other services (Karakeep for now). Shares wait in a local outbox and are
//! sent whenever the network is up, independently of the Inoreader quota. The service address
//! and key are written to `share.json` by `accounts` (from the computer, the browser installer or
//! the Kobo's setup page) and never appear in logs.

use crate::api::{Body, HttpRequest, Method, Transport};
use crate::cache::atomic_write;
use crate::credentials::restrict;
use crate::error::{Error, Redacted, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

pub const KARAKEEP: &str = "karakeep";
const SHARE_FILE: &str = "share.json";

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

/// A Karakeep address without its trailing slash. Only web addresses are accepted.
pub fn karakeep_base(url: &str) -> Result<String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(Error::InvalidRequest("the Karakeep address must start with https:// (or http://)".into()));
    }
    Ok(url.trim_end_matches('/').to_string())
}

/// Sets or removes the Karakeep settings, keeping the other services.
pub fn set_karakeep(data_dir: &Path, k: Option<KarakeepConfig>) -> Result<()> {
    let mut cfg = load_config(data_dir).unwrap_or_default();
    cfg.karakeep = k;
    save_config(data_dir, &cfg)
}

#[derive(Debug, Clone, PartialEq)]
pub enum KarakeepCheck {
    Accepted,
    Refused,
    Unverified(String),
}

/// Asks Karakeep who the key belongs to.
pub fn check_karakeep(k: &KarakeepConfig, transport: &dyn Transport) -> KarakeepCheck {
    let probe = HttpRequest {
        method: Method::Get,
        url: format!("{}/api/v1/users/me", k.url.trim_end_matches('/')),
        headers: vec![("Authorization".into(), format!("Bearer {}", k.api_key)), ("Accept".into(), "application/json".into())],
        body: None,
    };
    match transport.send(&probe) {
        Ok(r) if r.status == 200 => KarakeepCheck::Accepted,
        Ok(r) if r.status == 401 || r.status == 403 => KarakeepCheck::Refused,
        Ok(r) => KarakeepCheck::Unverified(format!("HTTP {}", r.status)),
        Err(e) => KarakeepCheck::Unverified(e.to_string()),
    }
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
