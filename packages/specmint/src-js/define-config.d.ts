// Mirrors the shape `specmint`'s Rust `Config` struct deserializes (the same
// field names as `specmint.example.toml`, since the config is handed to Rust as
// JSON with no field renaming). Keep in sync with
// `crates/specmint-core/src/config.rs`.

export type CaseStyle = "PascalCase" | "CamelCase" | "SnakeCase";
export type EnumMemberStrategy = "kubb_compat" | "case" | "preserve";
export type EnumStyle = "enum" | "literal" | "as_const";
export type ObjectSyntax = "type" | "interface";
export type OverrideKind = "schema_name" | "operation_id";

export interface SpecmintSourceCache {
  dir?: string;
  use_etag?: boolean;
  use_content_hash?: boolean;
}

export interface SpecmintSource {
  /** Local file path **or** an `http(s)://` URL. */
  input: string;
  cache?: SpecmintSourceCache;
}

export interface SpecmintOutput {
  path: string;
  clean?: boolean;
  banner?: string;
}

export interface SpecmintAdapter {
  validate?: boolean;
}

export interface SpecmintIdentNamingRule {
  case?: CaseStyle;
  prefix?: string;
  suffix?: string;
  preserve_valid_identifier?: boolean;
}

export interface SpecmintEnumMemberNamingRule {
  strategy?: EnumMemberStrategy;
  /** Used when `strategy` is `"case"`. */
  case?: CaseStyle;
  digit_prefix?: boolean;
  preserve_screaming_snake?: boolean;
  preserve_all_caps?: boolean;
}

export interface SpecmintIdentNamingPatch {
  case?: CaseStyle;
  prefix?: string;
  suffix?: string;
  preserve_valid_identifier?: boolean;
}

export interface SpecmintAffixNamingPatch {
  case?: CaseStyle;
  prefix?: string;
  suffix?: string;
}

export interface SpecmintEnumMemberNamingPatch {
  strategy?: EnumMemberStrategy;
  case?: CaseStyle;
  digit_prefix?: boolean;
  preserve_screaming_snake?: boolean;
  preserve_all_caps?: boolean;
}

export interface SpecmintMatch {
  kind: OverrideKind;
  pattern: string;
}

/** An affix patch for the names whose `match` applies. */
export interface SpecmintAffixOverride {
  match: SpecmintMatch;
  naming: SpecmintAffixNamingPatch;
}

export interface SpecmintNamingOverride {
  match: SpecmintMatch;
  /** Replace the resolved schema/type name entirely. */
  rename?: string;
  schema?: SpecmintIdentNamingPatch;
  enum_member?: SpecmintEnumMemberNamingPatch;
  operation?: SpecmintIdentNamingPatch;
}

export interface SpecmintNaming {
  schema?: SpecmintIdentNamingRule;
  enum_member?: SpecmintEnumMemberNamingRule;
  operation?: SpecmintIdentNamingRule;
  overrides?: SpecmintNamingOverride[];
}

export interface SpecmintTypesPlugin {
  enabled?: boolean;
  file?: string;
  enum_style?: EnumStyle;
  /** Unions / intersections / nullable objects always fall back to `"type"`. */
  syntax?: ObjectSyntax;
  strip_readonly?: boolean;
  compact?: boolean;
  /** Per-schema `enum_style` / `syntax`, matched by schema name. */
  overrides?: {
    match: SpecmintMatch;
    enum_style?: EnumStyle;
    /** Per-schema object declaration style (`"type"` | `"interface"`). */
    syntax?: ObjectSyntax;
  }[];
}

export interface SpecmintFactoriesPlugin {
  enabled?: boolean;
  file?: string;
  seed?: number;
  /**
   * `prefix + type name + suffix`; the type name is used as is, so `case` has no effect.
   * Keys left out keep their defaults (prefix `"create"`, no suffix).
   */
  naming?: SpecmintAffixNamingPatch;
  /** Matched (`kind: "schema_name"`) against the type name the factory builds. */
  overrides?: SpecmintAffixOverride[];
}

export interface SpecmintMswPlugin {
  enabled?: boolean;
  file?: string;
  /**
   * `"faker"` seeds success bodies from factories and needs the factories
   * plugin on; `"data"` requires caller data and works without it.
   */
  parser?: "faker" | "data";
  base_url?: string;
  handlers_barrel?: boolean;
  /**
   * `prefix + operation id cased by case + suffix`.
   * Keys left out keep their defaults (`"CamelCase"`, no prefix, `"Handler"`).
   */
  naming?: SpecmintAffixNamingPatch;
  /** Matched (`kind: "operation_id"`) against the operation id. */
  overrides?: SpecmintAffixOverride[];
}

export interface SpecmintZodPlugin {
  enabled?: boolean;
  file?: string;
  inferred?: boolean;
  /**
   * `prefix + type name cased by case + suffix`.
   * Keys left out keep their defaults (`"CamelCase"`, no prefix, `"Schema"`).
   */
  naming?: SpecmintAffixNamingPatch;
}

/** One section per plugin, keyed by plugin name. */
export interface SpecmintPlugins {
  types?: SpecmintTypesPlugin;
  factories?: SpecmintFactoriesPlugin;
  msw?: SpecmintMswPlugin;
  zod?: SpecmintZodPlugin;
}

export interface SpecmintConfig {
  source: SpecmintSource;
  output: SpecmintOutput;
  adapter?: SpecmintAdapter;
  naming?: SpecmintNaming;
  plugins?: SpecmintPlugins;
}

/** Identity helper for editor autocomplete — mirrors `defineConfig` in vite/vitest/etc. */
export declare function defineConfig(config: SpecmintConfig): SpecmintConfig;
