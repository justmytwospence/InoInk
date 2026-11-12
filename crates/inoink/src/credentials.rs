//! Credential storage and computer-to-Kobo provisioning.
//!
//! The Kobo generates an age (X25519) key pair; its public key is exported for the computer.
//! The `inoink-cli` helper performs OAuth on the computer and writes an age-encrypted
//! credentials file for that key; the device imports it and deletes the transfer file.
//! Tokens live in a file outside the database and outside backups. Kobo's onboard FAT storage
//! has no meaningful file permissions, so this protects against transfer exposure and casual
//! browsing only, not against someone with physical access to the device.

use crate::api::auth::{AppConfig, TokenStore, Tokens};
use crate::cache::atomic_write;
use crate::error::{Error, Redacted, Result};
use age::secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

const DEVICE_KEY: &str = "device.key";
const DEVICE_PUB: &str = "provision/device.pub";
const TRANSFER: &str = "provision/credentials.age";
const CREDENTIALS: &str = "credentials.json";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub scope: String,
    /// Inoreader user id; keys the database so accounts never mix.
    #[serde(default)]
    pub user_id: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &Redacted(&self.client_secret))
            .field("access_token", &Redacted(&self.access_token))
            .field("refresh_token", &Redacted(&self.refresh_token))
            .finish()
    }
}

impl Credentials {
    pub fn new(cfg: &AppConfig, t: &Tokens) -> Credentials {
        Credentials {
            client_id: cfg.client_id.clone(),
            client_secret: cfg.client_secret.clone(),
            redirect_uri: cfg.redirect_uri.clone(),
            access_token: t.access_token.clone(),
            refresh_token: t.refresh_token.clone(),
            expires_at: t.expires_at,
            scope: t.scope.clone(),
            user_id: String::new(),
        }
    }

    pub fn app_config(&self) -> AppConfig {
        AppConfig::new(&self.client_id, &self.client_secret, &self.redirect_uri)
    }

    pub fn tokens(&self) -> Tokens {
        Tokens {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            expires_at: self.expires_at,
            scope: self.scope.clone(),
        }
    }
}

/// Token store backed by `<data>/credentials.json`, written atomically.
pub struct FileCredentials {
    path: PathBuf,
    lock: Mutex<()>,
}

pub(crate) fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

impl FileCredentials {
    pub fn new(data_dir: &Path) -> FileCredentials {
        FileCredentials { path: data_dir.join(CREDENTIALS), lock: Mutex::new(()) }
    }

    pub fn load_credentials(&self) -> Result<Option<Credentials>> {
        match std::fs::read(&self.path) {
            Ok(b) => Ok(Some(serde_json::from_slice(&b).map_err(|_| Error::Data("credentials file is damaged".into()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_credentials(&self, c: &Credentials) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        atomic_write(&self.path, &serde_json::to_vec(c)?)?;
        restrict(&self.path);
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let _ = std::fs::remove_file(&self.path);
        Ok(())
    }
}

impl TokenStore for FileCredentials {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(self.load_credentials()?.map(|c| c.tokens()))
    }

    /// Replaces only the token fields; the app credentials are kept.
    fn save(&self, t: &Tokens) -> Result<()> {
        let mut c = self.load_credentials()?.ok_or(Error::NeedsAuthorization)?;
        c.access_token = t.access_token.clone();
        c.refresh_token = t.refresh_token.clone();
        c.expires_at = t.expires_at;
        c.scope = t.scope.clone();
        self.save_credentials(&c)
    }
}

/// Creates the device key if needed and exports its public key. Returns the recipient string.
pub fn ensure_device_key(data_dir: &Path) -> Result<String> {
    let key_path = data_dir.join(DEVICE_KEY);
    let identity = match std::fs::read_to_string(&key_path) {
        Ok(s) => age::x25519::Identity::from_str(s.trim()).map_err(|_| Error::Data("device key is damaged".into()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = age::x25519::Identity::generate();
            atomic_write(&key_path, id.to_string().expose_secret().as_bytes())?;
            restrict(&key_path);
            id
        }
        Err(e) => return Err(e.into()),
    };
    let recipient = identity.to_public().to_string();
    atomic_write(&data_dir.join(DEVICE_PUB), recipient.as_bytes())?;
    Ok(recipient)
}

/// Computer side: encrypts credentials for a device's public key.
pub fn encrypt_for(recipient: &str, c: &Credentials) -> Result<Vec<u8>> {
    let r = age::x25519::Recipient::from_str(recipient.trim()).map_err(|_| Error::InvalidRequest("not a device public key".into()))?;
    age::encrypt(&r, &serde_json::to_vec(c)?).map_err(|e| Error::Data(format!("encryption failed: {e}")))
}

/// Computer side: reads the device public key from a mounted Kobo data directory.
pub fn read_device_recipient(data_dir: &Path) -> Result<String> {
    std::fs::read_to_string(data_dir.join(DEVICE_PUB))
        .map(|s| s.trim().to_string())
        .map_err(|_| Error::InvalidRequest("the device has not created a key yet; open InoInk once".into()))
}

/// Computer side: writes the encrypted transfer file for the device to import.
pub fn write_transfer(data_dir: &Path, encrypted: &[u8]) -> Result<()> {
    atomic_write(&data_dir.join(TRANSFER), encrypted)
}

/// Device side: decrypts a transfer file with this device's key.
pub(crate) fn decrypt_with_device_key(data_dir: &Path, bytes: &[u8]) -> Result<Vec<u8>> {
    let key = std::fs::read_to_string(data_dir.join(DEVICE_KEY)).map_err(|_| Error::Data("device key is missing".into()))?;
    let id = age::x25519::Identity::from_str(key.trim()).map_err(|_| Error::Data("device key is damaged".into()))?;
    age::decrypt(&id, bytes).map_err(|_| Error::Data("the transfer file is not for this device".into()))
}

/// Device side: imports and deletes a pending transfer file. Returns whether one was imported.
/// A damaged or foreign file is deleted too, and reported as an error.
pub fn import_provisioned(data_dir: &Path) -> Result<bool> {
    let transfer = data_dir.join(TRANSFER);
    let bytes = match std::fs::read(&transfer) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let result = (|| {
        let plain = decrypt_with_device_key(data_dir, &bytes)?;
        let creds: Credentials = serde_json::from_slice(&plain).map_err(|_| Error::Data("the transfer file is damaged".into()))?;
        FileCredentials::new(data_dir).save_credentials(&creds)
    })();
    let _ = std::fs::remove_file(&transfer);
    result.map(|_| true)
}
