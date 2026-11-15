//! Semantic OpenAPI checks that catch codegen footguns.
//!
//! We intentionally do **not** validate against the official OAS 3.0 JSON
//! Schema meta-document: this API’s schema is tagged `openapi: 3.0.3` but uses
//! 3.1 `const` (~90×). Meta-schema validation would reject a document we must
//! accept. Instead we validate on the typed [`openapiv3::OpenAPI`] model:
//! duplicate ids, unresolved local `$ref`s, and path-param mismatches.

use std::collections::BTreeSet;

use openapiv3::{
    AdditionalProperties, OpenAPI, Operation, Parameter, ParameterSchemaOrContent, PathItem,
    ReferenceOr, RequestBody, Response, Schema, SchemaKind, Type,
};
use regex::Regex;
use tracing::warn;

use crate::error::{Error, Result};

/// Run semantic validation. Returns an error listing all problems found.
pub fn validate_openapi(spec: &OpenAPI) -> Result<()> {
    let mut errors = Vec::new();

    if !spec.openapi.starts_with("3.0") {
        errors.push(format!(
            "unsupported openapi version `{}` (expected 3.0.x; use oas3 crate for 3.1)",
            spec.openapi
        ));
    }

    let catalogs = ComponentCatalogs::from_spec(spec);
    check_duplicate_operation_ids(spec, &mut errors);
    check_path_parameters(spec, &mut errors);
    check_local_refs(spec, &catalogs, &mut errors);

    if errors.is_empty() {
        return Ok(());
    }

    let mut msg = format!(
        "OpenAPI validation failed with {} issue(s):\n",
        errors.len()
    );
    for (i, e) in errors.iter().enumerate() {
        msg.push_str(&format!("  {}. {e}\n", i + 1));
    }
    Err(Error::adapter(msg))
}

struct ComponentCatalogs {
    schemas: BTreeSet<String>,
    responses: BTreeSet<String>,
    parameters: BTreeSet<String>,
    request_bodies: BTreeSet<String>,
}

impl ComponentCatalogs {
    fn from_spec(spec: &OpenAPI) -> Self {
        let mut out = Self {
            schemas: BTreeSet::new(),
            responses: BTreeSet::new(),
            parameters: BTreeSet::new(),
            request_bodies: BTreeSet::new(),
        };
        if let Some(c) = &spec.components {
            out.schemas.extend(c.schemas.keys().cloned());
            out.responses.extend(c.responses.keys().cloned());
            out.parameters.extend(c.parameters.keys().cloned());
            out.request_bodies.extend(c.request_bodies.keys().cloned());
        }
        out
    }

    fn contains(&self, kind: &str, name: &str) -> bool {
        match kind {
            "schemas" => self.schemas.contains(name),
            "responses" => self.responses.contains(name),
            "parameters" => self.parameters.contains(name),
            "requestBodies" => self.request_bodies.contains(name),
            // Headers / examples / links / callbacks: warn-only later if needed.
            _ => true,
        }
    }
}

fn check_duplicate_operation_ids(spec: &OpenAPI, errors: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for (path, method, op) in spec.operations() {
        let Some(id) = op.operation_id.as_deref() else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            errors.push(format!("duplicate operationId `{id}` ({method} {path})"));
        }
    }
}

fn check_path_parameters(spec: &OpenAPI, errors: &mut Vec<String>) {
    let brace_re = Regex::new(r"\{([^}/]+)\}").expect("static regex");
    for (path, item_ref) in spec.paths.iter() {
        let Some(item) = item_ref.as_item() else {
            continue;
        };
        let template_params: BTreeSet<String> = brace_re
            .captures_iter(path)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();
        if template_params.is_empty() {
            continue;
        }

        for (method, op) in item.iter() {
            let declared = declared_path_param_names(spec, item, op);
            for name in &template_params {
                if !declared.contains(name) {
                    errors.push(format!(
                        "path `{path}` {method}: template `{{{name}}}` has no matching path parameter"
                    ));
                }
            }
        }
    }
}

fn declared_path_param_names(spec: &OpenAPI, item: &PathItem, op: &Operation) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for p in item.parameters.iter().chain(op.parameters.iter()) {
        if let Some(name) = path_param_name(spec, p) {
            names.insert(name);
        }
    }
    names
}

