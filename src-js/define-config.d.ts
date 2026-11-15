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
  discriminator?: string;
  enum_suffix?: string;
}

export interface SpecmintIdentNamingRule {
  case?: CaseStyle;
  prefix?: string;
  suffix?: string;
  preserve_valid_identifier?: boolean;
}

export interface SpecmintAffixNamingRule {
  case?: CaseStyle;
  prefix?: string;
  suffix?: string;
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

export interface SpecmintNamingOverride {
  match: { kind: OverrideKind; pattern: string };
  /** Replace the resolved schema/type name entirely. */
  rename?: string;
  enum_style?: EnumStyle;
  /** Per-schema object declaration style (`"type"` | `"interface"`). */
  syntax?: ObjectSyntax;
  schema?: SpecmintIdentNamingPatch;
  enum_member?: SpecmintEnumMemberNamingPatch;
  operation?: SpecmintIdentNamingPatch;
  factory?: SpecmintAffixNamingPatch;
  handler?: SpecmintAffixNamingPatch;
}

export interface SpecmintNaming {
  schema?: SpecmintIdentNamingRule;
  enum_member?: SpecmintEnumMemberNamingRule;
  operation?: SpecmintIdentNamingRule;
  factory?: SpecmintAffixNamingRule;
  handler?: SpecmintAffixNamingRule;
  zod?: SpecmintAffixNamingRule;
  overrides?: SpecmintNamingOverride[];
}

export interface SpecmintTypesEmitter {
  enabled?: boolean;
  file?: string;
  enum_style?: EnumStyle;
  /** Unions / intersections / nullable objects always fall back to `"type"`. */
  syntax?: ObjectSyntax;
  strip_readonly?: boolean;
  compact?: boolean;
}

export interface SpecmintFactoriesEmitter {
  enabled?: boolean;
  file?: string;
  seed?: number;
}

export interface SpecmintMswEmitter {
  enabled?: boolean;
  file?: string;
  /** `"faker"` seeds success bodies from factories; `"data"` requires caller data. */
  parser?: string;
  base_url?: string;
  handlers_barrel?: boolean;
}

export interface SpecmintZodEmitter {
  enabled?: boolean;
  file?: string;
  inferred?: boolean;
}

export interface SpecmintEmitters {
  types?: SpecmintTypesEmitter;
  factories?: SpecmintFactoriesEmitter;
  msw?: SpecmintMswEmitter;
  zod?: SpecmintZodEmitter;
}

export interface SpecmintConfig {
  source: SpecmintSource;
  output: SpecmintOutput;
  adapter?: SpecmintAdapter;
  naming?: SpecmintNaming;
  emitters?: SpecmintEmitters;
}

/** Identity helper for editor autocomplete — mirrors `defineConfig` in vite/vitest/etc. */
export declare function defineConfig(config: SpecmintConfig): SpecmintConfig;
