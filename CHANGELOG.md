# Changelog

All notable changes to slint are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-14

### Added

- **CLI** — Initial `slint` binary: static-first Agent Skills linting, computed `--fix`, optional `--llm` model pass, plugins (TOML rule packs and Extism Wasm), and reporters (`stylish`, `json`, `github`, `compact`).
- **Rules** — Built-in catalogue covering name, description, frontmatter, body, bundle, project, and model-assisted checks; every finding cites its source document.
- **Docs** — Astro + MDX site with a routing-signal design; rule pages generated from `slint rules --json`.
- **Editors** — VS Code / Cursor extension that runs the CLI on save and surfaces citations on diagnostics.
- **Tooling** — pnpm + Turborepo monorepo scripts (`pnpm build`, `pnpm lint`, `pnpm install:cli`, coverage via `cargo-llvm-cov`), Clippy/rustfmt, Biome for docs/vscode, comment checks that reject `TODO` / `FIXME` / stub macros, GitHub Actions CI, and a cross-platform release workflow.
