//! Factories plugin → `factories.ts` (Kubb v5 plugin-faker parity).
//!
//! - `faker.seed(seed)` once at module scope (reproducible runs, distinct values per call)
//! - `DeepPartial` overrides applied **shallow** — nested objects replace
//!   (avoids hybrid oneOf/anyOf arms)
//! - Always concrete values (no `maybeNull`); OAS nullability is for types only
//! - Schema-graph cycle breaks (self-ref / lazy getters) — not numeric depth
//! - Nested `$ref` / arrays of refs **reuse** other `create*` factories

use std::fmt::Write as _;
use std::path::PathBuf;

use indexmap::IndexSet;
use serde::Deserialize;

use crate::adapter::SchemaNode;
use crate::config::{AffixNamingPatch, AffixNamingRule, CaseStyle, Config, OverrideKind};
use crate::error::Result;
use crate::naming::{AffixNames, AffixOverride, Naming};
use crate::plugin::{OutputFile, Plugin, PluginContext};
use crate::plugins::cycles::{build_ref_graph, find_circular_names};
use crate::plugins::faker_expr::{FakerCtx, SchemaMap, enum_literal_list, faker_expr};
use crate::plugins::types::{EnumStyle, OperationTypeNames, TypeStyles, TypesOptions};

pub struct FactoriesPlugin;

/// `[plugins.factories]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactoriesOptions {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_file")]
    pub file: String,
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// Factory names: `prefix + type name + suffix`. The type name is used as
    /// is, so `case` has no effect. Keys left out keep their defaults
    /// (prefix `create`, no suffix).
    #[serde(default)]
    pub naming: AffixNamingPatch,
    /// Per-type naming patches, matched (`kind = "schema_name"`) against the
    /// type name the factory builds.
    #[serde(default)]
    pub overrides: Vec<AffixOverride>,
}

/// The factory name for each type, after overrides.
#[derive(Debug)]
pub struct FactoryNames(AffixNames);

impl FactoryNames {
    /// The name of the factory that builds `type_name` (`createPet`). The type
    /// name is used as is: factories ignore `case`.
    pub fn name(&self, type_name: &str) -> String {
        let rule = self.0.rule_for(type_name);
        format!("{}{type_name}{}", rule.prefix, rule.suffix)
    }
}

fn default_naming() -> AffixNamingRule {
    AffixNamingRule {
        case: CaseStyle::PascalCase,
        prefix: "create".into(),
        suffix: String::new(),
    }
}

fn default_file() -> String {
    "factories.ts".into()
}

fn default_seed() -> u64 {
    2
}

impl Default for FactoriesOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            file: default_file(),
            seed: default_seed(),
            naming: AffixNamingPatch::default(),
            overrides: Vec::new(),
        }
    }
}

impl FactoriesOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        config.plugins.section_or_default(FactoriesPlugin.name())
    }

    /// How other generated files import the factories file.
    pub fn import_path(&self) -> String {
        super::relative_import(&self.file)
    }

    pub fn names(&self) -> Result<FactoryNames> {
        Ok(FactoryNames(AffixNames::new(
            default_naming().apply_patch(&self.naming),
            OverrideKind::SchemaName,
            self.overrides.clone(),
            "plugins.factories.overrides",
        )?))
    }
}

impl Plugin for FactoriesPlugin {
    fn name(&self) -> &'static str {
        "factories"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["types"]
    }

    fn enabled(&self, config: &Config) -> Result<bool> {
        Ok(FactoriesOptions::from_config(config)?.enabled)
    }

    fn writes_test_data(&self) -> bool {
        true
    }

    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(
            FactoriesOptions::from_config(config)?.file,
        )])
    }

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        let options = FactoriesOptions::from_config(ctx.config)?;
        let body = render_factories(ctx, &options)?;
        Ok(vec![OutputFile {
            relative_path: PathBuf::from(options.file),
            contents: body,
        }])
    }
}

