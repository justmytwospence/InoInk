//! InoInk core: a renderer-independent Inoreader client.

pub mod api;
pub mod cache;
pub mod content;
pub mod credentials;
pub mod error;
pub mod highlight;
pub mod model;
pub mod runtime;
pub mod share;
pub mod store;
pub mod sync;
pub mod util;

/// The InoInk version shown in About and Status.
/// The release version. Release and dev builds set `INOINK_VERSION` (see `scripts/version.sh`);
/// a plain `cargo build` falls back to the placeholder in Cargo.toml.
pub const VERSION: &str = match option_env!("INOINK_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub use error::{Error, Result};
