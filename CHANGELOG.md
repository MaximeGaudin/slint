# Changelog

All notable changes to slint are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **JSON envelope versioning** — `--format json` opens with `schemaVersion: 1`, and the full envelope shape is documented field by field at slint.dev/json.
- **WebAssembly plugin ABI spec** — the wire contract for plugin authors (export, input, output, validation, failure semantics) is specified field by field at slint.dev/plugin-abi, written from the implementation.
- **Community files** — `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md` (vulnerability reporting plus the threat model for plugin execution, LLM transmission, and cloned-repo configs), and GitHub issue/PR templates.

### Changed

- **Suppression comments** — A `slint-disable` comment is scoped to the document it is written in: it no longer silences findings on the files bundled beside `SKILL.md`.
- **Suppression comments** — A directive inside a fenced code block is documentation of the syntax, not a live directive, so showing an example no longer silences the rest of the document.
- **Suppression comments** — A directive that suppressed nothing (for example a misspelled rule name) is now reported as a `suppression/unused` warning, the way ESLint reports unused disable directives. The warning counts toward `--max-warnings` and can be retuned or turned off in config.

### Fixed

- **Dogfooding** — slint passes on its own bundled `.cursor/skills` again: the fix-github-issue skill's pre-flight script reference is written as a repo-root path (`./scripts/check.sh`) instead of a bundled one, and a regression test keeps the shipped skills passing slint itself.

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
