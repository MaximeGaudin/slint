# Changelog

All notable changes to the slint VS Code extension are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Ignore quick fix** — Ignore a finding from the editor's quick-fix menu with a `<!-- slint-disable -->` (whole file) or `<!-- slint-disable-next-line -->` comment, placed correctly even when the finding sits inside a fenced code block ([#17](https://github.com/MaximeGaudin/slint/pull/17)).
- **Run cancellation** — Starting a new lint for the same file cancels the in-flight `slint` process, so superseded runs no longer leave stale diagnostics or a stuck spinner ([#21](https://github.com/MaximeGaudin/slint/pull/21)).

### Changed

- **Marketplace packaging** — The extension now ships a README, this changelog, an icon, and a complete manifest (`extensionKind: workspace`, keywords, bugs URL, gallery banner), and CI packages/validates the manifest on every run.

## [0.2.0] - 2026-08-20

### Changed

- Moved with the monorepo to pnpm workspaces + Turborepo and the hardened JS quality gates (Biome, `tsc --noEmit`, comment checks).

## [0.1.0] - 2026-08-14

### Added

- **Initial release** — Lints `SKILL.md` files on save (static rules, or static rules plus the model pass), optional debounced lint-while-typing, diagnostics with rule citations on hover, a quick fix that applies the CLI's computed fixes, and workspace-wide lint commands.
