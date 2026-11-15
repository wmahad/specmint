//! Compiled-in emitters (Kubb plugins without npm packaging).

mod cycles;
mod factories;
mod faker_expr;
mod msw;
mod registry;
mod types;
mod zod;

use std::path::PathBuf;

use crate::adapter::ParsedSpec;
use crate::config::{Config, EffectiveEmitters};
use crate::error::Result;
use crate::naming::Naming;

pub use registry::EmitterRegistry;

pub struct OutputFile {
    pub relative_path: PathBuf,
    pub contents: String,
}

pub struct EmitCtx<'a> {
    pub config: &'a Config,
    pub naming: &'a Naming,
    pub spec: &'a ParsedSpec,
    pub enabled: EffectiveEmitters,
}

pub trait Emitter: Send + Sync {
    fn id(&self) -> &'static str;
    fn dependencies(&self) -> &'static [&'static str];
    fn enabled(&self, ctx: &EmitCtx<'_>) -> bool;
    fn generate(&self, ctx: &EmitCtx<'_>) -> Result<Vec<OutputFile>>;
}
