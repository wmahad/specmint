//! Single config for source, output, naming, and the per-plugin sections.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

const DEFAULT_BANNER: &str = r#"/* oxlint-disable typescript/no-unused-vars */
// @ts-nocheck
/*
 * ---------------------------------------------------------------
 * ## THIS FILE WAS GENERATED FROM API
 * ## Do not edit manually.
 * ---------------------------------------------------------------
 */"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateMode {
    Full,
    Types,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub source: SourceConfig,
    pub output: OutputConfig,
    #[serde(default)]
    pub adapter: AdapterConfig,
    #[serde(default)]
    pub naming: NamingConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Local OpenAPI file **or** `http(s)://` URL — one field for both.
    pub input: String,
    pub cache: CacheConfig,
}

impl SourceConfig {
    pub fn is_url(&self) -> bool {
        is_http_url(&self.input)
    }
}

fn is_http_url(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

/// Accepts `input`, or legacy `path` / `url` (exactly one required).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceConfigSerde {
    input: Option<String>,
    path: Option<PathBuf>,
    url: Option<String>,
    #[serde(default)]
    cache: CacheConfig,
}

impl<'de> Deserialize<'de> for SourceConfig {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let raw = SourceConfigSerde::deserialize(deserializer)?;
        let set = [raw.input.is_some(), raw.path.is_some(), raw.url.is_some()]
            .into_iter()
            .filter(|x| *x)
            .count();
        if set == 0 {
            return Err(D::Error::custom(
                "source.input is required (file path or http(s) URL); legacy source.path / source.url also work",
            ));
        }
        if set > 1 {
            return Err(D::Error::custom(
                "set only one of source.input, source.path, or source.url",
            ));
        }
        let input = raw
            .input
            .or(raw.url)
            .or_else(|| raw.path.map(|p| p.to_string_lossy().into_owned()))
            .expect("checked above");
        Ok(Self {
            input,
            cache: raw.cache,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    #[serde(default = "default_cache_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_true")]
    pub use_etag: bool,
    #[serde(default = "default_true")]
    pub use_content_hash: bool,
}

fn default_cache_dir() -> PathBuf {
    PathBuf::from(".cache/openapi")
}

fn default_true() -> bool {
    true
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            dir: default_cache_dir(),
            use_etag: true,
            use_content_hash: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub clean: bool,
    #[serde(default = "default_banner")]
    pub banner: String,
}

fn default_banner() -> String {
    DEFAULT_BANNER.to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    #[serde(default = "default_true")]
    pub validate: bool,
    #[serde(default = "default_discriminator")]
    pub discriminator: String,
    #[serde(default = "default_enum_suffix")]
    pub enum_suffix: String,
}

fn default_discriminator() -> String {
    "propagate".into()
}

fn default_enum_suffix() -> String {
    // Kubb config uses a single space to avoid `FooEnumEnum` doubling.
    " ".into()
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            // On by default: catches duplicate ids / broken $refs / path-param gaps
            // before plugins produce confusing TS.
            validate: true,
            discriminator: default_discriminator(),
            enum_suffix: default_enum_suffix(),
        }
    }
}

/// Config-owned naming policy (inversion of control).
///
/// Plugins never hardcode casing — they ask [`crate::naming::Naming`], which
/// applies these kind rules + regex overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingConfig {
    /// Schema / interface / type alias identifiers.
    #[serde(default = "IdentNamingRule::schema_default")]
    pub schema: IdentNamingRule,
    /// Members inside `export enum`.
    #[serde(default)]
    pub enum_member: EnumMemberNamingRule,
    /// OperationId → auxiliary type base (`ListPets` → `ListPetsQueryParams`).
    #[serde(default = "IdentNamingRule::operation_default")]
    pub operation: IdentNamingRule,
    #[serde(default = "AffixNamingRule::factory_default")]
    pub factory: AffixNamingRule,
    #[serde(default = "AffixNamingRule::handler_default")]
    pub handler: AffixNamingRule,
    #[serde(default = "AffixNamingRule::zod_default")]
    pub zod: AffixNamingRule,
    #[serde(default)]
    pub overrides: Vec<NamingOverride>,

    // --- Legacy flat aliases (still accepted; overlay onto nested rules) ---
    #[serde(default)]
    schema_case: Option<CaseStyle>,
    #[serde(default)]
    operation_case: Option<CaseStyle>,
    #[serde(default)]
    factory_prefix: Option<String>,
    #[serde(default)]
    handler_case: Option<CaseStyle>,
    #[serde(default)]
    handler_suffix: Option<String>,
}

fn default_pascal() -> CaseStyle {
    CaseStyle::PascalCase
}

fn default_factory_prefix() -> String {
    "create".into()
}

