//! Inoreader credential storage.
//!
//! The sign-in lives in `<data>/credentials.json`, outside the database and its backups. It is
//! written by `inoink-cli setup` (through `accounts`' transfer file) or by the Kobo's own setup
//! page. Kobo's onboard FAT storage has no meaningful file permissions, so anyone with the
//! device can read it: use an application registration you can revoke.

use crate::api::auth::{AppConfig, TokenStore, Tokens};
use crate::cache::atomic_write;
use crate::error::{Error, Redacted, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    /// The account's name, for the Accounts screen.
    #[serde(default)]
    pub user_name: String,
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
            user_name: String::new(),
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
    /// When set, token refreshes are only saved while the file still belongs to this account, so
    /// a worker of a replaced sign-in cannot overwrite the new one.
    expect_user: Option<String>,
}

pub(crate) fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

impl FileCredentials {
    pub fn new(data_dir: &Path) -> FileCredentials {
        FileCredentials { path: data_dir.join(CREDENTIALS), lock: Mutex::new(()), expect_user: None }
    }

    /// A store whose `save` refuses to write once the file holds another account.
    pub fn for_account(data_dir: &Path, user_id: &str) -> FileCredentials {
        let mut f = FileCredentials::new(data_dir);
        f.expect_user = Some(user_id.to_string()).filter(|u| !u.is_empty());
        f
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
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
        if self.expect_user.as_deref().is_some_and(|u| c.user_id != u) {
            return Err(Error::NeedsAuthorization);
        }
        c.access_token = t.access_token.clone();
        c.refresh_token = t.refresh_token.clone();
        c.expires_at = t.expires_at;
        c.scope = t.scope.clone();
        self.save_credentials(&c)
    }
}