fn render_factories(ctx: &PluginContext<'_>, cfg: &FactoriesOptions) -> Result<String> {
    let spec = ctx.spec;
    let naming = ctx.naming;
    let types = TypesOptions::from_config(ctx.config)?;
    let types_file = types.import_path();
    let type_styles = types.styles()?;
    let schemas: &SchemaMap = &spec.schemas;
    let graph = build_ref_graph(schemas);
    let circular = find_circular_names(&graph);
    let factory_names = cfg.names()?;
    let write_ctx = FactoryWriteCtx {
        naming,
        schemas,
        graph: &graph,
        circular: &circular,
        factory_names: &factory_names,
        type_styles: &type_styles,
    };

    let mut type_names: IndexSet<String> = IndexSet::new();
    for (name, _) in schemas {
        type_names.insert(naming.schema_type_name(name));
    }
    for operation in &spec.operations {
        let names = OperationTypeNames::new(naming, operation);
        for status_code in operation.responses.keys() {
            type_names.insert(names.response(status_code));
        }
        type_names.extend(names.success_response());
        type_names.extend(names.request());
    }

    let mut out = String::new();
    let _ = writeln!(out, "import {{ faker }} from '@faker-js/faker';");
    let _ = writeln!(out, "import type {{");
    for name in &type_names {
        let _ = writeln!(out, "  {name},");
    }
    let _ = writeln!(out, "}} from '{types_file}';");
    let _ = writeln!(out, "faker.seed({});", cfg.seed);
    out.push('\n');
    out.push_str(&prelude());
    out.push('\n');

    for (schema_name, schema) in schemas {
        let type_name = naming.schema_type_name(schema_name);
        let factory = factory_names.name(&type_name);
        write_schema_factory(
            &mut out,
            &factory,
            &type_name,
            Some(schema_name.as_str()),
            schema,
            &write_ctx,
        )?;
        out.push('\n');
    }

    for operation in &spec.operations {
        let names = OperationTypeNames::new(naming, operation);
        for (status_code, schema) in &operation.responses {
            let type_name = names.response(status_code);
            let factory = factory_names.name(&type_name);
            write_schema_factory(&mut out, &factory, &type_name, None, schema, &write_ctx)?;
            out.push('\n');
        }
        if let (Some(type_name), Some(status_code)) =
            (names.success_response(), names.success_status())
        {
            let factory = factory_names.name(&type_name);
            let status_factory = factory_names.name(&names.response(status_code));
            let _ = writeln!(
                out,
                "export function {factory}(\n  overrides?: DeepPartial<{type_name}>,\n): {type_name} {{"
            );
            let _ = writeln!(out, "  return {status_factory}(overrides);");
            let _ = writeln!(out, "}}");
            out.push('\n');
        }
        if let (Some(type_name), Some(request_body)) = (names.request(), &operation.request_body) {
            let factory = factory_names.name(&type_name);
            write_schema_factory(
                &mut out,
                &factory,
                &type_name,
                None,
                request_body,
                &write_ctx,
            )?;
            out.push('\n');
        }
    }

    Ok(out)
}

/// Shared inputs for writing one `create*` factory (keeps call sites readable).
struct FactoryWriteCtx<'a> {
    naming: &'a Naming,
    schemas: &'a SchemaMap,
    graph: &'a crate::plugins::cycles::RefGraph,
    circular: &'a IndexSet<String>,
    factory_names: &'a FactoryNames,
    type_styles: &'a TypeStyles,
}

fn write_schema_factory(
    out: &mut String,
    factory: &str,
    type_name: &str,
    root_schema_name: Option<&str>,
    schema: &SchemaNode,
    ctx: &FactoryWriteCtx<'_>,
) -> Result<()> {
    let naming = ctx.naming;
    let schemas = ctx.schemas;
    match schema {
        SchemaNode::Enum { values, .. } => {
            let literals = enum_literal_list(values);
            // A TypeScript `enum` does not accept its values as string literals.
            let cast = match root_schema_name {
                Some(name) if ctx.type_styles.enum_style(name) == EnumStyle::Enum => {
                    format!(" as {type_name}[]")
                }
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "export function {factory}(value?: {type_name}): {type_name} {{"
            );
            if literals.is_empty() {
                let _ = writeln!(out, "  return value as {type_name};");
            } else {
                let _ = writeln!(
                    out,
                    "  return value ?? faker.helpers.arrayElement<{type_name}>([{literals}]{cast});"
                );
            }
            let _ = writeln!(out, "}}");
        }
        SchemaNode::Ref { name } => {
            let target_type_name = naming.schema_type_name(name);
            let target_factory = ctx.factory_names.name(&target_type_name);
            let is_enum = matches!(schemas.get(name), Some(SchemaNode::Enum { .. }));
            if is_enum {
                let _ = writeln!(
                    out,
                    "export function {factory}(value?: {type_name}): {type_name} {{"
                );
                let _ = writeln!(out, "  return {target_factory}(value);");
                let _ = writeln!(out, "}}");
            } else {
                let _ = writeln!(
                    out,
                    "export function {factory}(\n  overrides?: DeepPartial<{type_name}>,\n): {type_name} {{"
                );
                let _ = writeln!(out, "  return {target_factory}(overrides);");
                let _ = writeln!(out, "}}");
            }
        }
        SchemaNode::Array { items, .. } => {
            let _ = writeln!(
                out,
                "export function {factory}(\n  overrides?: DeepPartial<{type_name}>,\n): {type_name} {{"
            );
            let faker_ctx =
                FakerCtx::for_schema(root_schema_name, ctx.graph, ctx.circular, ctx.factory_names);
            let item_expr = match items.as_ref() {
                SchemaNode::Ref { name } => {
                    let item_type_name = naming.schema_type_name(name);
                    let item_factory = ctx.factory_names.name(&item_type_name);
                    format!("{item_factory}()")
                }
                other => faker_expr(other, naming, schemas, faker_ctx),
            };
            let _ = writeln!(
                out,
                "  const defaults: {type_name} = arrayOf(() => ({item_expr}));"
            );
            let _ = writeln!(
                out,
                "  return overrides === undefined ? defaults : (overrides as {type_name});"
            );
            let _ = writeln!(out, "}}");
        }
        _ => {
            let faker_ctx =
                FakerCtx::for_schema(root_schema_name, ctx.graph, ctx.circular, ctx.factory_names);
            let defaults = faker_expr(schema, naming, schemas, faker_ctx);
            let paginated = is_paginated_list_schema(schema);
            let _ = writeln!(
                out,
                "export function {factory}(\n  overrides?: DeepPartial<{type_name}>,\n): {type_name} {{"
            );
            let _ = writeln!(out, "  const defaults: {type_name} = {defaults};");
            if paginated {
                let _ = writeln!(
                    out,
                    "  return overrides === undefined ? withSyncedPagination(defaults) : withSyncedPagination(applyOverrides(defaults, overrides), overrides);"
                );
            } else {
                let _ = writeln!(
                    out,
                    "  return overrides === undefined ? defaults : applyOverrides(defaults, overrides);"
                );
            }
            let _ = writeln!(out, "}}");
        }
    }
    Ok(())
}

