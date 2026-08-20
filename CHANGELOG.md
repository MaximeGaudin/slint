# Changelog

All notable changes to slint are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-08-20

### Added

- **`frontmatter/unknown-field`** — Warn on non-spec top-level frontmatter keys so product-specific options move under `metadata`.
- **`body/undeclared-tool`** — Flag host-private tools (e.g. AskQuestion) that lack `allowed-tools` or a portable fallback.
- **`body/hardcoded-repo-path`** — Warn when instructions hardcode consumer-repo paths without a missing-path discovery, override, or stop gate.
- **`body/imperative-instructions`** — Prefer direct imperative steps over conversational or passive wording.
- **`bundle/script-prerequisites`** — Require a Prerequisites (or Requirements / Compatibility) section when a skill ships scripts.

### Changed

- **`bundle/unused-file`** — Misplaced companions outside standard skill directories get a layout diagnosis instead of an “unreferenced” message.
- **Inline disables** — `slint-disable-next-line` before a fenced code block now covers example paths inside the fence (not only the opening marker).

## [0.1.0] - 2026-08-14

### Added

- **CLI** — Initial `slint` binary: static-first Agent Skills linting, computed `--fix`, optional `--llm` model pass, plugins (TOML rule packs and Extism Wasm), and reporters (`stylish`, `json`, `github`, `compact`).
- **Rules** — Built-in catalogue covering name, description, frontmatter, body, bundle, project, and model-assisted checks; every finding cites its source document.
- **Docs** — Astro + MDX site with a routing-signal design; rule pages generated from `slint rules --json`.
- **Editors** — VS Code / Cursor extension that runs the CLI on save and surfaces citations on diagnostics.
- **Tooling** — pnpm + Turborepo monorepo scripts (`pnpm build`, `pnpm lint`, `pnpm install:cli`, coverage via `cargo-llvm-cov`), Clippy/rustfmt, Biome for docs/vscode, comment checks that reject `TODO` / `FIXME` / stub macros, GitHub Actions CI, and a cross-platform release workflow.
