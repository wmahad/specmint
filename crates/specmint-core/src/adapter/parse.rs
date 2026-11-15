//! Parse OpenAPI 3.0 YAML/JSON into [`ParsedSpec`] via the typed [`openapiv3`] crate.
//!
//! OpenAPI 3.1 `const` is rewritten to `enum: [value]` + `x-const: true` before
//! deserialize — `openapiv3` drops bare `const` (not part of 3.0).

use indexmap::IndexMap;
use openapiv3::{
    AdditionalProperties, IntegerFormat, NumberFormat, OpenAPI, Parameter,
    ParameterSchemaOrContent, ReferenceOr, Schema, SchemaKind, StatusCode, StringFormat, Type,
    VariantOrUnknownOrEmpty,
};
use serde_json::Value;

use crate::adapter::model::*;
use crate::adapter::validate::validate_openapi;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    /// Run semantic OpenAPI checks after typed deserialize.
    pub validate: bool,
}

pub fn parse_openapi(bytes: &[u8]) -> Result<ParsedSpec> {
    parse_openapi_with_options(bytes, ParseOptions::default())
}

pub fn parse_openapi_with_options(bytes: &[u8], opts: ParseOptions) -> Result<ParsedSpec> {
    let spec = load_openapi(bytes)?;
    if opts.validate {
        validate_openapi(&spec)?;
    }
    convert_spec(spec)
}

pub(crate) fn load_openapi(bytes: &[u8]) -> Result<OpenAPI> {
    let mut doc = load_document(bytes)?;
    // Preserve 3.1-style const discriminators that openapiv3 would otherwise drop.
    rewrite_const_to_enum(&mut doc);
    // OAS 3.1 `type: ["string", "null"]` → `type: string` + `nullable: true`.
    rewrite_nullable_type_arrays(&mut doc);

    let bytes = serde_json::to_vec(&doc)?;
    let mut de = serde_json::Deserializer::from_slice(&bytes);
    serde_path_to_error::deserialize(&mut de).map_err(|e| {
        Error::adapter(format!(
            "failed to deserialize OpenAPI 3.0 document at `{}`: {e}",
            e.path()
        ))
    })
}

fn convert_spec(spec: OpenAPI) -> Result<ParsedSpec> {
    let meta = SpecMeta {
        title: Some(spec.info.title.clone()),
        version: Some(spec.info.version.clone()),
        openapi: Some(spec.openapi.clone()),
    };

    let mut schemas = IndexMap::new();
    if let Some(components) = &spec.components {
        for (name, schema_or_ref) in &components.schemas {
            schemas.insert(name.clone(), convert_ref_or_schema(schema_or_ref, name)?);
        }
    }

    let components = ComponentsCtx::from_spec(&spec);

    let mut operations = Vec::new();
    for (path, path_item_ref) in spec.paths.iter() {
        let Some(path_item) = path_item_ref.as_item() else {
            continue;
        };
        for (method, op) in path_item.iter() {
            operations.push(convert_operation(
                path,
                method,
                op,
                &path_item.parameters,
                &components,
            )?);
        }
    }

    Ok(ParsedSpec {
        meta,
        schemas,
        operations,
    })
}

/// Resolve `#/components/{parameters|requestBodies|responses}/Name` for operations.
struct ComponentsCtx<'a> {
    components: Option<&'a openapiv3::Components>,
}

impl<'a> ComponentsCtx<'a> {
    fn from_spec(spec: &'a OpenAPI) -> Self {
        Self {
            components: spec.components.as_ref(),
        }
    }

    fn parameter(&self, reference: &str) -> Option<&Parameter> {
        let name = component_ref_name(reference, "parameters")?;
        let entry = self.components?.parameters.get(name)?;
        resolve_ref_or(entry, |r| self.parameter(r))
    }

