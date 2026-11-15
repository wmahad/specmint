//! In-memory model of an OpenAPI document after parsing.
//!
//! Plugins read this instead of the raw OpenAPI JSON/YAML so naming, cycles,
//! and nullability rules stay consistent across plugins.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type SchemaMap = IndexMap<String, SchemaNode>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSpec {
    pub meta: SpecMeta,
    pub schemas: SchemaMap,
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

impl SchemaNode {
    /// The shape as a response carries it: `writeOnly` properties left out of
    /// inline objects. `$ref`s are kept as they are, so a named component
    /// still has its write fields wherever it is used as input.
    pub fn without_write_only(&self) -> SchemaNode {
        match self {
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
                    .map(|(k, v)| (k.clone(), v.without_write_only()))
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
                        .map(|a| Box::new(a.without_write_only())),
                    nullable: *nullable,
                    description: description.clone(),
                }
            }
            SchemaNode::Array {
                items,
                nullable,
                description,
            } => SchemaNode::Array {
                items: Box::new(items.without_write_only()),
                nullable: *nullable,
                description: description.clone(),
            },
            SchemaNode::AllOf { items, nullable } => SchemaNode::AllOf {
                items: items.iter().map(SchemaNode::without_write_only).collect(),
                nullable: *nullable,
            },
            SchemaNode::OneOf {
                items,
                nullable,
                discriminator,
            } => SchemaNode::OneOf {
                items: items.iter().map(SchemaNode::without_write_only).collect(),
                nullable: *nullable,
                discriminator: discriminator.clone(),
            },
            SchemaNode::AnyOf { items, nullable } => SchemaNode::AnyOf {
                items: items.iter().map(SchemaNode::without_write_only).collect(),
                nullable: *nullable,
            },
            other => other.clone(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn object(props: &[(&str, SchemaNode)], required: &[&str], write_only: &[&str]) -> SchemaNode {
        SchemaNode::Object {
            properties: props
                .iter()
                .map(|(name, schema)| (name.to_string(), schema.clone()))
                .collect(),
            required: required.iter().map(|r| r.to_string()).collect(),
            readonly_properties: vec![],
            writeonly_properties: write_only.iter().map(|w| w.to_string()).collect(),
            additional_properties: None,
            nullable: false,
            description: None,
        }
    }

    fn string() -> SchemaNode {
        SchemaNode::Primitive {
            primitive: PrimitiveKind::String,
            format: None,
            nullable: false,
            min_length: None,
            max_length: None,
            description: None,
        }
    }

    #[test]
    fn without_write_only_drops_nested_write_only_fields_and_keeps_refs() {
        let secret = SchemaNode::Ref {
            name: "Secret".into(),
        };
        let schema = SchemaNode::Array {
            items: Box::new(object(
                &[
                    ("id", string()),
                    ("password", string()),
                    ("secret", secret.clone()),
                ],
                &["id", "password"],
                &["password"],
            )),
            nullable: false,
            description: None,
        };

        let expected = SchemaNode::Array {
            items: Box::new(object(
                &[("id", string()), ("secret", secret)],
                &["id"],
                &[],
            )),
            nullable: false,
            description: None,
        };
        assert_eq!(
            serde_json::to_value(schema.without_write_only()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}
