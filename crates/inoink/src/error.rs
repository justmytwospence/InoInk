use std::fmt;

/// Failure classes the UI and the sync worker act on.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No network or the connection failed before a response arrived.
    #[error("offline")]
    Offline,
    /// The access token is invalid and could not be refreshed.
    #[error("sign in again")]
    NeedsAuthorization,
    /// A zone's quota is exhausted; requests resume after `reset_after_secs`.
    #[error("quota paused for {reset_after_secs}s")]
    QuotaPaused { reset_after_secs: u64 },
    /// A transient failure (5xx, timeout); the same request may be retried.
    #[error("retryable: {0}")]
    Retryable(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A non-idempotent write may or may not have been applied.
    #[error("outcome unknown: {0}")]
    UnknownOutcome(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("data: {0}")]
    Data(String),
    #[error("io: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Storage(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Data(e.to_string())
    }
}

impl Error {
    /// Whether the same request may simply be issued again later.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Offline | Error::Retryable(_) | Error::QuotaPaused { .. })
    }
}

/// Redacts a value in `Debug`/`Display` output.
pub struct Redacted<'a>(pub &'a str);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