    fn request_body(&self, reference: &str) -> Option<&openapiv3::RequestBody> {
        let name = component_ref_name(reference, "requestBodies")?;
        let entry = self.components?.request_bodies.get(name)?;
        resolve_ref_or(entry, |r| self.request_body(r))
    }

    fn response(&self, reference: &str) -> Option<&openapiv3::Response> {
        let name = component_ref_name(reference, "responses")?;
        let entry = self.components?.responses.get(name)?;
        resolve_ref_or(entry, |r| self.response(r))
    }
}

fn component_ref_name<'a>(reference: &'a str, kind: &str) -> Option<&'a str> {
    let prefix = format!("#/components/{kind}/");
    reference
        .strip_prefix(&prefix)
        .or_else(|| {
            // Tolerate refs without the leading `#`.
            reference.strip_prefix(&format!("/components/{kind}/"))
        })
        .filter(|n| !n.is_empty() && !n.contains('/'))
}

fn resolve_ref_or<'a, T>(
    node: &'a ReferenceOr<T>,
    resolve_ref: impl Fn(&str) -> Option<&'a T>,
) -> Option<&'a T> {
    match node {
        ReferenceOr::Item(item) => Some(item),
        ReferenceOr::Reference { reference } => resolve_ref(reference),
    }
}

fn load_document(bytes: &[u8]) -> Result<Value> {
    if looks_like_json(bytes) {
        Ok(serde_json::from_slice(bytes)?)
    } else {
        let yaml: serde_yaml::Value = serde_yaml::from_slice(bytes)?;
        Ok(serde_json::to_value(yaml)?)
    }
}

fn looks_like_json(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'{' || *b == b'[')
}

/// `const: X` → `enum: [X]` + `x-const: true` so typed parse keeps discriminator tags.
pub(crate) fn rewrite_const_to_enum(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(c) = map.remove("const") {
                if !map.contains_key("enum") {
                    map.insert("enum".into(), Value::Array(vec![c]));
                }
                map.insert("x-const".into(), Value::Bool(true));
            }
            for child in map.values_mut() {
                rewrite_const_to_enum(child);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                rewrite_const_to_enum(child);
            }
        }
        _ => {}
    }
}

/// OAS 3.1 `type: ["string", "null"]` → `type: "string", nullable: true` for openapiv3.
pub(crate) fn rewrite_nullable_type_arrays(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(types)) = map.get("type").cloned() {
                let mut non_null = Vec::new();
                let mut saw_null = false;
                for t in &types {
                    match t.as_str() {
                        Some("null") => saw_null = true,
                        Some(other) => non_null.push(other.to_string()),
                        None => {}
                    }
                }
                if saw_null && non_null.len() == 1 {
                    map.insert("type".into(), Value::String(non_null.remove(0)));
                    map.insert("nullable".into(), Value::Bool(true));
                }
            }
            for child in map.values_mut() {
                rewrite_nullable_type_arrays(child);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                rewrite_nullable_type_arrays(child);
            }
        }
        _ => {}
    }
}

