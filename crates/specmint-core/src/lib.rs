//! `specmint` — OpenAPI → TypeScript code generation.
//!
//! Pipeline: load schema → parse into [`ParsedSpec`] → run the enabled plugins
//! → write files. Core knows plugins only through the [`plugin::Plugin`]
//! trait; the built-in plugins live in [`plugins`].

pub mod adapter;
pub mod config;
pub mod error;
pub mod generate;
pub mod naming;
pub mod plugin;
pub mod plugins;
pub mod source;
pub mod write;

pub use config::Config;
pub use error::{Error, Result};
pub use generate::{ConfigSource, GenerateOptions, generate};
