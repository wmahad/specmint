//! MSW handlers plugin → `handlers.ts` (improved vs Kubb).
//!
//! - Options: body | resolver | `{ status, data, delayMs }`
//! - Status helpers on `handler.response[code]` / `.success`
//! - Optional `handlers_barrel` for Kubb-style `export const handlers = […]`
//! - Prefer status `switch` + type guards over `as` casts
//! - `parser: "faker"` seeds success bodies; `"data"` leaves defaults to the caller

use std::fmt::Write as _;
use std::path::PathBuf;

use indexmap::IndexSet;
use serde::Deserialize;

use crate::adapter::SchemaNode;
use crate::config::{AffixNamingPatch, AffixNamingRule, CaseStyle, Config, OverrideKind};
use crate::error::Result;
use crate::naming::{AffixNames, AffixOverride, apply_affix_rule};
use crate::plugin::{ActiveDependency, OutputFile, Plugin, PluginContext};
use crate::plugins::factories::FactoriesOptions;
use crate::plugins::types::{OperationTypeNames, TypesOptions};

pub struct MswPlugin;

/// `[plugins.msw]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MswOptions {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_file")]
    pub file: String,
    /// `"faker"` seeds success bodies from factories; `"data"` requires caller data.
    #[serde(default = "default_parser")]
    pub parser: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// When true, emit `export const handlers = […all handlers…]` (Kubb `handlers: true`).
    /// Default false — the app imports handlers à la carte.
    #[serde(default)]
    pub handlers_barrel: bool,
    /// Handler names: `prefix + operation id cased by case + suffix`.
    /// Keys left out keep their defaults (camelCase, no prefix, `Handler`).
    #[serde(default)]
    pub naming: AffixNamingPatch,
    /// Per-operation naming patches, matched (`kind = "operation_id"`) against
    /// the operation id.
    #[serde(default)]
    pub overrides: Vec<AffixOverride>,
}

/// The handler name for each operation, after overrides.
#[derive(Debug)]
pub struct HandlerNames(AffixNames);

impl HandlerNames {
    /// The name of the handler for `operation_id` (`listPetsHandler`).
    pub fn name(&self, operation_id: &str) -> String {
        apply_affix_rule(operation_id, &self.0.rule_for(operation_id))
    }
}

fn default_naming() -> AffixNamingRule {
    AffixNamingRule {
        case: CaseStyle::CamelCase,
        prefix: String::new(),
        suffix: "Handler".into(),
    }
}

fn default_file() -> String {
    "handlers.ts".into()
}

fn default_parser() -> String {
    "faker".into()
}

fn default_base_url() -> String {
    "*".into()
}

impl Default for MswOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            file: default_file(),
            parser: default_parser(),
            base_url: default_base_url(),
            handlers_barrel: false,
            naming: AffixNamingPatch::default(),
            overrides: Vec::new(),
        }
    }
}

impl MswOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        config.plugins.section_or_default(MswPlugin.name())
    }

    pub fn handler_names(&self) -> Result<HandlerNames> {
        Ok(HandlerNames(AffixNames::new(
            default_naming().apply_patch(&self.naming),
            OverrideKind::OperationId,
            self.overrides.clone(),
            "plugins.msw.overrides",
        )?))
    }
}

impl Plugin for MswPlugin {
    fn name(&self) -> &'static str {
        "msw"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["types", "factories"]
    }

    fn enabled(&self, config: &Config) -> Result<bool> {
        Ok(MswOptions::from_config(config)?.enabled)
    }

    fn active_dependencies(&self, config: &Config) -> Result<Vec<ActiveDependency>> {
        let mut dependencies = vec![ActiveDependency {
            plugin: "types",
            alternative: None,
        }];
        // Only the faker parser imports factories.
        if MswOptions::from_config(config)?.parser == "faker" {
            dependencies.push(ActiveDependency {
                plugin: "factories",
                alternative: Some("set `parser = \"data\"` in [plugins.msw]".to_string()),
            });
        }
        Ok(dependencies)
    }

    fn writes_test_data(&self) -> bool {
        true
    }

    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(MswOptions::from_config(config)?.file)])
    }

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        let options = MswOptions::from_config(ctx.config)?;
        let body = render_msw(ctx, &options)?;
        Ok(vec![OutputFile {
            relative_path: PathBuf::from(options.file),
            contents: body,
        }])
    }
}