fn convert_operation(
    path: &str,
    method: &str,
    op: &openapiv3::Operation,
    path_parameters: &[ReferenceOr<Parameter>],
    components: &ComponentsCtx<'_>,
) -> Result<OperationNode> {
    let operation_id = op
        .operation_id
        .clone()
        .unwrap_or_else(|| synthesize_operation_id(method, path));

    let mut parameters = Vec::new();
    // Path-item parameters apply to all operations under the path (OAS), then
    // operation parameters. Operation params with the same name+in override.
    for param in path_parameters.iter().chain(op.parameters.iter()) {
        if let Some(p) = convert_parameter(param, components)? {
            if let Some(existing) = parameters
                .iter()
                .position(|e: &ParameterNode| e.name == p.name && e.location == p.location)
            {
                parameters[existing] = p;
            } else {
                parameters.push(p);
            }
        }
    }

    let request_body = match &op.request_body {
        Some(ReferenceOr::Item(body)) => first_media_schema(&body.content)
            .map(|schema| convert_ref_or_schema(schema, &format!("{operation_id}Request")))
            .transpose()?,
        Some(ReferenceOr::Reference { reference }) => components
            .request_body(reference)
            .and_then(|body| first_media_schema(&body.content))
            .map(|schema| convert_ref_or_schema(schema, &format!("{operation_id}Request")))
            .transpose()?,
        None => None,
    };

    let mut responses = IndexMap::new();
    for (code, response) in &op.responses.responses {
        let code_key = status_code_key(code);
        let schema = response_schema(response, components, &format!("{operation_id}{code_key}"))?;
        responses.insert(code_key, schema);
    }
    if let Some(default) = &op.responses.default {
        let schema = response_schema(default, components, &format!("{operation_id}Default"))?;
        responses.insert("default".into(), schema);
    }

    Ok(OperationNode {
        operation_id,
        method: method.to_ascii_lowercase(),
        path: path.to_string(),
        tags: op.tags.clone(),
        parameters,
        request_body,
        responses,
    })
}

fn response_schema(
    response: &ReferenceOr<openapiv3::Response>,
    components: &ComponentsCtx<'_>,
    hint_name: &str,
) -> Result<SchemaNode> {
    let resolved = match response {
        ReferenceOr::Item(resp) => Some(resp),
        ReferenceOr::Reference { reference } => components.response(reference),
    };
    match resolved {
        Some(resp) => {
            if let Some(schema) = first_media_schema(&resp.content) {
                convert_ref_or_schema(schema, hint_name)
            } else {
                // Empty bodies still get a status so MSW handlers accept JSON.
                Ok(SchemaNode::Unknown {
                    description: Some(resp.description.clone()).filter(|s| !s.is_empty()),
                })
            }
        }
        None => Ok(SchemaNode::Unknown { description: None }),
    }
}

fn status_code_key(code: &StatusCode) -> String {
    match code {
        StatusCode::Code(n) => n.to_string(),
        StatusCode::Range(n) => format!("{n}XX"),
    }
}

fn first_media_schema(
    content: &IndexMap<String, openapiv3::MediaType>,
) -> Option<&ReferenceOr<Schema>> {
    if let Some(json) = content.get("application/json") {
        return json.schema.as_ref();
    }
    content.values().find_map(|m| m.schema.as_ref())
}

fn convert_parameter(
    param: &ReferenceOr<Parameter>,
    components: &ComponentsCtx<'_>,
) -> Result<Option<ParameterNode>> {
    let resolved = match param {
        ReferenceOr::Item(p) => Some(p),
        ReferenceOr::Reference { reference } => components.parameter(reference),
    };
    let Some(param) = resolved else {
        return Ok(None);
    };

    let (location, data) = match param {
        Parameter::Query { parameter_data, .. } => (ParameterLocation::Query, parameter_data),
        Parameter::Header { parameter_data, .. } => (ParameterLocation::Header, parameter_data),
        Parameter::Path { parameter_data, .. } => (ParameterLocation::Path, parameter_data),
        Parameter::Cookie { parameter_data, .. } => (ParameterLocation::Cookie, parameter_data),
    };

    let required = data.required || location == ParameterLocation::Path;
    let schema = match &data.format {
        ParameterSchemaOrContent::Schema(schema) => convert_ref_or_schema(schema, &data.name)?,
        ParameterSchemaOrContent::Content(content) => first_media_schema(content)
            .map(|s| convert_ref_or_schema(s, &data.name))
            .transpose()?
            .unwrap_or_else(default_string_primitive),
    };

    Ok(Some(ParameterNode {
        name: data.name.clone(),
        location,
        required,
        schema,
    }))
}

fn default_string_primitive() -> SchemaNode {
    SchemaNode::Primitive {
        primitive: PrimitiveKind::String,
        format: None,
        min_length: None,
        max_length: None,
        nullable: false,
        description: None,
    }
}

