// specmint.config.ts — example config for specmint (TS/JS equivalent of specmint.example.toml)
//
// Kubb cutover isn't done yet — keep using `yarn generate-api-types` for
// production generation until it is.
//
// Loaded by Node's native TypeScript support (no bundler) when the installed
// `specmint` binary runs and finds `specmint.config.ts` in the working directory.
// `defineConfig` is just an identity function for editor autocomplete — see
// `specmint/config`'s types for the full shape.

import { defineConfig } from "specmint/config";

export default defineConfig({
  source: {
    // File path **or** http(s) URL — one field:
    //   input: "https://petstore3.swagger.io/api/v3/openapi.json"
    //   input: "./tests/fixtures/tiny.openapi.yaml"
    //   input: ".cache/openapi/schema.yml"
    // Auth for URLs (never commit secrets):
    //   export SPECMINT_BASIC_AUTH='username:password'
    input: "https://petstore3.swagger.io/api/v3/openapi.json",
    cache: {
      dir: ".cache/openapi",
      use_etag: true,
      use_content_hash: true,
    },
  },

  output: {
    // Side-by-side evaluation (default):
    path: "./.generated-rust",
    // Cutover / usage probe — overwrite Kubb output and run tsc/tests:
    // path: "../src/@generated",
    clean: true,
    banner: `/* oxlint-disable typescript/no-unused-vars */
// @ts-nocheck
/*
 * ---------------------------------------------------------------
 * ## THIS FILE WAS GENERATED FROM API
 * ## Do not edit manually.
 * ---------------------------------------------------------------
 */`,
  },

  adapter: {
    validate: false,
  },

  naming: {
    // Naming shared by every plugin: schema, enum-member and operation names.
    // Names only one plugin writes are set in that plugin's section below.
    schema: {
      case: "PascalCase",
      // prefix: "",
      // suffix: "",
      preserve_valid_identifier: true, // keeps HTTPHeader, etc.
    },
    enum_member: {
      strategy: "kubb_compat", // kubb_compat | case | preserve
      digit_prefix: true, // 5_5_ETAG → TypeName55ETAG
      preserve_screaming_snake: true,
      preserve_all_caps: true,
    },
    operation: {
      case: "PascalCase",
    },
    // overrides: [
    //   {
    //     match: { kind: "schema_name", pattern: "^LegacyPet$" },
    //     rename: "Pet",
    //     // Other fields:
    //     // schema: { prefix: "I", suffix: "Dto" },
    //     // enum_member: { strategy: "case", case: "PascalCase", digit_prefix: false },
    //     // operation: { prefix: "Api" }, // with kind: "operation_id"
    //   },
    // ],
  },

  plugins: {
    types: {
      enabled: true,
      file: "types.ts",
      enum_style: "enum",
      // Object schemas: "type" (Kubb default) or "interface".
      // Unions / intersections / nullable objects always stay as `type`.
      syntax: "type",
      strip_readonly: true,
      compact: true,
      overrides: [
        {
          match: { kind: "schema_name", pattern: "Discriminator$" },
          enum_style: "literal",
          // syntax: "interface", // these schemas only
        },
      ],
    },
    factories: {
      enabled: false,
      file: "factories.ts",
      seed: 2,
      // prefix + type name + suffix (the type name is used as is)
      naming: { prefix: "create", suffix: "" },
      // overrides: [{ match: { kind: "schema_name", pattern: "^Pet$" }, naming: { prefix: "build" } }],
    },
    msw: {
      enabled: false,
      file: "handlers.ts",
      parser: "faker", // "faker" needs factories on; "data" does not
      base_url: "*",
      naming: { case: "CamelCase", prefix: "", suffix: "Handler" },
      // overrides: [{ match: { kind: "operation_id", pattern: "^getPet$" }, naming: { suffix: "Msw" } }],
    },
    zod: {
      enabled: false,
      file: "zod.ts",
      inferred: true,
      naming: { case: "CamelCase", prefix: "", suffix: "Schema" },
    },
  },
});
