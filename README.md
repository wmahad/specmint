# specmint

OpenAPI → TypeScript artifacts generator (Rust core, napi-rs Node bindings).

| Output                | Purpose                                |
| --------------------- | -------------------------------------- |
| `types.ts`            | Types, enums, operation helpers        |
| `factories.ts`        | Faker `createX()` factories            |
| `handlers.ts`         | MSW handlers (optional Zod via config) |

Not an SDK generator — no RTK Query, fetch clients, or React hooks.

## Install

```bash
pnpm add -D specmint

# any of these work once installed (bin is on PATH via the package manager)
pnpm specmint
yarn specmint
npx specmint
```

`postinstall` downloads the matching `specmint.<platform>.node` from the GitHub
Release (tag `specmint-v<version>`). No token is needed. If you hit the GitHub
API rate limit, set `GITHUB_TOKEN` (also read: `GH_TOKEN`, `GH_PACKAGES_TOKEN`,
`NODE_AUTH_TOKEN`). In a source checkout, run `pnpm build` instead.

## Config

Looks for, in order: `specmint.config.mts` → `.ts` → `.js` / `.mjs` → `specmint.toml`.

```ts
import { defineConfig } from "specmint/config";

export default defineConfig({
  source: { input: "https://example.com/openapi.json" }, // or a local file path
  output: { path: "./.generated", clean: true },
  emitters: {
    types: { enabled: true },
    factories: { enabled: true },
    msw: { enabled: true },
  },
});
```

Full field reference: [`specmint.example.toml`](specmint.example.toml) /
[`specmint.config.example.mts`](specmint.config.example.mts).

URL inputs use ETag caching under `.cache/openapi/`. Auth via
`SPECMINT_BASIC_AUTH='user:password'`. Emit is skipped when the parsed-spec hash
is unchanged (`--force` overrides on the Cargo CLI).

## CLI

**npm bin** — auto-resolves config in the current directory, no flags:

```bash
pnpm specmint    # or: yarn specmint / npx specmint / pnpm exec specmint
```

Or add a script in the consuming app (`"generate-api": "specmint"`) and run
`pnpm generate-api` / `yarn generate-api`.

**Cargo binary** — full flags:

```bash
export SPECMINT_BASIC_AUTH='user:password'
cargo run -p specmint-cli -- generate --config specmint.toml --force --timings
cargo run -p specmint-cli -- generate --config specmint.toml --mode types
cargo run -p specmint-cli -- generate --config specmint.toml --emitters types,factories,msw
```

## Architecture

```
crates/
├── specmint-core/   # pipeline: config → source → adapter → naming → emit → write
├── specmint-cli/    # `specmint` binary
└── specmint-napi/   # Node addon (cdylib)
```

Emitters (types → factories → msw → zod) are compiled in and toggled via config.

## Develop

```bash
pnpm build          # napi addon for this platform (+ index.js / index.d.ts)
pnpm run lint       # oxlint on bin / src-js / scripts
pnpm run format     # rustfmt
pnpm run typecheck  # cargo clippy -D warnings
pnpm run test       # cargo test --workspace
```

Generated (do not commit): `*.node`, `index.js`, `index.d.ts`, `target/`.

## Packaging

- npm package: `specmint` (JS stubs + bin; no `.node`)
- Native binaries: GitHub Release assets on `specmint-v*`, fetched by `postinstall`
- Platforms: `darwin-arm64`, `darwin-x64`, `linux-x64-gnu`, `linux-arm64-gnu`, `win32-x64-msvc`
