//! The services InoInk signs in to, and how their settings reach the Kobo.
//!
//! The computer (`inoink-cli setup`), the browser installer and the Kobo's own setup page all
//! produce the same thing: a `provision/accounts.json` transfer file with one optional entry per
//! service. The Kobo imports it as InoInk starts: the Inoreader sign-in goes to
//! `credentials.json`, Karakeep to `share.json`, and the transfer file is deleted. The files are
//! plain: the Kobo's FAT storage has no permissions, so anyone holding the device can read them.

use crate::api::auth::{exchange_code, AppConfig};
use crate::api::{ApiClient, Clock, TokenStore, Tokens, Transport};
use crate::cache::atomic_write;
use crate::credentials::{Credentials, FileCredentials};
use crate::error::{Error, Result};
use crate::share::{self, KarakeepConfig};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const TRANSFER: &str = "provision/accounts.json";
/// The redirect page on the project site that hands the sign-in back to the computer or the Kobo.
pub const RELAY_REDIRECT_URI: &str = "https://justmytwospence.github.io/InoInk/callback.html";
pub const LOCAL_REDIRECT_URI: &str = "http://localhost:8080/callback";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Inoreader,
    Karakeep,
}

/// One setting of a service, as a form field.
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub hint: &'static str,
}

const INOREADER_FIELDS: [Field; 3] = [
    Field { key: "client_id", label: "App ID", secret: false, hint: "From Inoreader's Preferences > Developer API" },
    Field { key: "client_secret", label: "App key", secret: true, hint: "" },
    Field { key: "redirect_uri", label: "Redirect URI", secret: false, hint: "Exactly as registered in Inoreader" },
];

const KARAKEEP_FIELDS: [Field; 2] = [
    Field { key: "url", label: "Address", secret: false, hint: "https://..." },
    Field { key: "api_key", label: "API key", secret: true, hint: "Karakeep > Settings > API Keys" },
];

impl Service {
    pub const ALL: [Service; 2] = [Service::Inoreader, Service::Karakeep];

    pub fn id(self) -> &'static str {
        match self {
            Service::Inoreader => "inoreader",
            Service::Karakeep => "karakeep",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Service::Inoreader => "Inoreader",
            Service::Karakeep => "Karakeep",
        }
    }

    pub fn parse(s: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|v| v.id().eq_ignore_ascii_case(s.trim()))
    }

    pub fn fields(self) -> &'static [Field] {
        match self {
            Service::Inoreader => &INOREADER_FIELDS,
            Service::Karakeep => &KARAKEEP_FIELDS,
        }
    }
}

/// The transfer file. Unknown services are ignored, so a newer computer can name future ones.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transfer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inoreader: Option<Credentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub karakeep: Option<KarakeepConfig>,
}

impl Transfer {
    pub fn is_empty(&self) -> bool {
        self.inoreader.is_none() && self.karakeep.is_none()
    }
}

pub fn read_transfer(data_dir: &Path) -> Result<Option<Transfer>> {
    match std::fs::read(data_dir.join(TRANSFER)) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|_| Error::Data("the accounts file is damaged".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes the transfer file for the Kobo to import, merged into one that is still waiting.
pub fn write_transfer(data_dir: &Path, t: &Transfer) -> Result<()> {
    let mut merged = read_transfer(data_dir).ok().flatten().unwrap_or_default();
    if t.inoreader.is_some() {
        merged.inoreader = t.inoreader.clone();
    }
    if t.karakeep.is_some() {
        merged.karakeep = t.karakeep.clone();
    }
    let path = data_dir.join(TRANSFER);
    atomic_write(&path, &serde_json::to_vec_pretty(&merged)?)?;
    crate::credentials::restrict(&path);
    Ok(())
}

/// Kobo side: imports and deletes a waiting transfer file. Returns the services it set up. Files
/// from older versions (an encryption key and `.age` transfers) are deleted.
pub fn import_provisioned(data_dir: &Path) -> Result<Vec<Service>> {
    let _ = std::fs::remove_file(data_dir.join("device.key"));
    let _ = std::fs::remove_file(data_dir.join("provision/device.pub"));
    let mut legacy = false;
    for old in ["provision/credentials.age", "provision/share.age"] {
        if std::fs::remove_file(data_dir.join(old)).is_ok() {
            legacy = true;
        }
    }
    let path = data_dir.join(TRANSFER);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return if legacy {
                Err(Error::Data("a sign-in file from an older inoink-cli was deleted; run the installer's sign-in again".into()))
            } else {
                Ok(Vec::new())
            };
        }
        Err(e) => return Err(e.into()),
    };
    let result = (|| {
        let t: Transfer = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Data("the accounts file was damaged and was deleted; set up the accounts again".into()))?;
        let mut done = Vec::new();
        if let Some(c) = &t.inoreader {
            FileCredentials::new(data_dir).save_credentials(c)?;
            done.push(Service::Inoreader);
        }
        if let Some(k) = t.karakeep {
            share::set_karakeep(data_dir, Some(k))?;
            done.push(Service::Karakeep);
        }
        Ok(done)
    })();
    let _ = std::fs::remove_file(&path);
    result
}

struct Memory(Mutex<Option<Tokens>>);
impl TokenStore for Memory {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, t: &Tokens) -> Result<()> {
        *self.0.lock().unwrap() = Some(t.clone());
        Ok(())
    }
}

/// Trades an authorization code for tokens and learns whose account it is.
pub fn sign_in_inoreader(cfg: &AppConfig, transport: Arc<dyn Transport>, clock: Arc<dyn Clock>, code: &str) -> Result<Credentials> {
    let tokens = exchange_code(cfg, transport.as_ref(), clock.as_ref(), code)?;
    let api = ApiClient::new(cfg.clone(), transport, clock, Arc::new(Memory(Mutex::new(Some(tokens.clone())))))?;
    let info = api.user_info()?;
    let mut creds = Credentials::new(cfg, &tokens);
    creds.user_id = info.user_id;
    creds.user_name = info.user_name;
    Ok(creds)
}

/// The `state` of a sign-in whose redirect page should hand the result to a local address.
pub fn relay_state(nonce: &str, host: &str, port: u16) -> String {
    format!("{nonce}_{host}_{port}")
}

pub fn sign_out(data_dir: &Path, s: Service) -> Result<()> {
    match s {
        Service::Inoreader => FileCredentials::new(data_dir).clear(),
        Service::Karakeep => share::set_karakeep(data_dir, None),
    }
}

/// A short state line for the Accounts screen, or `None` when the service is not set up.
pub fn describe(data_dir: &Path, s: Service) -> Option<String> {
    match s {
        Service::Inoreader => {
            let c = FileCredentials::new(data_dir).load_credentials().ok().flatten()?;
            Some(if !c.user_name.is_empty() {
                format!("Signed in as {}", c.user_name)
            } else if !c.user_id.is_empty() {
                format!("Signed in (account {})", c.user_id)
            } else {
                "Signed in".to_string()
            })
        }
        Service::Karakeep => {
            let k = share::load_config(data_dir).ok()?.karakeep?;
            let host = url::Url::parse(&k.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or(k.url);
            Some(format!("Sends to {host}"))
        }
    }
}
