#!/usr/bin/env node
// Published `specmint` CLI: load the napi binding, then resolve
// TS/JS/TOML config from the process working directory.
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

import { nodeFileName, platformArchString } from "../src-js/platform.mjs";

const require = createRequire(import.meta.url);
const rootDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function loadBinding() {
  const target = platformArchString();
  if (!target) {
    console.error(`specmint: unsupported platform ${process.platform}-${process.arch}`);
    process.exit(1);
  }
  const nodePath = path.join(rootDir, nodeFileName("specmint", target));
  if (!existsSync(nodePath)) {
    console.error(`specmint: native binding not found at ${nodePath}. Try reinstalling.`);
    process.exit(1);
  }
  return require(nodePath);
}

// Priority matches other JS-ecosystem tools (vite, eslint, ...): TS config
// first, then plain JS, falling back to the native TOML format. `.mts` is
// preferred over `.ts` when both would apply, since it's unambiguously ESM to
// Node (no package.json "type" field needed, no CJS-then-ESM reparse warning).
const JS_CONFIG_NAMES = [
  "specmint.config.mts",
  "specmint.config.ts",
  "specmint.config.js",
  "specmint.config.mjs",
];

function findConfig(cwd) {
  for (const name of JS_CONFIG_NAMES) {
    const candidate = path.join(cwd, name);
    if (existsSync(candidate)) return { kind: "js", path: candidate };
  }
  const tomlPath = path.join(cwd, "specmint.toml");
  if (existsSync(tomlPath)) return { kind: "toml", path: tomlPath };
  return null;
}

// Node's native TypeScript stripping (no bundler involved) needs a modern
// enough Node to import a bare `.ts` file. Give a clear hint instead of a
// raw ERR_UNKNOWN_FILE_EXTENSION stack trace when it's missing.
function explainConfigImportError(err, configPath) {
  if (
    (configPath.endsWith(".ts") || configPath.endsWith(".mts")) &&
    (err?.code === "ERR_UNKNOWN_FILE_EXTENSION" ||
      err?.code === "ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING")
  ) {
    return (
      `specmint: couldn't load ${configPath} — this Node version (${process.version}) doesn't support ` +
      "importing .ts files natively. Upgrade Node (22.18+/23.6+ enable this by default), or rename the " +
      "config to specmint.config.mjs.\n" +
      `  original error: ${err.message}`
    );
  }
  return `specmint: failed to load ${configPath}: ${err.message}`;
}

async function loadJsConfig(configPath) {
  let mod;
  try {
    mod = await import(pathToFileURL(configPath).href);
  } catch (err) {
    console.error(explainConfigImportError(err, configPath));
    process.exit(1);
  }
  const config = mod.default;
  if (typeof config !== "object" || config === null || Array.isArray(config)) {
    console.error(`specmint: ${configPath} must have a default export that is a config object.`);
    process.exit(1);
  }
  return config;
}

async function main() {
  const cwd = process.cwd();
  const config = findConfig(cwd);
  if (!config) {
    console.error(`specmint: no specmint.config.mts/.ts/.js/.mjs or specmint.toml found in ${cwd}`);
    process.exit(1);
  }

  const binding = loadBinding();
  try {
    const report =
      config.kind === "toml"
        ? binding.generateFromPath(config.path)
        : binding.generate(JSON.stringify(await loadJsConfig(config.path)));
    for (const file of report.written) console.log(file);
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exitCode = 1;
  }
}

// adding this to triger another release.
main();