fn render_msw(ctx: &PluginContext<'_>, msw_cfg: &MswOptions) -> Result<String> {
    let spec = ctx.spec;
    let naming = ctx.naming;
    let use_faker = msw_cfg.parser == "faker";
    let types_file = TypesOptions::from_config(ctx.config)?.import_path();
    let factories = FactoriesOptions::from_config(ctx.config)?;
    let factories_file = factories.import_path();
    let factory_names = factories.names()?;
    let handler_names_for = msw_cfg.handler_names()?;

    let mut type_imports: IndexSet<String> = IndexSet::new();
    let mut factory_imports: IndexSet<String> = IndexSet::new();
    let mut handler_names: Vec<String> = Vec::new();

    for op in &spec.operations {
        let names = OperationTypeNames::new(naming, op);
        for code in op.responses.keys() {
            type_imports.insert(names.response(code));
        }
        if let Some(success) = names.success_response() {
            if use_faker {
                factory_imports.insert(factory_names.name(&success));
            }
            type_imports.insert(success);
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "import {{ delay, http, HttpResponse }} from 'msw';");
    let _ = writeln!(out, "import type {{ HttpHandler }} from 'msw';");
    if !type_imports.is_empty() {
        let _ = writeln!(out, "import type {{");
        for name in &type_imports {
            let _ = writeln!(out, "  {name},");
        }
        let _ = writeln!(out, "}} from '{types_file}';");
    }
    if use_faker && !factory_imports.is_empty() {
        let _ = writeln!(out, "import {{");
        for name in &factory_imports {
            let _ = writeln!(out, "  {name},");
        }
        let _ = writeln!(out, "}} from '{factories_file}';");
    }
    out.push('\n');
    out.push_str(PRELUDE);
    out.push('\n');

    for op in &spec.operations {
        let handler = handler_names_for.name(&op.operation_id);
        handler_names.push(handler.clone());
        let names = OperationTypeNames::new(naming, op);
        let method = op.method.to_ascii_lowercase();
        let http_method = msw_http_method(&method);
        let url = msw_url(&msw_cfg.base_url, &op.path);
        // The response the success alias points at, as the types plugin picks it.
        let primary = names
            .success_status()
            .map(|code| (code, &op.responses[code]));
        let success_type = names.success_response();
        // Empty-body success responses (`Unknown`) have no useful factory default.
        let success_factory = primary.and_then(|(_code, schema)| {
            if matches!(schema, SchemaNode::Unknown { .. }) {
                None
            } else {
                success_type.as_ref().map(|t| factory_names.name(t))
            }
        });

        let success_ty = if matches!(primary.map(|(_, s)| s), Some(SchemaNode::Unknown { .. })) {
            "unknown".into()
        } else {
            success_type.clone().unwrap_or_else(|| "unknown".into())
        };
        let status_lit = {
            let lit = op
                .responses
                .keys()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(" | ");
            if lit.is_empty() { "number".into() } else { lit }
        };

        let _ = writeln!(
            out,
            "export function {handler}(\n  input?: HandlerInput<{success_ty}, {status_lit}>,\n): HttpHandler {{"
        );
        let _ = writeln!(
            out,
            "  return http.{http_method}('{url}', async (info) => {{"
        );
        let _ = writeln!(out, "    const normalized = normalizeHandlerInput(input);");
        let _ = writeln!(
            out,
            "    if (normalized.delayMs) await delay(normalized.delayMs);"
        );
        let _ = writeln!(out, "    if (isResolver(normalized.data)) {{");
        let _ = writeln!(out, "      return normalized.data(info);");
        let _ = writeln!(out, "    }}");

        if let Some((primary_code, _)) = primary {
            let _ = writeln!(
                out,
                "    const status = normalized.status ?? {primary_code};"
            );
            let _ = writeln!(out, "    switch (status) {{");
            for (code, schema) in &op.responses {
                let status_num: u16 = code.parse().unwrap_or(200);
                let type_name = names.response(code);
                let _ = writeln!(out, "      case {code}: {{");
                if status_num == 204 {
                    let _ = writeln!(
                        out,
                        "        return new HttpResponse(null, {{ status: {status_num} }});"
                    );
                } else if matches!(schema, SchemaNode::Unknown { .. }) {
                    // Spec has no JSON schema — still echo caller data (common in tests).
                    let _ = writeln!(
                        out,
                        "        return HttpResponse.json(normalized.data ?? null, {{ status: {status_num} }});"
                    );
                } else if Some(code.as_str()) == Some(primary_code) && use_faker {
                    if let Some(ref factory) = success_factory {
                        let _ = writeln!(
                            out,
                            "        const body: {type_name} = normalized.data ?? {factory}();"
                        );
                        let _ = writeln!(
                            out,
                            "        return HttpResponse.json(body, {{ status: {status_num} }});"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "        return HttpResponse.json(normalized.data, {{ status: {status_num} }});"
                        );
                    }
                } else {
                    // Non-success (or no faker): caller supplies body when needed.
                    let _ = writeln!(
                        out,
                        "        return HttpResponse.json(normalized.data as {type_name} | undefined, {{ status: {status_num} }});"
                    );
                }
                let _ = writeln!(out, "      }}");
            }
            let _ = writeln!(out, "      default: {{");
            let _ = writeln!(
                out,
                "        return HttpResponse.json(normalized.data ?? null, {{ status: Number(status) }});"
            );
            let _ = writeln!(out, "      }}");
            let _ = writeln!(out, "    }}");
        } else {
            let _ = writeln!(
                out,
                "    return HttpResponse.json(normalized.data ?? null, {{ status: normalized.status ?? 200 }});"
            );
        }

        let _ = writeln!(out, "  }});");
        let _ = writeln!(out, "}}");
        out.push('\n');

        // Namespaced response helpers (no separate builders const / as const).
        let _ = writeln!(out, "{handler}.response = {{");
        for (code, _) in &op.responses {
            let status_num: u16 = code.parse().unwrap_or(200);
            let type_name = names.response(code);
            if status_num == 204 {
                let _ = writeln!(
                    out,
                    "  {code}: (_data?: {type_name}) => new HttpResponse(null, {{ status: {status_num} }}),"
                );
            } else {
                let _ = writeln!(
                    out,
                    "  {code}: (data: {type_name}) => HttpResponse.json(data, {{ status: {status_num} }}),"
                );
            }
        }
        let _ = writeln!(out, "}};");
        if let Some((primary_code, _)) = primary {
            let _ = writeln!(
                out,
                "{handler}.success = {handler}.response[{primary_code}];"
            );
        }
        // Kubb-compatible top-level aliases still used across the test suite.
        for (code, _) in &op.responses {
            let _ = writeln!(
                out,
                "export const {handler}Response{code} = {handler}.response[{code}];"
            );
        }
        let _ = writeln!(out, "{handler}.path = '{url}';");
        let _ = writeln!(out, "{handler}.method = '{method}' as const;");
        out.push('\n');
    }

    // Opt-in Kubb `handlers: true` barrel — off by default so existing imports stay lean.
    if msw_cfg.handlers_barrel && !handler_names.is_empty() {
        let _ = writeln!(out, "export const handlers = [");
        for name in &handler_names {
            let _ = writeln!(out, "  {name}(),");
        }
        let _ = writeln!(out, "] as const;");
        out.push('\n');
    }

    Ok(out)
}

const PRELUDE: &str = r#"type Resolver = (
  info: Parameters<Parameters<typeof http.get>[1]>[0],
) => Response | Promise<Response>;

