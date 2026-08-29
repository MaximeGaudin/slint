# Contributing to slint

Thanks for wanting to contribute. This file is the short version that gets
you building and opening PRs; [the docs site's contributing page]
(https://slint.dev/contributing/) has the long one about how the crate is
laid out and how to write a rule.

## Getting set up

Requirements: a stable Rust toolchain (pinned via `rust-toolchain.toml`;
the workspace MSRV is 1.95), Node 22 and pnpm 11.

```bash
git clone https://github.com/MaximeGaudin/slint
cd slint
pnpm install
cargo test --workspace   # the whole suite, including the CLI end to end
cargo run -p slint-cli -- .
```

## Layout

| Path | What lives there |
| --- | --- |
| `packages/core` | The library: skill parsing, rules, engine, LLM pass, plugins, fixes, reporters. |
| `apps/cli` | The `slint` binary. |
| `apps/vscode` | VS Code / Cursor extension — runs the binary, does not reimplement it. |
| `apps/docs` | The docs site (Astro + MDX); rule pages are generated from `slint rules --json`. |

## Checks to run before opening a PR

`./scripts/check.sh` mirrors CI. CI runs:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings` (stub macros and
  `dbg!` are denied via workspace lints)
- `cargo test --workspace --locked`
- `node scripts/check-no-todos.mjs` — no `TODO` / `FIXME` / `XXX` comments;
  stub macros (`todo!`, `unimplemented!`) are denied by clippy instead
- `cargo check --workspace --locked` against MSRV 1.95
- `cargo deny check` (advisories, bans, licenses, sources)
- JS: `pnpm exec turbo run lint --filter=slint-docs --filter=slint-vscode`
  (Biome, `tsc`, `astro check`, plus the no-todos check for the docs site)
  and the vscode unit tests
- Coverage via `cargo llvm-cov --workspace --summary-only` (informational —
  no threshold gates a PR)

`check.sh` runs the fmt/clippy/deny/no-todos/test/JS set locally and skips
what is matrix- or CI-only (MSRV, coverage upload).

Two checks are easy to miss:

- **Tests are named after behaviour**, not functions: `test a_name_that_says_what_it_protects`.
  The suite is where the design is written down; a bug fix starts with a
  failing regression test.
- **The rule catalogue is synced, not copied.** After changing any rule,
  regenerate `apps/docs/src/data/rules.json` from the binary:

  ```bash
  pnpm sync:docs
  ```

  The site never describes a rule from memory — it renders what `slint
  rules --json` says.

## What a change needs

- **Every rule cites a source.** `reference_title` / `reference_url` are not
  optional; a rule that cannot cite anything is a plugin, not a built-in.
- **Every rule carries advice.** Naming a problem without saying what to do
  is half a rule.
- **Static before model.** If a regex can answer it, it belongs in
  `packages/core/src/rules/`, not in the LLM prompt.
- **No model-written fixes.** `--fix` is computed, always.

## Opening the PR

- Fill in the PR template: what changed, which issues it fixes, how it was
  tested.
- Keep fixes focused; unrelated refactors belong in their own PR.
- CI must be green before merge; if a check fails locally after `./scripts/check.sh`,
  fix it here rather than hoping CI disagrees.

## Reporting bugs and vulnerabilities

Bugs and rule-quality reports: a GitHub issue using the bug template
(include a minimal reproduction — the smallest skill that triggers it).

Security issues: **not** a public issue. See [SECURITY.md](SECURITY.md) for
the private reporting path and the threat model.
