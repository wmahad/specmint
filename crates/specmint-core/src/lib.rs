//! `specmint` — OpenAPI → TypeScript code generation.
//!
//! Pipeline: load schema → parse into [`ParsedSpec`] → run emitters (types,
//! factories, MSW, optional Zod) → write files. One binary; emitters are
//! compiled in (no npm plugin graph).

pub mod adapter;
pub mod config;
pub mod emit;
pub mod error;
pub mod generate;
pub mod naming;
pub mod source;
pub mod write;

pub use config::Config;
pub use error::{Error, Result};
pub use generate::{ConfigSource, GenerateOptions, generate};
