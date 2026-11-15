//! TypeScript types plugin → `types.ts` (Kubb plugin-ts analogue).

use std::fmt::Write as _;
use std::path::PathBuf;

use indexmap::IndexMap;

use crate::adapter::{
    Discriminator, EnumValue, OperationNode, ParameterLocation, ParameterNode, ParsedSpec,
    PrimitiveKind, SchemaNode,
};
use serde::Deserialize;

use crate::config::{CaseStyle, Config, OverrideKind, OverrideMatch};
use crate::error::Result;
use crate::naming::{Naming, Override, OverrideSet, to_case};
use crate::plugin::{OutputFile, Plugin, PluginContext};

pub struct TypesPlugin;

/// `[plugins.types]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypesOptions {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_file")]
    pub file: String,
    #[serde(default = "default_enum_style")]
    pub enum_style: EnumStyle,
    /// How to declare object schemas (`type` stays drop-in with Kubb).
    /// Unions / intersections / nullable objects always fall back to `type`.
    #[serde(default)]
    pub syntax: ObjectSyntax,
    #[serde(default = "default_true")]
    pub strip_readonly: bool,
    #[serde(default = "default_true")]
    pub compact: bool,
    /// Per-schema `enum_style` / `syntax`, matched by schema name.
    #[serde(default)]
    pub overrides: Vec<TypesOverride>,
}

/// How enum schemas are declared.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnumStyle {
    Enum,
    Literal,
    AsConst,
}

/// Top-level declaration style for object-like schemas.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ObjectSyntax {
    /// `export type Foo = { … }`, the default.
    #[default]
    Type,
    /// `export interface Foo { … }` when the schema is a non-nullable object.
    Interface,
}

/// `[[plugins.types.overrides]]`: a different enum style or object syntax for
/// the schemas whose name matches.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypesOverride {
    #[serde(rename = "match")]
    pub target: OverrideMatch,
    #[serde(default)]
    pub enum_style: Option<EnumStyle>,
    #[serde(default)]
    pub syntax: Option<ObjectSyntax>,
}

impl Override for TypesOverride {
    fn target(&self) -> &OverrideMatch {
        &self.target
    }
}

/// The enum style and object syntax for each schema, after overrides.
#[derive(Debug)]
pub struct TypeStyles {
    enum_style: EnumStyle,
    syntax: ObjectSyntax,
    overrides: OverrideSet<TypesOverride>,
}

impl TypeStyles {
    pub fn enum_style(&self, schema_name: &str) -> EnumStyle {
        self.overrides
            .matching(OverrideKind::SchemaName, schema_name)
            .find_map(|ovr| ovr.enum_style)
            .unwrap_or(self.enum_style)
    }

    pub fn syntax(&self, schema_name: &str) -> ObjectSyntax {
        self.overrides
            .matching(OverrideKind::SchemaName, schema_name)
            .find_map(|ovr| ovr.syntax)
            .unwrap_or(self.syntax)
    }
}

fn default_true() -> bool {
    true
}

fn default_file() -> String {
    "types.ts".into()
}

fn default_enum_style() -> EnumStyle {
    EnumStyle::Enum
}

impl Default for TypesOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            file: default_file(),
            enum_style: default_enum_style(),
            syntax: ObjectSyntax::Type,
            strip_readonly: true,
            compact: true,
            overrides: Vec::new(),
        }
    }
}

impl TypesOptions {
    pub fn from_config(config: &Config) -> Result<Self> {
        config.plugins.section_or_default(TypesPlugin.name())
    }

    pub fn styles(&self) -> Result<TypeStyles> {
        Ok(TypeStyles {
            enum_style: self.enum_style,
            syntax: self.syntax,
            overrides: OverrideSet::new(self.overrides.clone(), "plugins.types.overrides")?,
        })
    }

    /// How other generated files import the types file.
    pub fn import_path(&self) -> String {
        super::relative_import(&self.file)
    }
}

/// The names the types plugin gives one operation's types. Other plugins
/// import these from the types file, so they take the names from here.
pub struct OperationTypeNames<'a> {
    base: String,
    operation: &'a OperationNode,
}

impl<'a> OperationTypeNames<'a> {
    pub fn new(naming: &Naming, operation: &'a OperationNode) -> Self {
        Self {
            base: naming.operation_type_base(&operation.operation_id),
            operation,
        }
    }

    fn is_query(&self) -> bool {
        self.operation.method.eq_ignore_ascii_case("get")
    }

    /// The response type for one status code (`GetPet200`).
    pub fn response(&self, status: &str) -> String {
        format!("{}{status}", self.base)
    }

