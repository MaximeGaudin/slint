# slint

The linter for Agent Skills — ESLint-shaped, static-first, optional model pass.

## Layout

| Path | What it is |
|------|------------|
| [`apps/cli`](apps/cli) | `slint` binary (Rust, Cargo workspace) |
| [`apps/vscode`](apps/vscode) | VS Code / Cursor extension |
| [`apps/docs`](apps/docs) | Documentation and product brief |
| [`packages/core`](packages/core) | Shared Rust library (`slint`) used by the CLI |

JS/TS apps use **pnpm workspaces** + **Turborepo**. Rust stays on a **Cargo workspace** — that is the right tool for the CLI/core crates; Turbo orchestrates the Node side (and can wrap `cargo` scripts later if useful).

## Quick start

```bash
pnpm install

# CLI
cargo install --path apps/cli
slint --help
cargo test --workspace   # or: pnpm test:cli

# VS Code extension
pnpm build:vscode
# or everything Turbo knows about:
pnpm build
```

See [`apps/docs/README.md`](apps/docs/README.md) for usage, rules, and configuration.
