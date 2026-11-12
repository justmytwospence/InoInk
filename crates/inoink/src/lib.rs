//! InoInk core: a renderer-independent Inoreader client.

pub mod api;
pub mod cache;
pub mod content;
pub mod credentials;
pub mod error;
pub mod model;
pub mod runtime;
pub mod store;
pub mod sync;
pub mod util;

/// The InoInk version shown in About and Status.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use error::{Error, Result};