    /// The first 2xx status, which the success alias points at.
    pub fn success_status(&self) -> Option<&'a str> {
        self.operation
            .responses
            .keys()
            .find(|code| code.starts_with('2'))
            .map(String::as_str)
    }

    /// The success alias (`GetPetQueryResponse`, `UpdatePetMutationResponse`),
    /// written when the operation has a 2xx response.
    pub fn success_response(&self) -> Option<String> {
        self.success_status()?;
        let kind = if self.is_query() {
            "QueryResponse"
        } else {
            "MutationResponse"
        };
        Some(format!("{}{kind}", self.base))
    }

    /// The request body type (`UpdatePetMutationRequest`), written when the
    /// operation has a body.
    pub fn request(&self) -> Option<String> {
        self.operation.request_body.as_ref()?;
        let kind = if self.is_query() {
            "QueryRequest"
        } else {
            "MutationRequest"
        };
        Some(format!("{}{kind}", self.base))
    }

    /// The type grouping the parameters in one location (`GetPetPathParams`).
    /// Cookie parameters get no type yet.
    pub fn params(&self, location: ParameterLocation) -> Option<String> {
        let group = match location {
            ParameterLocation::Query => "QueryParams",
            ParameterLocation::Path => "PathParams",
            ParameterLocation::Header => "HeaderParams",
            ParameterLocation::Cookie => return None,
        };
        Some(format!("{}{group}", self.base))
    }
}

impl Plugin for TypesPlugin {
    fn name(&self) -> &'static str {
        "types"
    }

    fn enabled(&self, config: &Config) -> Result<bool> {
        Ok(TypesOptions::from_config(config)?.enabled)
    }

    fn output_files(&self, config: &Config) -> Result<Vec<PathBuf>> {
        Ok(vec![PathBuf::from(TypesOptions::from_config(config)?.file)])
    }

    fn generate(&self, ctx: &PluginContext<'_>) -> Result<Vec<OutputFile>> {
        let options = TypesOptions::from_config(ctx.config)?;
        let body = render_types(ctx.spec, ctx.naming, &options)?;
        let contents = if options.compact {
            compact_blank_lines(&body)
        } else {
            body
        };

        Ok(vec![OutputFile {
            relative_path: PathBuf::from(options.file),
            contents,
        }])
    }
}

fn render_types(spec: &ParsedSpec, naming: &Naming, types_cfg: &TypesOptions) -> Result<String> {
    let mut out = String::new();
    let styles = types_cfg.styles()?;

    for (name, schema) in &spec.schemas {
        let type_name = naming.schema_type_name(name);
        let enum_style = styles.enum_style(name);
        let syntax = styles.syntax(name);
        write_declaration(
            &mut out,
            name,
            &type_name,
            schema,
            &TypeRenderCtx {
                naming,
                enum_style,
                syntax,
                // Named component schemas keep readOnly fields (used as responses).
                // Request bodies strip them separately below when configured.
                strip_readonly: false,
                schemas: &spec.schemas,
            },
        )?;
        out.push('\n');
    }

    // Operation auxiliary types (query/path/header params + response aliases).
    for op in &spec.operations {
        let names = OperationTypeNames::new(naming, op);

        // Header params are additive and safe even when unused (empty groups are skipped).
        for location in [
            ParameterLocation::Query,
            ParameterLocation::Path,
            ParameterLocation::Header,
        ] {
            let group_name = names
                .params(location)
                .expect("written locations have a name");
            write_param_group(
                &mut out,
                &group_name,
                &op.parameters,
                location,
                naming,
                &spec.schemas,
            )?;
        }

        for (code, schema) in &op.responses {
            write_response_type(
                &mut out,
                &names.response(code),
                schema,
                naming,
                &spec.schemas,
            )?;
        }

        if let (Some(alias), Some(code)) = (names.success_response(), names.success_status()) {
            let _ = writeln!(out, "export type {alias} = {};", names.response(code));
            out.push('\n');
        }

        if let (Some(request), Some(body)) = (names.request(), &op.request_body) {
            let ts = schema_to_ts(body, naming, types_cfg.strip_readonly, &spec.schemas)?;
            let _ = writeln!(out, "export type {request} = {ts};");
            out.push('\n');
        }
    }

    Ok(out)
}

