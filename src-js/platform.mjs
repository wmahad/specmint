// Maps the running Node process to the platform-arch string napi-rs uses for
// the compiled `.node` file name (e.g. `specmint.darwin-arm64.node`). Shared by
// the postinstall downloader and the CLI's binding loader so both agree on
// exactly one supported-platform list.
//
// musl/Alpine Linux isn't distinguished from glibc yet (always resolves to
// "gnu") — fine for now, revisit if that's actually needed.

const SUPPORTED = {
  "darwin-arm64": ["darwin", "arm64"],
  "darwin-x64": ["darwin", "x64"],
  "linux-x64-gnu": ["linux", "x64"],
  "linux-arm64-gnu": ["linux", "arm64"],
  "win32-x64-msvc": ["win32", "x64"],
};

export function platformArchString() {
  const { platform, arch } = process;
  for (const [key, [p, a]] of Object.entries(SUPPORTED)) {
    if (p === platform && a === arch) return key;
  }
  return null;
}

export function nodeFileName(binaryName, target) {
  return `${binaryName}.${target}.node`;
}
