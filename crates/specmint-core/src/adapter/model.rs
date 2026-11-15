//! In-memory model of an OpenAPI document after parsing.
//!
//! Plugins read this instead of the raw OpenAPI JSON/YAML so naming, cycles,
//! and nullability rules stay consistent across plugins.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSpec {
    pub meta: SpecMeta,
    pub schemas: IndexMap<String, SchemaNode>,
    pub operations: Vec<OperationNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecMeta {
    pub title: Option<String>,
    pub version: Option<String>,
    pub openapi: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationNode {
    pub operation_id: String,
    pub method: String,
    pub path: String,
    pub tags: Vec<String>,
    pub parameters: Vec<ParameterNode>,
    pub request_body: Option<SchemaNode>,
    pub responses: IndexMap<String, SchemaNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterNode {
    pub name: String,
    pub location: ParameterLocation,
    pub required: bool,
    pub schema: SchemaNode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Query,
    Path,
    Header,
    Cookie,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SchemaNode {
    Ref {
        name: String,
    },
    Object {
        properties: IndexMap<String, SchemaNode>,
        required: Vec<String>,
        /// Property names marked `readOnly: true` in OAS (stripped from request bodies).
        #[serde(default)]
        readonly_properties: Vec<String>,
        /// Property names marked `writeOnly: true` in OAS (stripped from response bodies).
        #[serde(default)]
        writeonly_properties: Vec<String>,
        additional_properties: Option<Box<SchemaNode>>,
        nullable: bool,
        description: Option<String>,
    },
    Array {
        items: Box<SchemaNode>,
        nullable: bool,
        description: Option<String>,
    },
    Enum {
        values: Vec<EnumValue>,
        nullable: bool,
        description: Option<String>,
    },
    Primitive {
        primitive: PrimitiveKind,
        format: Option<String>,
        min_length: Option<u32>,
        max_length: Option<u32>,
        nullable: bool,
        description: Option<String>,
    },
    AllOf {
        items: Vec<SchemaNode>,
        nullable: bool,
    },
    OneOf {
        items: Vec<SchemaNode>,
        nullable: bool,
        discriminator: Option<Discriminator>,
    },
    AnyOf {
        items: Vec<SchemaNode>,
        nullable: bool,
    },
    Unknown {
        description: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discriminator {
    pub property_name: String,
    pub mapping: IndexMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum EnumValue {
    String(String),
    Number(f64),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PrimitiveKind {
    String,
    Number,
    Integer,
    Boolean,
    Null,
}