fn synthesize_operation_id(method: &str, path: &str) -> String {
    let cleaned: String = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|seg| seg.trim_matches('{').trim_matches('}'))
        .collect::<Vec<_>>()
        .join("_");
    format!("{method}_{cleaned}")
}

fn convert_ref_or_schema(schema: &ReferenceOr<Schema>, hint_name: &str) -> Result<SchemaNode> {
    match schema {
        ReferenceOr::Reference { reference } => Ok(SchemaNode::Ref {
            name: ref_name(reference),
        }),
        ReferenceOr::Item(schema) => convert_schema(schema, hint_name),
    }
}

fn convert_ref_or_box_schema(
    schema: &ReferenceOr<Box<Schema>>,
    hint_name: &str,
) -> Result<SchemaNode> {
    match schema {
        ReferenceOr::Reference { reference } => Ok(SchemaNode::Ref {
            name: ref_name(reference),
        }),
        ReferenceOr::Item(schema) => convert_schema(schema, hint_name),
    }
}

fn convert_schema(schema: &Schema, hint_name: &str) -> Result<SchemaNode> {
    let nullable = schema.schema_data.nullable;
    let description = schema.schema_data.description.clone();

    match &schema.schema_kind {
        SchemaKind::Type(Type::String(string_type)) => {
            if !string_type.enumeration.is_empty() {
                let values = string_type
                    .enumeration
                    .iter()
                    .map(|maybe_string| match maybe_string {
                        Some(value) => EnumValue::String(value.clone()),
                        None => EnumValue::Null,
                    })
                    .collect();
                return Ok(normalize_enum_node(values, nullable, description));
            }
            Ok(SchemaNode::Primitive {
                primitive: PrimitiveKind::String,
                format: format_string(&string_type.format),
                min_length: string_type.min_length.map(|len| len as u32),
                max_length: string_type.max_length.map(|len| len as u32),
                nullable,
                description,
            })
        }
        SchemaKind::Type(Type::Number(number_type)) => {
            if !number_type.enumeration.is_empty() {
                let values = number_type
                    .enumeration
                    .iter()
                    .map(|maybe_number| match maybe_number {
                        Some(number) => EnumValue::Number(*number),
                        None => EnumValue::Null,
                    })
                    .collect();
                return Ok(normalize_enum_node(values, nullable, description));
            }
            Ok(SchemaNode::Primitive {
                primitive: PrimitiveKind::Number,
                format: format_number(&number_type.format),
                min_length: None,
                max_length: None,
                nullable,
                description,
            })
        }
        SchemaKind::Type(Type::Integer(integer_type)) => {
            if !integer_type.enumeration.is_empty() {
                let values = integer_type
                    .enumeration
                    .iter()
                    .map(|maybe_integer| match maybe_integer {
                        Some(integer) => EnumValue::Number(*integer as f64),
                        None => EnumValue::Null,
                    })
                    .collect();
                return Ok(normalize_enum_node(values, nullable, description));
            }
            Ok(SchemaNode::Primitive {
                primitive: PrimitiveKind::Integer,
                format: format_integer(&integer_type.format),
                min_length: None,
                max_length: None,
                nullable,
                description,
            })
        }
        SchemaKind::Type(Type::Boolean(boolean_type)) => {
            if !boolean_type.enumeration.is_empty() {
                let values = boolean_type
                    .enumeration
                    .iter()
                    .map(|maybe_bool| match maybe_bool {
                        Some(flag) => EnumValue::Bool(*flag),
                        None => EnumValue::Null,
                    })
                    .collect();
                return Ok(normalize_enum_node(values, nullable, description));
            }
            Ok(SchemaNode::Primitive {
                primitive: PrimitiveKind::Boolean,
                format: None,
                min_length: None,
                max_length: None,
                nullable,
                description,
            })
        }
        SchemaKind::Type(Type::Object(object_type)) => {
            convert_object(object_type, hint_name, nullable, description)
        }
        SchemaKind::Type(Type::Array(array_type)) => {
            let items = array_type
                .items
                .as_ref()
                .map(|item_schema| {
                    convert_ref_or_box_schema(item_schema, &format!("{hint_name}Item"))
                })
                .transpose()?
                .unwrap_or(SchemaNode::Unknown { description: None });
            Ok(SchemaNode::Array {
                items: Box::new(items),
                nullable,
                description,
            })
        }
        SchemaKind::AllOf { all_of } => {
            let items = all_of
                .iter()
                .enumerate()
                .map(|(i, v)| convert_ref_or_schema(v, &format!("{hint_name}AllOf{i}")))
                .collect::<Result<Vec<_>>>()?;
            // Kubb: single-member allOf flattens to the member (merge nullable).
            if items.len() == 1 {
                return Ok(flatten_single_allof(items, nullable));
            }
            Ok(SchemaNode::AllOf { items, nullable })
        }
        SchemaKind::OneOf { one_of } => {
            let items = one_of
                .iter()
                .enumerate()
                .map(|(i, v)| convert_ref_or_schema(v, &format!("{hint_name}OneOf{i}")))
                .collect::<Result<Vec<_>>>()?;
            Ok(SchemaNode::OneOf {
                items,
                nullable,
                discriminator: schema
                    .schema_data
                    .discriminator
                    .as_ref()
                    .map(convert_discriminator),
            })
        }
        SchemaKind::AnyOf { any_of } => {
            let items = any_of
                .iter()
                .enumerate()
                .map(|(i, v)| convert_ref_or_schema(v, &format!("{hint_name}AnyOf{i}")))
                .collect::<Result<Vec<_>>>()?;
            Ok(SchemaNode::AnyOf { items, nullable })
        }
        SchemaKind::Not { .. } => Ok(SchemaNode::Unknown { description }),
        SchemaKind::Any(any_schema) => convert_any_schema(
            any_schema,
            &schema.schema_data,
            hint_name,
            nullable,
            description,
        ),
    }
}

