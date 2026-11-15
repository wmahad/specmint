//! Naming resolvers — Kubb `Resolver` equivalent shared by all emitters.
//!
//! **Inversion of control:** emitters never choose casing themselves. They call
//! methods on [`Naming`], which applies [`crate::config::NamingConfig`] kind
//! rules plus regex overrides.

mod case;

use regex::Regex;

use crate::config::{
    AffixNamingRule, CaseStyle, EnumMemberNamingRule, EnumStyle, IdentNamingRule, NamingConfig,
    NamingOverride, OverrideKind, TypesEmitterConfig,
};
use crate::error::Result;

pub use case::{
    apply_ident_rule, enum_member_name as enum_member_name_with_rule, is_valid_ts_identifier,
    schema_type_name, to_case,
};

#[derive(Debug, Clone)]
pub struct Naming {
    config: NamingConfig,
    override_regexes: Vec<(OverrideKind, Regex, NamingOverride)>,
}

impl Naming {
    pub fn new(config: NamingConfig) -> Result<Self> {
        let config = config.normalize();
        let mut override_regexes = Vec::new();
        for ovr in &config.overrides {
            let re = Regex::new(&ovr.target.pattern)
                .map_err(|e| crate::error::Error::config(format!("bad override regex: {e}")))?;
            override_regexes.push((ovr.target.kind, re, ovr.clone()));
        }
        Ok(Self {
            config,
            override_regexes,
        })
    }

    fn matching_overrides(&self, kind: OverrideKind, name: &str) -> Vec<&NamingOverride> {
        self.override_regexes
            .iter()
            .filter_map(|(k, re, ovr)| (*k == kind && re.is_match(name)).then_some(ovr))
            .collect()
    }

    fn schema_rule(&self, schema_name: &str) -> IdentNamingRule {
        let mut rule = self.config.schema.clone();
        for ovr in self.matching_overrides(OverrideKind::SchemaName, schema_name) {
            if let Some(patch) = &ovr.schema {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn enum_member_rule(&self, schema_or_type_hint: &str) -> EnumMemberNamingRule {
        let mut rule = self.config.enum_member.clone();
        for ovr in self.matching_overrides(OverrideKind::SchemaName, schema_or_type_hint) {
            if let Some(patch) = &ovr.enum_member {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn operation_rule(&self, operation_id: &str) -> IdentNamingRule {
        let mut rule = self.config.operation.clone();
        for ovr in self.matching_overrides(OverrideKind::OperationId, operation_id) {
            if let Some(patch) = &ovr.operation {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn factory_rule(&self, type_name: &str) -> AffixNamingRule {
        let mut rule = self.config.factory.clone();
        for ovr in self.matching_overrides(OverrideKind::SchemaName, type_name) {
            if let Some(patch) = &ovr.factory {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn handler_rule(&self, operation_id: &str) -> AffixNamingRule {
        let mut rule = self.config.handler.clone();
        for ovr in self.matching_overrides(OverrideKind::OperationId, operation_id) {
            if let Some(patch) = &ovr.handler {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    pub fn schema_type_name(&self, name: &str) -> String {
        for ovr in self.matching_overrides(OverrideKind::SchemaName, name) {
            if let Some(rename) = &ovr.rename {
                return rename.clone();
            }
        }
        apply_ident_rule(name, &self.schema_rule(name))
    }

    pub fn enum_member_name(&self, type_name: &str, raw: &str) -> String {
        case::enum_member_name(type_name, raw, &self.enum_member_rule(type_name))
    }

    /// Enum member naming keyed off the original OpenAPI schema name (for overrides).
    pub fn enum_member_name_for_schema(
        &self,
        schema_name: &str,
        type_name: &str,
        raw: &str,
    ) -> String {
        case::enum_member_name(type_name, raw, &self.enum_member_rule(schema_name))
    }

    pub fn factory_name(&self, type_name: &str) -> String {
        let rule = self.factory_rule(type_name);
        format!("{}{type_name}{}", rule.prefix, rule.suffix)
    }

    pub fn handler_name(&self, operation_id: &str) -> String {
        let rule = self.handler_rule(operation_id);
        let base = to_case(operation_id, rule.case);
        format!("{}{base}{}", rule.prefix, rule.suffix)
    }

    pub fn zod_name(&self, type_name: &str) -> String {
        let rule = &self.config.zod;
        let base = to_case(type_name, rule.case);
        format!("{}{base}{}", rule.prefix, rule.suffix)
    }

    pub fn operation_type_base(&self, operation_id: &str) -> String {
        for ovr in self.matching_overrides(OverrideKind::OperationId, operation_id) {
            if let Some(rename) = &ovr.rename {
                return rename.clone();
            }
        }
        apply_ident_rule(operation_id, &self.operation_rule(operation_id))
    }

    pub fn enum_style_for_schema(
        &self,
        schema_name: &str,
        types_cfg: &TypesEmitterConfig,
    ) -> EnumStyle {
        for ovr in self.matching_overrides(OverrideKind::SchemaName, schema_name) {
            if let Some(style) = ovr.enum_style {
                return style;
            }
        }
        types_cfg.enum_style
    }

    pub fn object_syntax_for_schema(
        &self,
        schema_name: &str,
        types_cfg: &TypesEmitterConfig,
    ) -> crate::config::ObjectSyntax {
        for ovr in self.matching_overrides(OverrideKind::SchemaName, schema_name) {
            if let Some(syntax) = ovr.syntax {
                return syntax;
            }
        }
        types_cfg.syntax
    }
}

impl CaseStyle {
    pub fn convert(self, input: &str) -> String {
        to_case(input, self)
    }
}
