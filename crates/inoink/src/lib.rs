//! InoInk core: a renderer-independent Inoreader client.

pub mod api;
pub mod cache;
pub mod content;
pub mod error;
pub mod model;
pub mod store;
pub mod util;

pub use error::{Error, Result};
