# specmint

OpenAPI → TypeScript code generator with a Rust core and Node bindings. A port of [kubb](https://kubb.dev), aiming for feature parity with kubb v5 in v1.

| Output                | Purpose                                |
| --------------------- | -------------------------------------- |
| `types.ts`            | Types, enums, operation helpers        |
| `factories.ts`        | Faker `createX()` factories            |
| `handlers.ts`         | MSW handlers (optional Zod via config) |
| `zod.ts`              | Zod schemas                            |

Planned for v1, to match kubb's plugins: HTTP clients (fetch and axios), TanStack Query hooks for React and Vue, SWR hooks, Cypress request helpers, an MCP server generator and Redoc API docs. All of them ship inside the one `specmint` package and are switched on in config.

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
  plugins: {
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
cargo run -p specmint-cli -- generate --config specmint.toml --plugins types,factories,msw
```

## Repository layout

A pnpm workspace with Turborepo on top, and a Cargo workspace for the Rust code.

```
crates/
├── specmint-core/   # pipeline: config → source → adapter → naming → emit → write
├── specmint-cli/    # Rust CLI, for development and benchmarks
└── specmint-napi/   # Node addon (cdylib)
packages/
└── specmint/        # the npm package users install (bin, config helper, addon loader)
apps/docs/           # docs site (coming in #4)
examples/            # example projects (coming in #7)
benchmarks/          # specmint vs kubb (coming in #9)
```

Each Rust crate has a small private `package.json` whose scripts call cargo, so Turborepo can run and cache them. Plugins (types → factories → msw → zod) are compiled into the addon and switched on in config.

## Develop

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and the main commands (`pnpm install`, `pnpm build`, `pnpm test`, `pnpm lint`).

Generated and not committed: `target/`, and the addon files (`*.node`, `index.js`, `index.d.ts`) in `packages/specmint`.

## Packaging

- npm package: `specmint` (JS stubs + bin; no `.node`)
- Native binaries: GitHub Release assets on `specmint-v*`, fetched by `postinstall`
- Platforms: `darwin-arm64`, `darwin-x64`, `linux-x64-gnu`, `linux-arm64-gnu`, `win32-x64-msvc`