type HandlerOptions<TBody, TStatus extends number | string = number> = {
  status?: TStatus;
  data?: TBody | Resolver;
  delayMs?: number;
};

type HandlerInput<TBody, TStatus extends number | string = number> =
  | TBody
  | Resolver
  | HandlerOptions<TBody, TStatus>;

function isResolver(value: unknown): value is Resolver {
  return typeof value === 'function';
}

function isHandlerOptions<TBody, TStatus extends number | string>(
  value: object,
): value is HandlerOptions<TBody, TStatus> {
  // Only treat plain option bags as options. Many API bodies have a `status` or
  // `data` field — those must still be returned as the response body.
  if (Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return (
    keys.length > 0 &&
    keys.every((key) => key === 'status' || key === 'data' || key === 'delayMs')
  );
}

function normalizeHandlerInput<TBody, TStatus extends number | string>(
  input?: HandlerInput<TBody, TStatus>,
): HandlerOptions<TBody, TStatus> {
  if (input === undefined) return {};
  if (isResolver(input)) return { data: input };
  if (typeof input === 'object' && input !== null && isHandlerOptions<TBody, TStatus>(input)) {
    return input;
  }
  return { data: input };
}
"#;

fn msw_http_method(method: &str) -> &str {
    match method {
        "get" => "get",
        "post" => "post",
        "put" => "put",
        "patch" => "patch",
        "delete" => "delete",
        "options" => "options",
        "head" => "head",
        _ => "all",
    }
}

fn msw_url(base_url: &str, path: &str) -> String {
    // MSW / path-to-regexp expect Express-style `:param`, not OpenAPI `{param}`.
    // Kubb's URLPath does the same: `/pets/{petId}` → `/pets/:petId`.
    let path = openapi_path_to_msw(path);
    if base_url.is_empty() {
        path
    } else if base_url == "*" {
        format!("*{path}")
    } else {
        let base = base_url.trim_end_matches('/');
        format!("{base}{path}")
    }
}

fn openapi_path_to_msw(path: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            out.push(':');
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
                out.push(inner);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{msw_url, openapi_path_to_msw};

    #[test]
    fn converts_openapi_braces_to_colon_params() {
        assert_eq!(openapi_path_to_msw("/pets/{petId}"), "/pets/:petId");
        assert_eq!(
            openapi_path_to_msw("/tenants/{tenant_id}/members/{member_uuid}/"),
            "/tenants/:tenant_id/members/:member_uuid/"
        );
        assert_eq!(openapi_path_to_msw("pets"), "/pets");
    }

    #[test]
    fn prefixes_wildcard_base_url() {
        assert_eq!(
            msw_url("*", "/pet-stores/sub-stores/{parent_path}/"),
            "*/pet-stores/sub-stores/:parent_path/"
        );
    }
}
