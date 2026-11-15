//! The built-in plugins. Each one owns its name, its options (its section of
//! the config's `plugins` table), its on/off switch and its output files.
//!
//! Adding a plugin means adding a module here and listing it in [`builtin`].

mod cycles;
mod factories;
mod faker_expr;
mod msw;
mod types;
mod zod;

use crate::plugin::Plugin;

pub use factories::{FactoriesOptions, FactoriesPlugin, FactoryNames};
pub use msw::{HandlerNames, MswOptions, MswParser, MswPlugin};
pub use types::{
    EnumStyle, ObjectSyntax, OperationTypeNames, TypeStyles, TypesOptions, TypesOverride,
    TypesPlugin,
};
pub use zod::{ZodOptions, ZodPlugin};

/// How one generated file imports another: `"models/api.ts"` → `"./models/api"`.
fn relative_import(file: &str) -> String {
    format!("./{}", file.trim_end_matches(".ts"))
}

/// Every built-in plugin.
pub fn builtin() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(TypesPlugin),
        Box::new(FactoriesPlugin),
        Box::new(MswPlugin),
        Box::new(ZodPlugin),
    ]
}