fn path_param_name(spec: &OpenAPI, param: &ReferenceOr<Parameter>) -> Option<String> {
    let resolved = match param {
        ReferenceOr::Item(p) => Some(p),
        ReferenceOr::Reference { reference } => {
            let name = reference.strip_prefix("#/components/parameters/")?;
            match spec.components.as_ref()?.parameters.get(name)? {
                ReferenceOr::Item(p) => Some(p),
                ReferenceOr::Reference { reference: inner } => {
                    let name = inner.strip_prefix("#/components/parameters/")?;
                    match spec.components.as_ref()?.parameters.get(name)? {
                        ReferenceOr::Item(p) => Some(p),
                        ReferenceOr::Reference { .. } => None,
                    }
                }
            }
        }
    };
    match resolved? {
        Parameter::Path { parameter_data, .. } => Some(parameter_data.name.clone()),
        _ => None,
    }
}

fn check_local_refs(spec: &OpenAPI, catalogs: &ComponentCatalogs, errors: &mut Vec<String>) {
    let mut refs = BTreeSet::new();

    if let Some(c) = &spec.components {
        for schema in c.schemas.values() {
            collect_schema_refs(schema, &mut refs);
        }
        for resp in c.responses.values() {
            collect_response_refs(resp, &mut refs);
        }
        for param in c.parameters.values() {
            collect_parameter_refs(param, &mut refs);
        }
        for body in c.request_bodies.values() {
            collect_request_body_refs(body, &mut refs);
        }
    }

    for (_path, item_ref) in spec.paths.iter() {
        let Some(item) = item_ref.as_item() else {
            continue;
        };
        for param in &item.parameters {
            collect_parameter_refs(param, &mut refs);
        }
        for (_method, op) in item.iter() {
            for param in &op.parameters {
                collect_parameter_refs(param, &mut refs);
            }
            if let Some(body) = &op.request_body {
                collect_request_body_refs(body, &mut refs);
            }
            for resp in op.responses.responses.values() {
                collect_response_refs(resp, &mut refs);
            }
            if let Some(resp) = &op.responses.default {
                collect_response_refs(resp, &mut refs);
            }
        }
    }

    for r in refs {
        if let Some((kind, name)) = parse_components_ref(&r) {
            if !catalogs.contains(kind, name) {
                errors.push(format!(
                    "unresolved $ref `{r}` (no components/{kind}/{name})"
                ));
            }
        } else if r.starts_with("#/") {
            // External-ish JSON pointers we don't model — surface as warning, not hard fail.
            warn!(reference = %r, "skipping non-components local $ref during validation");
        }
    }
}

fn parse_components_ref(r: &str) -> Option<(&str, &str)> {
    let rest = r.strip_prefix("#/components/")?;
    let (kind, name) = rest.split_once('/')?;
    if name.is_empty() || name.contains('/') {
        // Nested pointers inside a schema are unusual; treat as non-catalog.
        return None;
    }
    Some((kind, name))
}

fn collect_schema_refs(schema: &ReferenceOr<Schema>, out: &mut BTreeSet<String>) {
    match schema {
        ReferenceOr::Reference { reference } => {
            out.insert(reference.clone());
        }
        ReferenceOr::Item(schema) => collect_schema_body_refs(schema, out),
    }
}

fn collect_schema_body_refs(schema: &Schema, out: &mut BTreeSet<String>) {
    match &schema.schema_kind {
        SchemaKind::Type(Type::Object(obj)) => {
            for prop in obj.properties.values() {
                match prop {
                    ReferenceOr::Reference { reference } => {
                        out.insert(reference.clone());
                    }
                    ReferenceOr::Item(s) => collect_schema_body_refs(s, out),
                }
            }
            if let Some(AdditionalProperties::Schema(s)) = &obj.additional_properties {
                collect_schema_refs(s, out);
            }
        }
        SchemaKind::Type(Type::Array(arr)) => {
            if let Some(items) = &arr.items {
                match items {
                    ReferenceOr::Reference { reference } => {
                        out.insert(reference.clone());
                    }
                    ReferenceOr::Item(s) => collect_schema_body_refs(s, out),
                }
            }
        }
        SchemaKind::OneOf { one_of }
        | SchemaKind::AllOf { all_of: one_of }
        | SchemaKind::AnyOf { any_of: one_of } => {
            for s in one_of {
                collect_schema_refs(s, out);
            }
        }
        SchemaKind::Not { not } => collect_schema_refs(not, out),
        SchemaKind::Any(any) => {
            for prop in any.properties.values() {
                match prop {
                    ReferenceOr::Reference { reference } => {
                        out.insert(reference.clone());
                    }
                    ReferenceOr::Item(s) => collect_schema_body_refs(s, out),
                }
            }
            for s in any
                .one_of
                .iter()
                .chain(any.all_of.iter())
                .chain(any.any_of.iter())
            {
                collect_schema_refs(s, out);
            }
            if let Some(items) = &any.items {
                match items {
                    ReferenceOr::Reference { reference } => {
                        out.insert(reference.clone());
                    }
                    ReferenceOr::Item(s) => collect_schema_body_refs(s, out),
                }
            }
            if let Some(AdditionalProperties::Schema(s)) = &any.additional_properties {
                collect_schema_refs(s, out);
            }
            if let Some(not) = &any.not {
                collect_schema_refs(not, out);
            }
        }
        SchemaKind::Type(_) => {}
    }
}

