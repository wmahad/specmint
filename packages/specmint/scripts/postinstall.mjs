#!/usr/bin/env node
// Downloads the prebuilt native addon for this platform from a GitHub Release
// (skipped if already built locally). A GitHub token is optional; it only raises
// the API rate limit.

const GITHUB_REPO = "wmahad/specmint";

import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { nodeFileName, platformArchString } from "../src-js/platform.mjs";

const rootDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function githubToken() {
  return (
    process.env.GITHUB_TOKEN ||
    process.env.GH_TOKEN ||
    process.env.GH_PACKAGES_TOKEN ||
    process.env.NODE_AUTH_TOKEN ||
    ""
  );
}

function authHeaders() {
  const token = githubToken();
  if (!token) return {};
  return { Authorization: `Bearer ${token}` };
}

async function download(url, headers) {
  const res = await fetch(url, {
    headers: {
      Accept: "application/octet-stream",
      "User-Agent": "specmint-postinstall",
      ...headers,
    },
    redirect: "follow",
  });
  return res;
}

async function downloadViaApi(version, nodeFile, headers) {
  const releaseUrl = `https://api.github.com/repos/${GITHUB_REPO}/releases/tags/specmint-v${version}`;
  const releaseRes = await fetch(releaseUrl, {
    headers: {
      Accept: "application/vnd.github+json",
      "User-Agent": "specmint-postinstall",
      ...headers,
    },
  });
  if (!releaseRes.ok) {
    return releaseRes;
  }
  const release = await releaseRes.json();
  const asset = (release.assets || []).find((a) => a.name === nodeFile);
  if (!asset) {
    return {
      ok: false,
      status: 404,
      statusText: "Asset not listed on release",
    };
  }
  return download(asset.url, headers);
}

async function main() {
  // Source checkout (the monorepo): build locally instead of downloading.
  if (existsSync(path.join(rootDir, "../../crates/specmint-napi/Cargo.toml"))) {
    return;
  }

  const target = platformArchString();
  if (!target) {
    console.error(
      `specmint: unsupported platform ${process.platform}-${process.arch}. ` +
        "Supported: darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc."
    );
    process.exit(1);
  }

  const nodeFile = nodeFileName("specmint", target);
  const nodePath = path.join(rootDir, nodeFile);
  if (existsSync(nodePath)) {
    return;
  }

  const { version } = JSON.parse(await readFile(path.join(rootDir, "package.json"), "utf8"));
  const browserUrl = `https://github.com/${GITHUB_REPO}/releases/download/specmint-v${version}/${nodeFile}`;
  const headers = authHeaders();
  const tokenPresent = Boolean(githubToken());

  console.log(`specmint: downloading native addon from ${browserUrl}`);

  // Browser download URLs 404 for private repos even with a token, so prefer the API.
  let res = tokenPresent
    ? await downloadViaApi(version, nodeFile, headers)
    : await download(browserUrl, headers);

  if (!res.ok && tokenPresent) {
    res = await download(browserUrl, headers);
  }

  if (!res.ok) {
    const hint = tokenPresent
      ? "Is there a GitHub Release published for this version, and does your token have `repo` (or Contents: Read) access?"
      : "Is there a GitHub Release published for this version? Setting GITHUB_TOKEN can help if you hit the API rate limit.";
    console.error(`specmint: failed to download ${browserUrl} (HTTP ${res.status}). ${hint}`);
    process.exit(1);
  }
  await writeFile(nodePath, Buffer.from(await res.arrayBuffer()));
}

main();