fn default_handler_suffix() -> String {
    "Handler".into()
}

fn default_zod_suffix() -> String {
    "Schema".into()
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            schema: IdentNamingRule::schema_default(),
            enum_member: EnumMemberNamingRule::default(),
            operation: IdentNamingRule::operation_default(),
            factory: AffixNamingRule::factory_default(),
            handler: AffixNamingRule::handler_default(),
            zod: AffixNamingRule::zod_default(),
            overrides: Vec::new(),
            schema_case: None,
            operation_case: None,
            factory_prefix: None,
            handler_case: None,
            handler_suffix: None,
        }
    }
}

impl NamingConfig {
    /// Fold legacy flat keys into nested kind rules.
    pub fn normalize(mut self) -> Self {
        if let Some(c) = self.schema_case.take() {
            self.schema.case = c;
        }
        if let Some(c) = self.operation_case.take() {
            self.operation.case = c;
        }
        if let Some(p) = self.factory_prefix.take() {
            self.factory.prefix = p;
        }
        if let Some(c) = self.handler_case.take() {
            self.handler.case = c;
        }
        if let Some(s) = self.handler_suffix.take() {
            self.handler.suffix = s;
        }
        self
    }
}

/// Prefix / case / suffix for a single identifier kind.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentNamingRule {
    #[serde(default = "default_pascal")]
    pub case: CaseStyle,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    /// Keep the OpenAPI name when it is already a valid TS identifier
    /// (preserves acronyms like `HTTPHeader`).
    #[serde(default = "default_true")]
    pub preserve_valid_identifier: bool,
}

impl Default for IdentNamingRule {
    fn default() -> Self {
        Self::schema_default()
    }
}

impl IdentNamingRule {
    pub fn schema_default() -> Self {
        Self {
            case: CaseStyle::PascalCase,
            prefix: String::new(),
            suffix: String::new(),
            preserve_valid_identifier: true,
        }
    }

    pub fn operation_default() -> Self {
        Self {
            case: CaseStyle::PascalCase,
            prefix: String::new(),
            suffix: String::new(),
            preserve_valid_identifier: false,
        }
    }

    pub fn apply_patch(&self, patch: &IdentNamingPatch) -> Self {
        Self {
            case: patch.case.unwrap_or(self.case),
            prefix: patch.prefix.clone().unwrap_or_else(|| self.prefix.clone()),
            suffix: patch.suffix.clone().unwrap_or_else(|| self.suffix.clone()),
            preserve_valid_identifier: patch
                .preserve_valid_identifier
                .unwrap_or(self.preserve_valid_identifier),
        }
    }
}

/// Factory / handler / zod style: affix around a cased base name.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AffixNamingRule {
    #[serde(default)]
    pub case: CaseStyle,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
}

impl Default for AffixNamingRule {
    fn default() -> Self {
        Self {
            case: CaseStyle::PascalCase,
            prefix: String::new(),
            suffix: String::new(),
        }
    }
}

impl AffixNamingRule {
    pub fn factory_default() -> Self {
        Self {
            case: CaseStyle::PascalCase,
            prefix: default_factory_prefix(),
            suffix: String::new(),
        }
    }

    pub fn handler_default() -> Self {
        Self {
            case: CaseStyle::CamelCase,
            prefix: String::new(),
            suffix: default_handler_suffix(),
        }
    }

    pub fn zod_default() -> Self {
        Self {
            case: CaseStyle::CamelCase,
            prefix: String::new(),
            suffix: default_zod_suffix(),
        }
    }