fn convert_any_schema(
    any_schema: &openapiv3::AnySchema,
    schema_data: &openapiv3::SchemaData,
    hint_name: &str,
    nullable: bool,
    description: Option<String>,
) -> Result<SchemaNode> {
    // OAS 3.1 `type: null` (also used in 3.0 specs via anyOf) — openapiv3 has no
    // Type::Null, so it lands in AnySchema with typ == "null".
    if any_schema.typ.as_deref() == Some("null") {
        return Ok(SchemaNode::Primitive {
            primitive: PrimitiveKind::Null,
            format: None,
            min_length: None,
            max_length: None,
            nullable: false,
            description,
        });
    }
    if !any_schema.enumeration.is_empty() {
        let values = any_schema
            .enumeration
            .iter()
            .filter_map(enum_value)
            .collect();
        return Ok(normalize_enum_node(values, nullable, description));
    }
    if !any_schema.all_of.is_empty() {
        let items = any_schema
            .all_of
            .iter()
            .enumerate()
            .map(|(index, schema)| {
                convert_ref_or_schema(schema, &format!("{hint_name}AllOf{index}"))
            })
            .collect::<Result<Vec<_>>>()?;
        if items.len() == 1 {
            return Ok(flatten_single_allof(items, nullable));
        }
        return Ok(SchemaNode::AllOf { items, nullable });
    }
    if !any_schema.one_of.is_empty() {
        let items = any_schema
            .one_of
            .iter()
            .enumerate()
            .map(|(index, schema)| {
                convert_ref_or_schema(schema, &format!("{hint_name}OneOf{index}"))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(SchemaNode::OneOf {
            items,
            nullable,
            discriminator: schema_data
                .discriminator
                .as_ref()
                .map(convert_discriminator),
        });
    }
    if !any_schema.any_of.is_empty() {
        let items = any_schema
            .any_of
            .iter()
            .enumerate()
            .map(|(index, schema)| {
                convert_ref_or_schema(schema, &format!("{hint_name}AnyOf{index}"))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(SchemaNode::AnyOf { items, nullable });
    }

    // Bare `{}` / freeform catch-all → open object (same as Kubb-ish `any` path).
    let object_type = openapiv3::ObjectType {
        properties: any_schema.properties.clone(),
        required: any_schema.required.clone(),
        additional_properties: any_schema.additional_properties.clone(),
        min_properties: any_schema.min_properties,
        max_properties: any_schema.max_properties,
    };
    convert_object(&object_type, hint_name, nullable, description)
}

fn convert_object(
    object_type: &openapiv3::ObjectType,
    hint_name: &str,
    nullable: bool,
    description: Option<String>,
) -> Result<SchemaNode> {
    let mut properties = IndexMap::new();
    let mut readonly_properties = Vec::new();
    let mut writeonly_properties = Vec::new();
    let mut const_properties = Vec::new();

    for (property_name, property_schema) in &object_type.properties {
        match property_schema {
            ReferenceOr::Item(schema) => {
                if schema.schema_data.read_only {
                    readonly_properties.push(property_name.clone());
                }
                if schema.schema_data.write_only {
                    writeonly_properties.push(property_name.clone());
                }
                if schema
                    .schema_data
                    .extensions
                    .get("x-const")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    const_properties.push(property_name.clone());
                }
                properties.insert(
                    property_name.clone(),
                    convert_schema(schema, &format!("{hint_name}_{property_name}"))?,
                );
            }
            ReferenceOr::Reference { reference } => {
                properties.insert(
                    property_name.clone(),
                    SchemaNode::Ref {
                        name: ref_name(reference),
                    },
                );
            }
        }
    }

    let mut required = object_type.required.clone();
    // `const` discriminator tags behave as required literals (Kubb / TS narrowing).
    for property_name in const_properties {
        if !required.iter().any(|name| name == &property_name) {
            required.push(property_name);
        }
    }

    let additional_properties = match &object_type.additional_properties {
        Some(AdditionalProperties::Any(false)) | None => None,
        Some(AdditionalProperties::Any(true)) => {
            Some(Box::new(SchemaNode::Unknown { description: None }))
        }
        Some(AdditionalProperties::Schema(schema)) => Some(Box::new(convert_ref_or_schema(
            schema,
            &format!("{hint_name}Additional"),
        )?)),
    };

    // OAS often uses a bare `{}` for free-form blobs (e.g. `table_config: {}`).
    let additional_properties = if properties.is_empty() && additional_properties.is_none() {
        Some(Box::new(SchemaNode::Unknown { description: None }))
    } else {
        additional_properties
    };

    Ok(SchemaNode::Object {
        properties,
        required,
        readonly_properties,
        writeonly_properties,
        additional_properties,
        nullable,
        description,
    })
}

fn convert_discriminator(discriminator: &openapiv3::Discriminator) -> Discriminator {
    let mut mapping = IndexMap::new();
    for (discriminator_value, schema_ref) in &discriminator.mapping {
        mapping.insert(discriminator_value.clone(), ref_name(schema_ref));
    }
    Discriminator {
        property_name: discriminator.property_name.clone(),
        mapping,
    }
}

fn format_string(f: &VariantOrUnknownOrEmpty<StringFormat>) -> Option<String> {
    match f {
        VariantOrUnknownOrEmpty::Item(StringFormat::Date) => Some("date".into()),
        VariantOrUnknownOrEmpty::Item(StringFormat::DateTime) => Some("date-time".into()),
        VariantOrUnknownOrEmpty::Item(StringFormat::Password) => Some("password".into()),
        VariantOrUnknownOrEmpty::Item(StringFormat::Byte) => Some("byte".into()),
        VariantOrUnknownOrEmpty::Item(StringFormat::Binary) => Some("binary".into()),
        VariantOrUnknownOrEmpty::Unknown(s) => Some(s.clone()),
        VariantOrUnknownOrEmpty::Empty => None,
    }
}

fn format_number(f: &VariantOrUnknownOrEmpty<NumberFormat>) -> Option<String> {
    match f {
        VariantOrUnknownOrEmpty::Item(NumberFormat::Float) => Some("float".into()),
        VariantOrUnknownOrEmpty::Item(NumberFormat::Double) => Some("double".into()),
        VariantOrUnknownOrEmpty::Unknown(s) => Some(s.clone()),
        VariantOrUnknownOrEmpty::Empty => None,
    }
}

fn format_integer(f: &VariantOrUnknownOrEmpty<IntegerFormat>) -> Option<String> {
    match f {
        VariantOrUnknownOrEmpty::Item(IntegerFormat::Int32) => Some("int32".into()),
        VariantOrUnknownOrEmpty::Item(IntegerFormat::Int64) => Some("int64".into()),
        VariantOrUnknownOrEmpty::Unknown(s) => Some(s.clone()),
        VariantOrUnknownOrEmpty::Empty => None,
    }
}

fn enum_value(value: &Value) -> Option<EnumValue> {
    match value {
        Value::String(s) => Some(EnumValue::String(s.clone())),
        Value::Number(n) => n.as_f64().map(EnumValue::Number),
        Value::Bool(b) => Some(EnumValue::Bool(*b)),
        Value::Null => Some(EnumValue::Null),
        _ => None,
    }
}

/// Kubb NullEnum / enum-with-null:
/// - `{ enum: [null] }` → null primitive (not empty/`never`)
/// - `{ enum: […, null] }` → strip null values + set nullable
fn normalize_enum_node(
    values: Vec<EnumValue>,
    nullable: bool,
    description: Option<String>,
) -> SchemaNode {
    let mut saw_null = nullable;
    let non_null: Vec<EnumValue> = values
        .into_iter()
        .filter(|v| {
            if matches!(v, EnumValue::Null) {
                saw_null = true;
                false
            } else {
                true
            }
        })
        .collect();

    if non_null.is_empty() {
        return SchemaNode::Primitive {
            primitive: PrimitiveKind::Null,
            format: None,
            min_length: None,
            max_length: None,
            nullable: false,
            description,
        };
    }

    SchemaNode::Enum {
        values: non_null,
        nullable: saw_null,
        description,
    }
}

/// Kubb: single-member `allOf` flattens to the member (OR-ing nullable).
fn flatten_single_allof(mut items: Vec<SchemaNode>, nullable: bool) -> SchemaNode {
    debug_assert_eq!(items.len(), 1);
    let mut item = items.pop().expect("single-member allOf");
    if nullable {
        set_root_nullable(&mut item, true);
    }
    item
}

fn set_root_nullable(schema: &mut SchemaNode, nullable: bool) {
    match schema {
        SchemaNode::Ref { .. } | SchemaNode::Unknown { .. } => {
            // Refs can't carry nullable on the node; wrap so `| null` still emits.
            if nullable {
                *schema = SchemaNode::AllOf {
                    items: vec![std::mem::replace(
                        schema,
                        SchemaNode::Unknown { description: None },
                    )],
                    nullable: true,
                };
            }
        }
        SchemaNode::Object { nullable: flag, .. }
        | SchemaNode::Array { nullable: flag, .. }
        | SchemaNode::Enum { nullable: flag, .. }
        | SchemaNode::Primitive { nullable: flag, .. }
        | SchemaNode::AllOf { nullable: flag, .. }
        | SchemaNode::OneOf { nullable: flag, .. }
        | SchemaNode::AnyOf { nullable: flag, .. } => *flag = true,
    }
}

fn ref_name(r: &str) -> String {
    r.rsplit('/').next().unwrap_or(r).trim().to_string()
}
