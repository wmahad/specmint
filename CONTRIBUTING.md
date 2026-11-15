# Contributing to specmint

Thanks for helping. Bug reports, fixes and new kubb features are all welcome.

## Before you start

- For bugs and planned work, use [Issues](https://github.com/wmahad/specmint/issues). Issues labelled `ready-for-agent` or `ready-for-human` are ready to pick up.
- For questions and ideas, use [Discussions](https://github.com/wmahad/specmint/discussions).
- For anything bigger than a small fix, comment on the issue first so we can agree on the approach.

`GLOSSARY.md` defines the project's terms, and `docs/adr/` records decisions that are already made. Please use the glossary's words in code, issues and docs.

## Setup

You need Rust (stable, via [rustup](https://rustup.rs)) and Node 22.18 or newer.

```bash
cargo test --workspace                                # run the tests
cargo clippy --workspace --all-targets -- -D warnings # lint the Rust code
cargo fmt --all                                       # format the Rust code
```

To build the Node addon for your platform, install the npm dependencies and run `npm run build`.

## Pull requests

- Keep each PR to one change, and link the issue it closes.
- Add or update tests for the change.
- If the generated output changes, say so in the PR and explain why.
- CI must pass, and a maintainer reviews every PR before it is merged. PRs are squash-merged, so the PR title becomes the commit message: write it as a short summary of the change.

By contributing, you agree that your work is licensed under the [MIT License](LICENSE), and you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