/// Emit `{Base}QueryParams` / `{Base}PathParams`, plus hoisted enums for enum params
/// (e.g. `ImportExportPipelinesExportJobsCreatePathParamsPipelineSlug`).
fn write_param_group(
    out: &mut String,
    group_name: &str,
    parameters: &[ParameterNode],
    location: ParameterLocation,
    naming: &Naming,
    schemas: &indexmap::IndexMap<String, SchemaNode>,
) -> Result<()> {
    let params: Vec<_> = parameters
        .iter()
        .filter(|p| p.location == location)
        .collect();
    if params.is_empty() {
        return Ok(());
    }

    let mut fields = Vec::new();
    for p in &params {
        let ts = if let Some(values) = enum_values_of(&p.schema) {
            let enum_name = format!("{group_name}{}", to_case(&p.name, CaseStyle::PascalCase));
            write_string_enum(out, &enum_name, values, naming)?;
            out.push('\n');
            enum_name
        } else {
            schema_to_ts(&p.schema, naming, false, schemas)?
        };
        fields.push((p.name.as_str(), !p.required, ts));
    }

    let _ = writeln!(out, "export type {group_name} = {{");
    for (name, optional, ts) in fields {
        let key = ts_property_key(name);
        if optional {
            let _ = writeln!(out, "  {key}?: {ts};");
        } else {
            let _ = writeln!(out, "  {key}: {ts};");
        }
    }
    let _ = writeln!(out, "}};");
    out.push('\n');
    Ok(())
}

/// Emit response type and, when present, Kubb-style `{Base}{code}ErrorCode` enum
/// extracted from `error_code` properties (including oneOf arms).
fn write_response_type(
    out: &mut String,
    type_name: &str,
    schema: &SchemaNode,
    naming: &Naming,
    schemas: &indexmap::IndexMap<String, SchemaNode>,
) -> Result<()> {
    let error_code_name = format!("{type_name}ErrorCode");

    if let Some(values) = collect_error_code_values(schema)
        && !values.is_empty()
    {
        write_string_enum(out, &error_code_name, &values, naming)?;
        out.push('\n');
        let rewritten = rewrite_error_code_refs(schema, &error_code_name);
        let rewritten = omit_writeonly_fields(&rewritten);
        let ts = schema_to_ts(&rewritten, naming, false, schemas)?;
        let _ = writeln!(out, "export type {type_name} = {ts};");
        out.push('\n');
        return Ok(());
    }

    let schema = omit_writeonly_fields(schema);
    let ts = schema_to_ts(&schema, naming, false, schemas)?;
    let _ = writeln!(out, "export type {type_name} = {ts};");
    out.push('\n');
    Ok(())
}

/// Drop `writeOnly` props from inlined response shapes (Kubb keysToOmit). Named
/// `$ref` aliases are left alone so Create/Input component types keep write fields.
fn omit_writeonly_fields(schema: &SchemaNode) -> SchemaNode {
    match schema {
        SchemaNode::Object {
            properties,
            required,
            readonly_properties,
            writeonly_properties,
            additional_properties,
            nullable,
            description,
        } => {
            let properties: indexmap::IndexMap<_, _> = properties
                .iter()
                .filter(|(name, _)| !writeonly_properties.iter().any(|w| w == *name))
                .map(|(k, v)| (k.clone(), omit_writeonly_fields(v)))
                .collect();
            let required: Vec<_> = required
                .iter()
                .filter(|name| !writeonly_properties.iter().any(|w| w == *name))
                .cloned()
                .collect();
            SchemaNode::Object {
                properties,
                required,
                readonly_properties: readonly_properties.clone(),
                writeonly_properties: vec![],
                additional_properties: additional_properties
                    .as_ref()
                    .map(|a| Box::new(omit_writeonly_fields(a))),
                nullable: *nullable,
                description: description.clone(),
            }
        }
        SchemaNode::Array {
            items,
            nullable,
            description,
        } => SchemaNode::Array {
            items: Box::new(omit_writeonly_fields(items)),
            nullable: *nullable,
            description: description.clone(),
        },
        SchemaNode::AllOf { items, nullable } => SchemaNode::AllOf {
            items: items.iter().map(omit_writeonly_fields).collect(),
            nullable: *nullable,
        },
        SchemaNode::OneOf {
            items,
            nullable,
            discriminator,
        } => SchemaNode::OneOf {
            items: items.iter().map(omit_writeonly_fields).collect(),
            nullable: *nullable,
            discriminator: discriminator.clone(),
        },
        SchemaNode::AnyOf { items, nullable } => SchemaNode::AnyOf {
            items: items.iter().map(omit_writeonly_fields).collect(),
            nullable: *nullable,
        },
        other => other.clone(),
    }
}