fn prelude() -> String {
    r#"type DeepPartial<T> = T extends Array<infer U>
  ? Array<DeepPartial<U>>
  : T extends object
    ? { [K in keyof T]?: DeepPartial<T[K]> }
    : T;

// Default length matches Kubb's `helpers.multiple(fn)` (always 3). A `min: 0`
// range often yields `[]`, which breaks tests that need at least one list item
// (e.g. data-versions → SRA dimension filters → table never leaves LinearProgress).
function arrayOf<T>(item: () => T, count: { min: number; max: number } = { min: 1, max: 3 }): T[] {
  return faker.helpers.multiple(item, { count });
}

/** Kubb-style shallow overrides. Nested objects replace wholesale — never deep-merge
 *  onto a random oneOf/anyOf arm (that produced invalid hybrid API shapes). */
function applyOverrides<T>(base: T, overrides: DeepPartial<T>): T {
  if (overrides === null || overrides === undefined) return base;
  if (Array.isArray(overrides)) return overrides as T;
  if (typeof overrides !== 'object') return overrides as T;
  if (typeof base !== 'object' || base === null || Array.isArray(base)) return overrides as T;
  // Spread materialises lazy getters (cycle breaks) once — Kubb does the same.
  return { ...(base as Record<string, unknown>), ...(overrides as Record<string, unknown>) } as T;
}

/** Keep `_meta.pagination.count` aligned with `results` unless the caller set count. */
function withSyncedPagination<T>(value: T, overrides?: DeepPartial<T>): T {
  const v = value as {
    results?: unknown[];
    _meta?: { pagination?: { count?: number; page?: number; pages?: number; page_size?: number } };
  };
  if (!v || !Array.isArray(v.results) || !v._meta?.pagination) return value;
  const overrideCount = (overrides as { _meta?: { pagination?: { count?: number } } } | undefined)
    ?._meta?.pagination?.count;
  if (overrideCount !== undefined) return value;
  const count = v.results.length;
  const pageSize = v._meta.pagination.page_size || 100;
  return {
    ...v,
    _meta: {
      ...v._meta,
      pagination: {
        ...v._meta.pagination,
        count,
        page: v._meta.pagination.page || 1,
        page_size: pageSize,
        pages: Math.max(1, Math.ceil(count / pageSize) || 1),
      },
    },
  } as T;
}

export function configureFaker(opts: { seed?: number }): void {
  if (opts.seed !== undefined) faker.seed(opts.seed);
}
"#
    .into()
}

fn is_paginated_list_schema(schema: &SchemaNode) -> bool {
    match schema {
        SchemaNode::Object { properties, .. } => {
            properties.contains_key("results") && properties.contains_key("_meta")
        }
        SchemaNode::AllOf { items, .. } => items.iter().any(is_paginated_list_schema),
        SchemaNode::Ref { .. } => false,
        _ => false,
    }
}
