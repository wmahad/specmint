//! Zod plugin → `zod.ts` (Phase 3 MVP).

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::adapter::{EnumValue, ParsedSpec, PrimitiveKind, SchemaNode};
use serde::Deserialize;

use crate::config::Config;
use crate::error::Result;
use crate::naming::Naming;
use crate::plugin::{OutputFile, Plugin, PluginContext};

pub struct ZodPlugin;

/// `[plugins.zod]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZodOptions {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_file")]
    pub file: String,
    #[serde(default = "default_true")]
    pub inferred: bool,
}

fn default_file() -> String {
    "zod.ts".into()
}

fn default_true() -> bool {
    true
}

impl Default for ZodOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            file: default_file(),
            inferred: true,
        }
    }
}

impl ZodOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        config.plugins.section_or_default(ZodPlugin.name())
    }
}

impl Plugin for ZodPlugin {
    fn name(&self) -> &'static str {
        "zod"
    }

    fn enabled(&self, config: &Config) -> Result<bool> {
        Ok(ZodOptions::from_config(config)?.enabled)
    }

    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(ZodOptions::from_config(config)?.file)])
    }

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        let options = ZodOptions::from_config(ctx.config)?;
        let body = render_zod(ctx.spec, ctx.naming, options.inferred)?;
        Ok(vec![OutputFile {
            relative_path: PathBuf::from(options.file),
            contents: body,
        }])
    }
}

fn render_zod(spec: &ParsedSpec, naming: &Naming, inferred: bool) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "import {{ z }} from 'zod';");
    out.push('\n');

    // Forward-declare refs with z.lazy by emitting schemas in order; for cycles use lazy.
    for (name, schema) in &spec.schemas {
        let zod_name = naming.zod_name(&naming.schema_type_name(name));
        let expr = schema_to_zod(schema, naming, true)?;
        let _ = writeln!(out, "export const {zod_name} = {expr};");
        if inferred {
            let type_name = naming.schema_type_name(name);
            let _ = writeln!(
                out,
                "export type {type_name}FromZod = z.infer<typeof {zod_name}>;"
            );
        }
        out.push('\n');
    }

    Ok(out)
}

fn schema_to_zod(schema: &SchemaNode, naming: &Naming, top_level: bool) -> Result<String> {
    let _ = top_level;
    let base = match schema {
        SchemaNode::Ref { name } => {
            let zod_name = naming.zod_name(&naming.schema_type_name(name));
            format!("z.lazy(() => {zod_name})")
        }
        SchemaNode::Primitive {
            primitive,
            format,
            nullable,
            ..
        } => {
            let mut expr = match primitive {
                PrimitiveKind::String => match format.as_deref() {
                    Some("uuid") => "z.string().uuid()".into(),
                    Some("email") => "z.string().email()".into(),
                    Some("uri") | Some("url") => "z.string().url()".into(),
                    Some("date-time") => "z.string().datetime({ offset: true })".into(),
                    _ => "z.string()".into(),
                },
                PrimitiveKind::Integer => "z.number().int()".into(),
                PrimitiveKind::Number => "z.number()".into(),
                PrimitiveKind::Boolean => "z.boolean()".into(),
                PrimitiveKind::Null => "z.null()".into(),
            };
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::Enum {
            values, nullable, ..
        } => {
            let strings: Vec<&str> = values
                .iter()
                .filter_map(|v| match v {
                    EnumValue::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            let mut expr = if strings.len() == values.len() && !strings.is_empty() {
                if strings.len() == 1 {
                    format!("z.literal('{}')", strings[0])
                } else {
                    let lit = strings
                        .iter()
                        .map(|s| format!("'{s}'"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("z.enum([{lit}])")
                }
            } else {
                // Mixed / non-string enums
                let parts: Vec<String> = values
                    .iter()
                    .map(|v| match v {
                        EnumValue::String(s) => format!("z.literal('{s}')"),
                        EnumValue::Number(n) => format!("z.literal({n})"),
                        EnumValue::Bool(b) => format!("z.literal({b})"),
                        EnumValue::Null => "z.null()".into(),
                    })
                    .collect();
                if parts.is_empty() {
                    "z.never()".into()
                } else if parts.len() == 1 {
                    parts[0].clone()
                } else {
                    format!("z.union([{}])", parts.join(", "))
                }
            };
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::Array {
            items, nullable, ..
        } => {
            let inner = schema_to_zod(items, naming, false)?;
            let mut expr = format!("z.array({inner})");
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::Object {
            properties,
            required,
            additional_properties,
            nullable,
            ..
        } => {
            let mut fields = Vec::new();
            for (prop, prop_schema) in properties {
                let mut field = schema_to_zod(prop_schema, naming, false)?;
                if !required.iter().any(|r| r == prop) {
                    field = format!("{field}.optional()");
                }
                fields.push(format!("  {prop}: {field}"));
            }
            let mut expr = format!("z.object({{\n{}\n}})", fields.join(",\n"));
            if let Some(ap) = additional_properties {
                let ap_expr = schema_to_zod(ap, naming, false)?;
                expr = format!("{expr}.catchall({ap_expr})");
            }
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::AllOf { items, nullable } => {
            if items.is_empty() {
                return Ok("z.unknown()".into());
            }
            let mut expr = schema_to_zod(&items[0], naming, false)?;
            for item in items.iter().skip(1) {
                let next = schema_to_zod(item, naming, false)?;
                expr = format!("z.intersection({expr}, {next})");
            }
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::OneOf {
            items, nullable, ..
        }
        | SchemaNode::AnyOf { items, nullable } => {
            if items.is_empty() {
                return Ok("z.never()".into());
            }
            let parts: Result<Vec<_>> = items
                .iter()
                .map(|i| schema_to_zod(i, naming, false))
                .collect();
            let parts = parts?;
            let mut expr = if parts.len() == 1 {
                parts[0].clone()
            } else {
                format!("z.union([{}])", parts.join(", "))
            };
            if *nullable {
                expr = format!("{expr}.nullable()");
            }
            expr
        }
        SchemaNode::Unknown { .. } => "z.unknown()".into(),
    };
    Ok(base)
}