fn write_string_enum(
    out: &mut String,
    type_name: &str,
    values: &[EnumValue],
    naming: &Naming,
) -> Result<()> {
    let _ = writeln!(out, "export enum {type_name} {{");
    for value in values {
        match value {
            EnumValue::String(s) => {
                let member = naming.enum_member_name(type_name, s);
                let _ = writeln!(out, "  {member} = '{s}',");
            }
            EnumValue::Number(n) => {
                let member = format!("Value{n}").replace('.', "_");
                let _ = writeln!(out, "  {member} = {n},");
            }
            EnumValue::Bool(b) => {
                let member = if *b { "True" } else { "False" };
                let _ = writeln!(out, "  {member} = {b},");
            }
            EnumValue::Null => {
                let _ = writeln!(out, "  Null = null,");
            }
        }
    }
    let _ = writeln!(out, "}}");
    Ok(())
}

fn enum_values_of(schema: &SchemaNode) -> Option<&[EnumValue]> {
    match schema {
        SchemaNode::Enum { values, .. } => Some(values.as_slice()),
        SchemaNode::AllOf { items, .. } if items.len() == 1 => enum_values_of(&items[0]),
        _ => None,
    }
}

/// Gather distinct `error_code` enum values from a response schema (object or oneOf/anyOf).
fn collect_error_code_values(schema: &SchemaNode) -> Option<Vec<EnumValue>> {
    let mut out = Vec::new();
    collect_error_code_values_into(schema, &mut out);
    if out.is_empty() { None } else { Some(out) }
}

fn collect_error_code_values_into(schema: &SchemaNode, out: &mut Vec<EnumValue>) {
    match schema {
        SchemaNode::Object { properties, .. } => {
            if let Some(code_schema) = properties.get("error_code")
                && let Some(values) = enum_values_of(code_schema)
            {
                for v in values {
                    if !out.contains(v) {
                        out.push(v.clone());
                    }
                }
            }
        }
        SchemaNode::OneOf { items, .. } | SchemaNode::AnyOf { items, .. } => {
            for item in items {
                collect_error_code_values_into(item, out);
            }
        }
        SchemaNode::AllOf { items, .. } => {
            for item in items {
                collect_error_code_values_into(item, out);
            }
        }
        _ => {}
    }
}

/// Replace inline `error_code` enums with a reference to the hoisted `{…}ErrorCode` type name.
fn rewrite_error_code_refs(schema: &SchemaNode, error_code_name: &str) -> SchemaNode {
    match schema {
        SchemaNode::Object {
            properties,
            required,
            readonly_properties,
            writeonly_properties,
            additional_properties,
            nullable,
            description,
        } => {
            let mut properties = properties.clone();
            if let Some(code_schema) = properties.get("error_code")
                && enum_values_of(code_schema).is_some()
            {
                let code_nullable = match code_schema {
                    SchemaNode::Enum { nullable, .. } => *nullable,
                    SchemaNode::AllOf { nullable, .. } => *nullable,
                    _ => false,
                };
                let ref_node = SchemaNode::Ref {
                    name: error_code_name.to_string(),
                };
                properties.insert(
                    "error_code".into(),
                    if code_nullable {
                        SchemaNode::AllOf {
                            items: vec![ref_node],
                            nullable: true,
                        }
                    } else {
                        ref_node
                    },
                );
            }
            SchemaNode::Object {
                properties,
                required: required.clone(),
                readonly_properties: readonly_properties.clone(),
                writeonly_properties: writeonly_properties.clone(),
                additional_properties: additional_properties.clone(),
                nullable: *nullable,
                description: description.clone(),
            }
        }
        SchemaNode::OneOf {
            items,
            nullable,
            discriminator,
        } => SchemaNode::OneOf {
            items: items
                .iter()
                .map(|i| rewrite_error_code_refs(i, error_code_name))
                .collect(),
            nullable: *nullable,
            discriminator: discriminator.clone(),
        },
        SchemaNode::AnyOf { items, nullable } => SchemaNode::AnyOf {
            items: items
                .iter()
                .map(|i| rewrite_error_code_refs(i, error_code_name))
                .collect(),
            nullable: *nullable,
        },
        SchemaNode::AllOf { items, nullable } => SchemaNode::AllOf {
            items: items
                .iter()
                .map(|i| rewrite_error_code_refs(i, error_code_name))
                .collect(),
            nullable: *nullable,
        },
        other => other.clone(),
    }
}

