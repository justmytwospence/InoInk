//! InoInk core: a renderer-independent Inoreader client.
//!
//! Modules: [`model`] (identifiers and value types), [`store`] (SQLite source of local truth),
//! [`api`] (Inoreader HTTP client). Nothing here depends on the UI.

pub mod api;
pub mod error;
pub mod model;
pub mod store;
pub mod util;

pub use error::{Error, Result};
