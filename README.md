# slint

The linter for Agent Skills — ESLint-shaped, static-first, optional model pass.

This is a monorepo:

| Path | What it is |
|------|------------|
| [`apps/cli`](apps/cli) | `slint` binary (Rust) |
| [`apps/vscode`](apps/vscode) | VS Code / Cursor extension |
| [`apps/docs`](apps/docs) | Documentation and product brief |
| [`packages/core`](packages/core) | Shared Rust library (`slint`) used by the CLI |

## Why not Nx?

The heavy lifting is a Cargo workspace. The VS Code extension is a thin Node shell around the binary. Nx adds little until there are several JS/TS apps with shared packages — we can add it then without reshaping the Rust side.

## Quick start

```bash
# CLI
cargo install --path apps/cli
slint --help

# Tests
cargo test --workspace

# VS Code extension
npm install
npm run build -w slint-vscode
```

See [`apps/docs/README.md`](apps/docs/README.md) for usage, rules, and configuration.