/// Shared options for emitting one schema declaration (keeps args readable).
struct TypeRenderCtx<'a> {
    naming: &'a Naming,
    enum_style: EnumStyle,
    syntax: ObjectSyntax,
    strip_readonly: bool,
    schemas: &'a indexmap::IndexMap<String, SchemaNode>,
}

fn write_declaration(
    out: &mut String,
    schema_name: &str,
    type_name: &str,
    schema: &SchemaNode,
    ctx: &TypeRenderCtx<'_>,
) -> Result<()> {
    let naming = ctx.naming;
    let enum_style = ctx.enum_style;
    let syntax = ctx.syntax;
    let strip_readonly = ctx.strip_readonly;
    let schemas = ctx.schemas;
    match (schema, enum_style) {
        (SchemaNode::Enum { values, .. }, EnumStyle::Enum) => {
            let _ = writeln!(out, "export enum {type_name} {{");
            for value in values {
                match value {
                    EnumValue::String(s) => {
                        let member = naming.enum_member_name_for_schema(schema_name, type_name, s);
                        let _ = writeln!(out, "  {member} = '{s}',");
                    }
                    EnumValue::Number(n) => {
                        let member = format!("Value{n}").replace('.', "_");
                        let _ = writeln!(out, "  {member} = {n},");
                    }
                    EnumValue::Bool(b) => {
                        let member = if *b { "True" } else { "False" };
                        let _ = writeln!(out, "  {member} = {b},");
                    }
                    EnumValue::Null => {
                        let _ = writeln!(out, "  Null = null,");
                    }
                }
            }
            let _ = writeln!(out, "}}");
        }
        (SchemaNode::Enum { values, .. }, EnumStyle::Literal | EnumStyle::AsConst)
            if values.len() == 1 =>
        {
            // Discriminator-style single-value enums → string literal type.
            let ts = schema_to_ts(schema, naming, strip_readonly, schemas)?;
            let _ = writeln!(out, "export type {type_name} = {ts};");
        }
        (
            SchemaNode::Object {
                nullable: false, ..
            },
            _,
        ) if syntax == ObjectSyntax::Interface => {
            let body = object_interface_body(schema, naming, strip_readonly, schemas)?;
            let _ = writeln!(out, "export interface {type_name} {body}");
        }
        _ => {
            let ts = schema_to_ts(schema, naming, strip_readonly, schemas)?;
            let _ = writeln!(out, "export type {type_name} = {ts};");
        }
    }
    Ok(())
}

/// Object property block for `interface` (no nullable wrapper — caller guards that).
fn object_interface_body(
    schema: &SchemaNode,
    naming: &Naming,
    strip_readonly: bool,
    schemas: &indexmap::IndexMap<String, SchemaNode>,
) -> Result<String> {
    let SchemaNode::Object {
        properties,
        required,
        readonly_properties,
        writeonly_properties: _,
        additional_properties,
        ..
    } = schema
    else {
        return Err(crate::error::Error::plugin(
            "types",
            "interface body requires an object schema",
        ));
    };

    let mut body = String::from("{\n");
    for (prop, prop_schema) in properties {
        if strip_readonly && readonly_properties.iter().any(|r| r == prop) {
            continue;
        }
        let optional = !required.iter().any(|r| r == prop);
        let ts = schema_to_ts(prop_schema, naming, strip_readonly, schemas)?;
        let key = ts_property_key(prop);
        if let Some(desc) = prop_schema_description(prop_schema) {
            let _ = writeln!(body, "  /**");
            let _ = writeln!(body, "   * {desc}");
            let _ = writeln!(body, "   */");
        }
        if optional {
            let _ = writeln!(body, "  {key}?: {ts};");
        } else {
            let _ = writeln!(body, "  {key}: {ts};");
        }
    }
    if let Some(additional) = additional_properties {
        let ts = schema_to_ts(additional, naming, strip_readonly, schemas)?;
        let _ = writeln!(body, "  [key: string]: {ts};");
    }
    body.push('}');
    Ok(body)
}