    pub fn apply_patch(&self, patch: &AffixNamingPatch) -> Self {
        Self {
            case: patch.case.unwrap_or(self.case),
            prefix: patch.prefix.clone().unwrap_or_else(|| self.prefix.clone()),
            suffix: patch.suffix.clone().unwrap_or_else(|| self.suffix.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnumMemberStrategy {
    /// Digit-prefix + clean-script (Kubb / our post-process). Default for drop-in.
    #[default]
    KubbCompat,
    /// Apply [`EnumMemberNamingRule::case`] after optional digit-prefix.
    Case,
    /// Keep raw value when it is a valid identifier; else fall back to kubb_compat.
    Preserve,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnumMemberNamingRule {
    #[serde(default)]
    pub strategy: EnumMemberStrategy,
    /// Used when `strategy = "case"`.
    #[serde(default = "default_pascal")]
    pub case: CaseStyle,
    /// Prefix `{TypeName}_` before casing when the raw key starts with a digit.
    #[serde(default = "default_true")]
    pub digit_prefix: bool,
    #[serde(default = "default_true")]
    pub preserve_screaming_snake: bool,
    #[serde(default = "default_true")]
    pub preserve_all_caps: bool,
}

impl Default for EnumMemberNamingRule {
    fn default() -> Self {
        Self {
            strategy: EnumMemberStrategy::KubbCompat,
            case: CaseStyle::PascalCase,
            digit_prefix: true,
            preserve_screaming_snake: true,
            preserve_all_caps: true,
        }
    }
}

impl EnumMemberNamingRule {
    pub fn apply_patch(&self, patch: &EnumMemberNamingPatch) -> Self {
        Self {
            strategy: patch.strategy.unwrap_or(self.strategy),
            case: patch.case.unwrap_or(self.case),
            digit_prefix: patch.digit_prefix.unwrap_or(self.digit_prefix),
            preserve_screaming_snake: patch
                .preserve_screaming_snake
                .unwrap_or(self.preserve_screaming_snake),
            preserve_all_caps: patch.preserve_all_caps.unwrap_or(self.preserve_all_caps),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum CaseStyle {
    #[default]
    PascalCase,
    CamelCase,
    SnakeCase,
}

/// Per-match patches — only set fields you want to override.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingOverride {
    #[serde(rename = "match")]
    pub target: OverrideMatch,
    /// Replace the resolved schema/type name entirely.
    #[serde(default)]
    pub rename: Option<String>,
    #[serde(default)]
    pub enum_style: Option<EnumStyle>,
    /// Per-schema object declaration style (`type` | `interface`).
    #[serde(default)]
    pub syntax: Option<ObjectSyntax>,
    #[serde(default)]
    pub schema: Option<IdentNamingPatch>,
    #[serde(default)]
    pub enum_member: Option<EnumMemberNamingPatch>,
    #[serde(default)]
    pub operation: Option<IdentNamingPatch>,
    #[serde(default)]
    pub factory: Option<AffixNamingPatch>,
    #[serde(default)]
    pub handler: Option<AffixNamingPatch>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct IdentNamingPatch {
    pub case: Option<CaseStyle>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub preserve_valid_identifier: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AffixNamingPatch {
    pub case: Option<CaseStyle>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EnumMemberNamingPatch {
    pub strategy: Option<EnumMemberStrategy>,
    pub case: Option<CaseStyle>,
    pub digit_prefix: Option<bool>,
    pub preserve_screaming_snake: Option<bool>,
    pub preserve_all_caps: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideMatch {
    pub kind: OverrideKind,
    pub pattern: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverrideKind {
    SchemaName,
    OperationId,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnumStyle {
    Enum,
    Literal,
    AsConst,
}

/// The `plugins` table: one section per plugin, keyed by plugin name.
///
/// Core does not know what a section holds. Each plugin reads its own with
/// [`PluginsConfig::section`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct PluginsConfig(BTreeMap<String, serde_json::Value>);

impl PluginsConfig {
    /// Names of the plugins that have a section.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// The `name` section parsed as `T`, or `T::default()` when the config has no
    /// such section.
    pub fn section_or_default<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        Ok(self.section(name)?.unwrap_or_default())
    }

    /// The `name` section parsed as `T`, or `None` when the config has no such
    /// section.
    pub fn section<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let Some(raw) = self.0.get(name) else {
            return Ok(None);
        };
        serde_path_to_error::deserialize(raw.clone())
            .map(Some)
            .map_err(|e| {
                let path = e.path().to_string();
                let at = if path == "." {
                    String::new()
                } else {
                    format!(".{path}")
                };
                Error::config(format!("plugins.{name}{at}: {}", e.inner()))
            })
    }
}

/// Top-level declaration style for object-like schemas.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ObjectSyntax {
    /// `export type Foo = { … }` (Kubb-compatible default).
    #[default]
    Type,
    /// `export interface Foo { … }` when the schema is a non-nullable object.
    Interface,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::config(format!("failed to read {}: {e}", path.display())))?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.finish()
    }

    /// Parses a config already resolved to JSON (e.g. by the JS/TS config loader
    /// in the npm package, which evaluates `specmint.config.ts` and hands back its
    /// default export as JSON — `Config`'s shape is otherwise format-agnostic).
    pub fn from_json_str(raw: &str) -> Result<Self> {
        let cfg: Self = serde_json::from_str(raw)?;
        cfg.finish()
    }

    fn finish(mut self) -> Result<Self> {
        self.naming = self.naming.normalize();
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<()> {
        if self.source.input.trim().is_empty() {
            return Err(Error::config("source.input must not be empty".to_string()));
        }
        for ovr in &self.naming.overrides {
            Regex::new(&ovr.target.pattern).map_err(|e| {
                Error::config(format!(
                    "invalid naming override regex {:?}: {e}",
                    ovr.target.pattern
                ))
            })?;
        }
        Ok(())
    }
}
