//! Naming resolvers — Kubb `Resolver` equivalent shared by all plugins.
//!
//! **Inversion of control:** plugins never choose casing themselves. They call
//! methods on [`Naming`], which applies [`crate::config::NamingConfig`] kind
//! rules plus regex overrides.

mod case;

use regex::Regex;

use serde::Deserialize;

use crate::config::{
    AffixNamingPatch, AffixNamingRule, CaseStyle, EnumMemberNamingRule, IdentNamingRule,
    NamingConfig, NamingOverride, OverrideKind, OverrideMatch,
};
use crate::error::{Error, Result};

pub use case::{
    apply_ident_rule, enum_member_name as enum_member_name_with_rule, is_valid_ts_identifier,
    schema_type_name, to_case,
};

/// An override entry: something that applies when its `match` does.
pub trait Override {
    fn target(&self) -> &OverrideMatch;
}

impl Override for NamingOverride {
    fn target(&self) -> &OverrideMatch {
        &self.target
    }
}

/// Override entries with their patterns compiled, in config order. Core uses
/// it for `[[naming.overrides]]`; plugins use it for their own overrides.
#[derive(Debug, Clone)]
pub struct OverrideSet<O> {
    entries: Vec<(Regex, O)>,
}

impl<O: Override> OverrideSet<O> {
    /// `context` names where the overrides come from, for errors
    /// (`"naming.overrides"`, `"plugins.<name>.overrides"`).
    pub fn new(overrides: Vec<O>, context: &str) -> Result<Self> {
        let entries = overrides
            .into_iter()
            .map(|ovr| {
                let pattern = &ovr.target().pattern;
                let re = Regex::new(pattern).map_err(|e| {
                    Error::config(format!("{context}: invalid regex {pattern:?}: {e}"))
                })?;
                Ok((re, ovr))
            })
            .collect::<Result<_>>()?;
        Ok(Self { entries })
    }

    /// Like [`OverrideSet::new`], but rejects overrides of any other kind, which would never apply.
    pub fn for_kind(overrides: Vec<O>, kind: OverrideKind, context: &str) -> Result<Self> {
        if let Some(other) = overrides.iter().find(|ovr| ovr.target().kind != kind) {
            return Err(Error::config(format!(
                "{context}: `kind = \"{}\"` never applies here; these overrides match `kind = \"{}\"`",
                other.target().kind.as_str(),
                kind.as_str()
            )));
        }
        Self::new(overrides, context)
    }

    /// The overrides of `kind` whose pattern matches `name`, in config order.
    pub fn matching<'a>(
        &'a self,
        kind: OverrideKind,
        name: &'a str,
    ) -> impl Iterator<Item = &'a O> + 'a {
        self.entries
            .iter()
            .filter(move |(re, ovr)| ovr.target().kind == kind && re.is_match(name))
            .map(|(_, ovr)| ovr)
    }
}

impl<O> Default for OverrideSet<O> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

/// `{ match, naming }`: an affix patch for the names whose `match` applies.
/// Plugins use it for the overrides of the names they own.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AffixOverride {
    #[serde(rename = "match")]
    pub target: OverrideMatch,
    pub naming: AffixNamingPatch,
}

impl Override for AffixOverride {
    fn target(&self) -> &OverrideMatch {
        &self.target
    }
}

/// An affix rule plus the overrides that patch it for some names: the names
/// one plugin owns, matched by one kind (schema name or operation id).
#[derive(Debug)]
pub struct AffixNames {
    rule: AffixNamingRule,
    kind: OverrideKind,
    overrides: OverrideSet<AffixOverride>,
}

impl AffixNames {
    pub fn new(
        rule: AffixNamingRule,
        kind: OverrideKind,
        overrides: Vec<AffixOverride>,
        context: &str,
    ) -> Result<Self> {
        Ok(Self {
            rule,
            kind,
            overrides: OverrideSet::for_kind(overrides, kind, context)?,
        })
    }

    pub fn rule_for(&self, name: &str) -> AffixNamingRule {
        self.overrides
            .matching(self.kind, name)
            .fold(self.rule.clone(), |rule, ovr| rule.apply_patch(&ovr.naming))
    }
}

/// `prefix + base cased by rule.case + suffix`.
pub fn apply_affix_rule(base: &str, rule: &AffixNamingRule) -> String {
    format!("{}{}{}", rule.prefix, to_case(base, rule.case), rule.suffix)
}

#[derive(Debug, Clone)]
pub struct Naming {
    config: NamingConfig,
    overrides: OverrideSet<NamingOverride>,
}

impl Naming {
    pub fn new(config: NamingConfig) -> Result<Self> {
        let config = config.normalize();
        let overrides = OverrideSet::new(config.overrides.clone(), "naming.overrides")?;
        Ok(Self { config, overrides })
    }

    fn schema_rule(&self, schema_name: &str) -> IdentNamingRule {
        let mut rule = self.config.schema.clone();
        for ovr in self
            .overrides
            .matching(OverrideKind::SchemaName, schema_name)
        {
            if let Some(patch) = &ovr.schema {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn enum_member_rule(&self, schema_or_type_hint: &str) -> EnumMemberNamingRule {
        let mut rule = self.config.enum_member.clone();
        for ovr in self
            .overrides
            .matching(OverrideKind::SchemaName, schema_or_type_hint)
        {
            if let Some(patch) = &ovr.enum_member {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    fn operation_rule(&self, operation_id: &str) -> IdentNamingRule {
        let mut rule = self.config.operation.clone();
        for ovr in self
            .overrides
            .matching(OverrideKind::OperationId, operation_id)
        {
            if let Some(patch) = &ovr.operation {
                rule = rule.apply_patch(patch);
            }
        }
        rule
    }

    pub fn schema_type_name(&self, name: &str) -> String {
        if let Some(rename) = self
            .overrides
            .matching(OverrideKind::SchemaName, name)
            .find_map(|ovr| ovr.rename.as_ref())
        {
            return rename.clone();
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

    pub fn operation_type_base(&self, operation_id: &str) -> String {
        if let Some(rename) = self
            .overrides
            .matching(OverrideKind::OperationId, operation_id)
            .find_map(|ovr| ovr.rename.as_ref())
        {
            return rename.clone();
        }
        apply_ident_rule(operation_id, &self.operation_rule(operation_id))
    }
}

impl CaseStyle {
    pub fn convert(self, input: &str) -> String {
        to_case(input, self)
    }
}