fn schema_to_ts(
    schema: &SchemaNode,
    naming: &Naming,
    strip_readonly: bool,
    schemas: &indexmap::IndexMap<String, SchemaNode>,
) -> Result<String> {
    match schema {
        SchemaNode::Ref { name } => {
            // Request bodies that $ref a component must expand objects so we can
            // omit readOnly fields (Kubb stripReadonly). Keep enum/primitive aliases
            // (`LanguageEnum[]`, not `'en' | 'nl'[]`) — expanding those was both a
            // paren bug and broke assignability from named input types.
            if strip_readonly
                && let Some(resolved) = schemas.get(name)
                && ref_should_expand_for_strip_readonly(resolved)
            {
                return schema_to_ts(resolved, naming, true, schemas);
            }
            Ok(naming.schema_type_name(name))
        }
        SchemaNode::Primitive {
            primitive,
            nullable,
            ..
        } => {
            let base = match primitive {
                PrimitiveKind::String => "string",
                PrimitiveKind::Number | PrimitiveKind::Integer => "number",
                PrimitiveKind::Boolean => "boolean",
                PrimitiveKind::Null => "null",
            };
            Ok(nullable_type(base, *nullable))
        }
        SchemaNode::Array {
            items, nullable, ..
        } => {
            // Array-level `nullable` → `T[] | null`. Item-level `nullable` on a
            // primitive/ref is usually a schema quirk (DRF etc.); Kubb emitted
            // `string[]` not `(string | null)[]`. Strip only the item *root*
            // nullability so nested object fields keep `| null`.
            let item = items_without_root_nullable(items);
            let inner = schema_to_ts(&item, naming, strip_readonly, schemas)?;
            // `A | B[]` ≠ `(A | B)[]`. Classify by resolved item shape when
            // strip_readonly expands refs (Ref→Enum must still get parens).
            let shape = array_item_shape(&item, strip_readonly, schemas);
            let array_ty = match shape {
                ArrayItemShape::UnionOrIntersection => format!("({inner})[]"),
                ArrayItemShape::Plain => format!("{inner}[]"),
            };
            Ok(nullable_type(&array_ty, *nullable))
        }
        SchemaNode::Object {
            properties,
            required,
            readonly_properties,
            writeonly_properties: _,
            additional_properties,
            nullable,
            ..
        } => {
            // Free-form blobs (`table_config: {}`, nullable value with no type) → `any`
            // for Kubb parity. `Record<string, unknown>` breaks spreads/indexing in app code.
            if properties.is_empty() {
                if let Some(additional) = additional_properties {
                    match additional.as_ref() {
                        SchemaNode::Unknown { .. } => {
                            return Ok(nullable_type("any", *nullable));
                        }
                        other => {
                            let ts = schema_to_ts(other, naming, strip_readonly, schemas)?;
                            return Ok(nullable_type(&format!("Record<string, {ts}>"), *nullable));
                        }
                    }
                }
                return Ok(nullable_type("any", *nullable));
            }
            let mut body = String::from("{\n");
            for (prop, prop_schema) in properties {
                if strip_readonly && readonly_properties.iter().any(|r| r == prop) {
                    continue;
                }
                let optional = !required.iter().any(|r| r == prop);
                let ts = schema_to_ts(prop_schema, naming, strip_readonly, schemas)?;
                let key = ts_property_key(prop);
                if let Some(desc) = prop_schema_description(prop_schema) {
                    let _ = writeln!(body, "  /**");
                    let _ = writeln!(body, "   * {desc}");
                    let _ = writeln!(body, "   */");
                }
                if optional {
                    let _ = writeln!(body, "  {key}?: {ts};");
                } else {
                    let _ = writeln!(body, "  {key}: {ts};");
                }
            }
            if let Some(additional) = additional_properties {
                let ts = schema_to_ts(additional, naming, strip_readonly, schemas)?;
                let _ = writeln!(body, "  [key: string]: {ts};");
            }
            body.push('}');
            Ok(nullable_type(&body, *nullable))
        }
        SchemaNode::Enum {
            values, nullable, ..
        } => {
            let parts: Vec<String> = values
                .iter()
                .map(|v| match v {
                    EnumValue::String(s) => format!("'{s}'"),
                    EnumValue::Number(n) => n.to_string(),
                    EnumValue::Bool(b) => b.to_string(),
                    EnumValue::Null => "null".into(),
                })
                .collect();
            let union = if parts.is_empty() {
                "never".into()
            } else {
                parts.join(" | ")
            };
            Ok(nullable_type(&union, *nullable))
        }
        SchemaNode::AllOf { items, nullable } => {
            // Kubb: flatten single-member; merge adjacent object literals into one
            // shape so `A & {…} & {…}` doesn't leave shapeless gaps for factories.
            if items.len() == 1 {
                let ts = schema_to_ts(&items[0], naming, strip_readonly, schemas)?;
                return Ok(nullable_type(&ts, *nullable));
            }
            let merged = merge_adjacent_allof_objects(items);
            let parts = merged
                .iter()
                .map(|i| schema_to_ts(i, naming, strip_readonly, schemas))
                .collect::<Result<Vec<_>>>()?;
            Ok(nullable_type(&parts.join(" & "), *nullable))
        }
        SchemaNode::OneOf {
            items,
            nullable,
            discriminator,
        } => {
            // `anyOf`/`oneOf` null arms → `T | null`. Discriminator mapping →
            // Kubb preserve: `(Arm & { prop: 'value' }) | …`.
            let mut saw_null = *nullable;
            let non_null: Vec<&SchemaNode> = items
                .iter()
                .filter(|i| {
                    if is_null_schema(i) {
                        saw_null = true;
                        false
                    } else {
                        true
                    }
                })
                .collect();
            if non_null.is_empty() {
                return Ok(if saw_null {
                    "null".into()
                } else {
                    "never".into()
                });
            }
            let parts = discriminated_union_parts(
                &non_null,
                discriminator.as_ref(),
                naming,
                strip_readonly,
                schemas,
            )?;
            Ok(nullable_type(&parts.join(" | "), saw_null))
        }
        SchemaNode::AnyOf { items, nullable } => {
            let mut saw_null = *nullable;
            let non_null: Vec<&SchemaNode> = items
                .iter()
                .filter(|i| {
                    if is_null_schema(i) {
                        saw_null = true;
                        false
                    } else {
                        true
                    }
                })
                .collect();
            if non_null.is_empty() {
                return Ok(if saw_null {
                    "null".into()
                } else {
                    "never".into()
                });
            }
            let parts = non_null
                .iter()
                .map(|i| schema_to_ts(i, naming, strip_readonly, schemas))
                .collect::<Result<Vec<_>>>()?;
            Ok(nullable_type(&parts.join(" | "), saw_null))
        }
        // Kubb default for unconstrained schemas is `any` (not `unknown`).
        SchemaNode::Unknown { .. } => Ok("any".into()),
    }
}

