//! OpenAPI document → parsed OpenAPI model shared by all emitters.

mod model;
mod parse;
mod validate;

pub use model::*;
pub use parse::{ParseOptions, parse_openapi, parse_openapi_with_options};
pub use validate::validate_openapi;

use sha2::{Digest, Sha256};

impl ParsedSpec {
    pub fn content_hash(&self) -> String {
        // Stable enough for skip-emit: schema names + op ids + serialized shapes.
        let mut hasher = Sha256::new();
        for (name, schema) in &self.schemas {
            hasher.update(name.as_bytes());
            hasher.update(serde_json::to_string(schema).unwrap_or_default().as_bytes());
        }
        for op in &self.operations {
            hasher.update(op.operation_id.as_bytes());
            hasher.update(op.method.as_bytes());
            hasher.update(op.path.as_bytes());
        }
        crate::source::hex_encode(hasher.finalize())
    }
}
