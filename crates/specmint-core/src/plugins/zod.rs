//! Zod plugin → `zod.ts` (Phase 3 MVP).

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::adapter::{EnumValue, ParsedSpec, PrimitiveKind, SchemaNode};
use serde::Deserialize;

use crate::config::{AffixNamingPatch, AffixNamingRule, CaseStyle, Config};
use crate::error::Result;
use crate::naming::{Naming, apply_affix_rule};
use crate::plugin::{OutputFile, Plugin, PluginContext};
use crate::plugins::cycles::{
    CycleKind, RefGraph, build_ref_graph, find_circular_names, property_cycle_kind,
};
use crate::plugins::faker_expr::SchemaMap;

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
    /// Schema constant names: `prefix + type name cased by case + suffix`.
    /// Keys left out keep their defaults (camelCase, no prefix, `Schema`).
    #[serde(default)]
    pub naming: AffixNamingPatch,
}

fn default_naming() -> AffixNamingRule {
    AffixNamingRule {
        case: CaseStyle::CamelCase,
        prefix: String::new(),
        suffix: "Schema".into(),
    }
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
            naming: AffixNamingPatch::default(),
        }
    }
}

impl ZodOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        config.plugins.section_or_default(ZodPlugin.name())
    }

    /// The name of the zod schema for `type_name` (`petSchema`).
    pub fn schema_name(&self, type_name: &str) -> String {
        apply_affix_rule(type_name, &default_naming().apply_patch(&self.naming))
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
        let body = render_zod(ctx.spec, ctx.naming, &options)?;
        Ok(vec![OutputFile {
            relative_path: PathBuf::from(options.file),
            contents: body,
        }])
    }
}

fn render_zod(spec: &ParsedSpec, naming: &Naming, zod: &ZodOptions) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "import {{ z }} from 'zod';");
    out.push('\n');

    let graph = build_ref_graph(&spec.schemas);
    let circular = find_circular_names(&graph);
    for (name, schema) in &spec.schemas {
        let zod_name = zod.schema_name(&naming.schema_type_name(name));
        let ctx = ZodCtx {
            naming,
            zod,
            schemas: &spec.schemas,
            graph: &graph,
            circular: &circular,
            root: name,
            direct_refs: false,
        };
        let expr = schema_to_zod(schema, ctx)?;
        let _ = writeln!(out, "export const {zod_name} = {expr};");
        if zod.inferred {
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

#[derive(Clone, Copy)]
struct ZodCtx<'a> {
    naming: &'a Naming,
    zod: &'a ZodOptions,
    schemas: &'a SchemaMap,
    graph: &'a RefGraph,
    circular: &'a indexmap::IndexSet<String>,
    /// The component schema being written.
    root: &'a str,
    /// Inside a getter, refs need no `z.lazy`: the getter already defers them.
    direct_refs: bool,
}

fn schema_to_zod(schema: &SchemaNode, ctx: ZodCtx<'_>) -> Result<String> {
    let base = match schema {
        SchemaNode::Ref { name } => {
            let zod_name = ctx.zod.schema_name(&ctx.naming.schema_type_name(name));
            if ctx.direct_refs {
                zod_name
            } else {
                format!("z.lazy(() => {zod_name})")
            }
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
            let inner = schema_to_zod(items, ctx)?;
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
                // A `z.lazy` that closes a cycle leaves TypeScript inferring `any`
                // (TS7022); a getter is Zod 4's typed way to recurse.
                let in_cycle = property_cycle_kind(
                    ctx.root,
                    prop_schema,
                    ctx.schemas,
                    ctx.graph,
                    ctx.circular,
                ) != CycleKind::None;
                let field_ctx = ZodCtx {
                    direct_refs: ctx.direct_refs || in_cycle,
                    ..ctx
                };
                let mut field = schema_to_zod(prop_schema, field_ctx)?;
                if !required.iter().any(|r| r == prop) {
                    field = format!("{field}.optional()");
                }
                if in_cycle {
                    fields.push(format!("  get {prop}() {{ return {field}; }}"));
                } else {
                    fields.push(format!("  {prop}: {field}"));
                }
            }
            let mut expr = format!("z.object({{\n{}\n}})", fields.join(",\n"));
            if let Some(ap) = additional_properties {
                let ap_expr = schema_to_zod(ap, ctx)?;
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
            let mut expr = schema_to_zod(&items[0], ctx)?;
            for item in items.iter().skip(1) {
                let next = schema_to_zod(item, ctx)?;
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
            let parts: Result<Vec<_>> = items.iter().map(|i| schema_to_zod(i, ctx)).collect();
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