fn prop_schema_description(schema: &SchemaNode) -> Option<&str> {
    match schema {
        SchemaNode::Object { description, .. }
        | SchemaNode::Array { description, .. }
        | SchemaNode::Enum { description, .. }
        | SchemaNode::Primitive { description, .. }
        | SchemaNode::Unknown { description } => description.as_deref(),
        _ => None,
    }
}

fn is_null_schema(schema: &SchemaNode) -> bool {
    matches!(
        schema,
        SchemaNode::Primitive {
            primitive: PrimitiveKind::Null,
            ..
        }
    )
}

/// Expand object-like refs under `strip_readonly`; keep enum/primitive aliases.
fn ref_should_expand_for_strip_readonly(resolved: &SchemaNode) -> bool {
    match resolved {
        SchemaNode::Object { .. }
        | SchemaNode::AllOf { .. }
        | SchemaNode::OneOf { .. }
        | SchemaNode::AnyOf { .. }
        | SchemaNode::Array { .. } => true,
        SchemaNode::Enum { .. }
        | SchemaNode::Primitive { .. }
        | SchemaNode::Unknown { .. }
        | SchemaNode::Ref { .. } => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayItemShape {
    /// Needs `(T)[]` so `|` / `&` bind correctly.
    UnionOrIntersection,
    Plain,
}

fn array_item_shape(
    item: &SchemaNode,
    strip_readonly: bool,
    schemas: &IndexMap<String, SchemaNode>,
) -> ArrayItemShape {
    // Only follow refs we actually expand; kept aliases (`LanguageEnum`) are Plain.
    let resolved = if strip_readonly {
        match item {
            SchemaNode::Ref { name } => match schemas.get(name) {
                Some(r) if ref_should_expand_for_strip_readonly(r) => r,
                _ => item,
            },
            other => other,
        }
    } else {
        item
    };
    match resolved {
        SchemaNode::OneOf { .. } | SchemaNode::AnyOf { .. } | SchemaNode::AllOf { .. } => {
            ArrayItemShape::UnionOrIntersection
        }
        SchemaNode::Enum { values, .. } if values.len() > 1 => ArrayItemShape::UnionOrIntersection,
        SchemaNode::Enum { values, .. } if values.iter().any(|v| matches!(v, EnumValue::Null)) => {
            ArrayItemShape::UnionOrIntersection
        }
        _ => ArrayItemShape::Plain,
    }
}

/// Kubb preserve: each mapped arm becomes `(Arm & { discProp: 'value' })`.
fn discriminated_union_parts(
    items: &[&SchemaNode],
    discriminator: Option<&Discriminator>,
    naming: &Naming,
    strip_readonly: bool,
    schemas: &IndexMap<String, SchemaNode>,
) -> Result<Vec<String>> {
    let Some(disc) = discriminator else {
        return items
            .iter()
            .map(|i| schema_to_ts(i, naming, strip_readonly, schemas))
            .collect();
    };

    let mut parts = Vec::with_capacity(items.len());
    let mut used_values = std::collections::HashSet::new();

    // Prefer mapping order for stable output.
    for (value, schema_name) in &disc.mapping {
        let Some(item) = items.iter().find(|i| match i {
            SchemaNode::Ref { name } => name == schema_name,
            _ => false,
        }) else {
            continue;
        };
        used_values.insert(value.as_str());
        let base = schema_to_ts(item, naming, strip_readonly, schemas)?;
        let lit = ts_string_literal(value);
        parts.push(format!("({base} & {{ {}: {} }})", disc.property_name, lit));
    }

    for item in items {
        let SchemaNode::Ref { name } = item else {
            parts.push(schema_to_ts(item, naming, strip_readonly, schemas)?);
            continue;
        };
        if disc.mapping.values().any(|n| n == name) {
            continue; // already emitted from mapping
        }
        // Implicit discriminant: schema name as value when mapping omits the arm.
        let base = schema_to_ts(item, naming, strip_readonly, schemas)?;
        let lit = ts_string_literal(name);
        parts.push(format!("({base} & {{ {}: {} }})", disc.property_name, lit));
    }

    let _ = used_values;
    Ok(parts)
}

fn ts_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Quote property keys that aren't valid TS identifiers (e.g. `X-Client-Token`).
fn ts_property_key(name: &str) -> String {
    if is_ts_identifier(name) {
        name.to_string()
    } else {
        ts_string_literal(name)
    }
}

fn is_ts_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c == '$' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

/// Merge consecutive inlined object members of an `allOf` (Kubb adjacent merge).
/// `$ref` / non-object members stay as separate intersection arms.
fn merge_adjacent_allof_objects(items: &[SchemaNode]) -> Vec<SchemaNode> {
    let mut out: Vec<SchemaNode> = Vec::new();
    for item in items {
        match (out.last_mut(), item) {
            (
                Some(SchemaNode::Object {
                    properties: left_props,
                    required: left_req,
                    readonly_properties: left_ro,
                    writeonly_properties: left_wo,
                    additional_properties: left_ap,
                    nullable: left_null,
                    description: left_desc,
                }),
                SchemaNode::Object {
                    properties: right_props,
                    required: right_req,
                    readonly_properties: right_ro,
                    writeonly_properties: right_wo,
                    additional_properties: right_ap,
                    nullable: right_null,
                    description: right_desc,
                },
            ) => {
                for (k, v) in right_props {
                    left_props.insert(k.clone(), v.clone());
                }
                for r in right_req {
                    if !left_req.iter().any(|x| x == r) {
                        left_req.push(r.clone());
                    }
                }
                for r in right_ro {
                    if !left_ro.iter().any(|x| x == r) {
                        left_ro.push(r.clone());
                    }
                }
                for w in right_wo {
                    if !left_wo.iter().any(|x| x == w) {
                        left_wo.push(w.clone());
                    }
                }
                if left_ap.is_none() {
                    *left_ap = right_ap.clone();
                }
                *left_null = *left_null || *right_null;
                if left_desc.is_none() {
                    *left_desc = right_desc.clone();
                }
            }
            _ => out.push(item.clone()),
        }
    }
    out
}

fn nullable_type(base: &str, nullable: bool) -> String {
    if !nullable {
        return base.to_string();
    }
    // Parens belong on array-of-union at the array site: `(A | B)[] | null`.
    // Only wrap here when `| null` would bind into a function return type.
    if base.contains("=>") {
        format!("({base}) | null")
    } else {
        format!("{base} | null")
    }
}

/// Clear `nullable` on the schema node itself (not nested properties).
fn items_without_root_nullable(items: &SchemaNode) -> SchemaNode {
    let mut item = items.clone();
    match &mut item {
        SchemaNode::Ref { .. } | SchemaNode::Unknown { .. } => {}
        SchemaNode::Object { nullable, .. }
        | SchemaNode::Array { nullable, .. }
        | SchemaNode::Enum { nullable, .. }
        | SchemaNode::Primitive { nullable, .. }
        | SchemaNode::AllOf { nullable, .. }
        | SchemaNode::OneOf { nullable, .. }
        | SchemaNode::AnyOf { nullable, .. } => *nullable = false,
    }
    item
}

fn compact_blank_lines(input: &str) -> String {
    input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