fn collect_parameter_refs(param: &ReferenceOr<Parameter>, out: &mut BTreeSet<String>) {
    match param {
        ReferenceOr::Reference { reference } => {
            out.insert(reference.clone());
        }
        ReferenceOr::Item(p) => {
            let data = match p {
                Parameter::Query { parameter_data, .. }
                | Parameter::Header { parameter_data, .. }
                | Parameter::Path { parameter_data, .. }
                | Parameter::Cookie { parameter_data, .. } => parameter_data,
            };
            match &data.format {
                ParameterSchemaOrContent::Schema(s) => collect_schema_refs(s, out),
                ParameterSchemaOrContent::Content(content) => {
                    for media in content.values() {
                        if let Some(schema) = &media.schema {
                            collect_schema_refs(schema, out);
                        }
                    }
                }
            }
        }
    }
}

fn collect_request_body_refs(body: &ReferenceOr<RequestBody>, out: &mut BTreeSet<String>) {
    match body {
        ReferenceOr::Reference { reference } => {
            out.insert(reference.clone());
        }
        ReferenceOr::Item(body) => {
            for media in body.content.values() {
                if let Some(schema) = &media.schema {
                    collect_schema_refs(schema, out);
                }
            }
        }
    }
}

fn collect_response_refs(resp: &ReferenceOr<Response>, out: &mut BTreeSet<String>) {
    match resp {
        ReferenceOr::Reference { reference } => {
            out.insert(reference.clone());
        }
        ReferenceOr::Item(resp) => {
            for media in resp.content.values() {
                if let Some(schema) = &media.schema {
                    collect_schema_refs(schema, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openapiv3::OpenAPI;

    fn parse(yaml: &str) -> OpenAPI {
        let mut doc: serde_json::Value =
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(yaml).unwrap()).unwrap();
        // Match production: const rewrite before openapiv3 deserialize.
        crate::adapter::parse::rewrite_const_to_enum(&mut doc);
        serde_json::from_value(doc).unwrap()
    }

    #[test]
    fn rejects_duplicate_operation_ids() {
        let spec = parse(
            r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /a:
    get:
      operationId: same
      responses: { "200": { description: ok } }
  /b:
    get:
      operationId: same
      responses: { "200": { description: ok } }
"#,
        );
        let err = validate_openapi(&spec).unwrap_err().to_string();
        assert!(err.contains("duplicate operationId `same`"), "{err}");
    }

    #[test]
    fn rejects_missing_path_param() {
        let spec = parse(
            r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /pets/{petId}:
    get:
      operationId: getPet
      responses: { "200": { description: ok } }
"#,
        );
        let err = validate_openapi(&spec).unwrap_err().to_string();
        assert!(err.contains("petId"), "{err}");
    }

    #[test]
    fn rejects_unresolved_schema_ref() {
        let spec = parse(
            r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: list
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Missing'
"#,
        );
        let err = validate_openapi(&spec).unwrap_err().to_string();
        assert!(err.contains("Missing"), "{err}");
    }

    #[test]
    fn accepts_valid_tiny_fixture() {
        let bytes = std::fs::read("tests/fixtures/tiny.openapi.yaml").unwrap();
        let ir_ok = crate::adapter::parse_openapi_with_options(
            &bytes,
            crate::adapter::ParseOptions { validate: true },
        );
        assert!(ir_ok.is_ok(), "{ir_ok:?}");
    }
}
