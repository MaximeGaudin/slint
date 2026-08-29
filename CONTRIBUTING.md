# Contributing to slint

Thanks for helping make slint better. slint is the linter for Agent Skills: static-first, every finding cited, fixes computed rather than generated. This page tells you how to get the code running and what is expected of a change.

The [documentation site](https://slint.dev) carries the same information with more depth on [writing rules](https://slint.dev/contributing) — the two must not disagree.

## Repository layout

| Path | What it is |
| --- | --- |
| `packages/core` | Shared Rust library (`slint` crate): parsing, rules, engine, LLM pass, plugins |
| `apps/cli` | The `slint` binary (`slint-cli`) |
| `apps/vscode` | VS Code extension — runs the binary, never reimplements it |
| `apps/docs` | Astro documentation site, rule pages synced from the binary |
| `scripts/` | Local CI mirror and helper scripts |

JS/TS apps use pnpm workspaces + Turborepo; Rust stays on a Cargo workspace.

## Getting it running

You need a Rust toolchain (MSRV **1.95**, from `rust-version` in [Cargo.toml](Cargo.toml); CI enforces it) and [pnpm](https://pnpm.io) (CI pins 11.21.0).

```bash
git clone https://github.com/MaximeGaudin/slint
cd slint
pnpm install
cargo test --workspace   # the whole suite, including the CLI end to end
cargo run -p slint-cli -- .   # lint a directory of skills
```

To install the binary into `~/.cargo/bin`:

```bash
pnpm install:cli    # or ./scripts/build-install.sh
```

## Local checks (mirror CI)

`.github/workflows/ci.yml` runs format, clippy, tests (Linux/macOS/Windows), the MSRV check, cargo-deny, coverage, the JS lint/test jobs, and a rules-catalogue drift check. The local mirror is one script:

```bash
./scripts/check.sh          # fmt + clippy + cargo-deny + todos + pnpm lint + tests
./scripts/check.sh --no-test   # skip cargo test
./scripts/check.sh --no-js     # skip pnpm lint/test
```

Run it before every push. Pieces of it, if you need them alone:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check                                  # needs cargo-deny
node scripts/check-no-todos.mjs                   # no TODO/FIXME/XXX/HACK/STUB in comments
pnpm exec turbo run lint --filter=slint-docs --filter=slint-vscode
pnpm package:vscode                               # validate the extension manifest
./scripts/check-rules-json.sh                     # docs-site rule catalogue matches the binary
```

`cargo deny`, `cargo-llvm-cov` and `llvm-tools-preview` are CI requirements too:

```bash
rustup component add llvm-tools-preview
cargo install cargo-deny cargo-llvm-cov --locked
```

Coverage must stay at or above **92% lines** (`pnpm coverage:cli` reports it locally).

## Commit style

History follows [Conventional Commits](https://www.conventionalcommits.org): a type, an optional scope, a lowercase summary, and the issue or PR number in parentheses when there is one. Real examples from this repository:

```text
fix: atomic writes, line-ending preservation, fenced-block-aware headings, single-pass convergence (#383)
llm: apply request timeout, shared client, transport retry with backoff, reply token cap (#386)
test: cover vscode extension spawning, diagnostics, concurrency, code actions (#394)
chore: add Clippy and rustfmt to the Rust workspace
```

Common types here: `fix`, `feat`, `rules`, `llm`, `plugin`, `cli`, `vscode`, `docs`, `engine`, `frontmatter`, `suppressions`, `test`, `ci`, `chore`. A scope from that list beats a generic type; the summary says what changed, not where.

## Pull requests

Keep a PR focused on one issue or one coherent change. Before opening it:

1. `./scripts/check.sh` is green locally.
2. New behaviour is covered by tests named after the behaviour they protect (see the [docs-site contributing page](https://slint.dev/contributing) for the house test style).
3. The PR description follows the [pull request template](.github/PULL_REQUEST_TEMPLATE.md): a short summary, `Fixes #<issue>` for each issue it closes, and a test plan.
4. Rust warnings are errors (`-D warnings`), and stub markers (`todo!`, `unimplemented!`, `dbg!`, `TODO`/`FIXME` comments) are denied — finishing the work is the only way through.

Opening a PR runs CI on it; wait for green before considering it done.

## Fixing an issue

Work happens in an isolated git worktree, never directly on a shared checkout, and a fix starts with a failing regression test committed before the fix itself. The full workflow — worktree setup, test-first discipline, local checks, PR, watching CI — lives in [`.cursor/skills/fix-github-issue/SKILL.md`](.cursor/skills/fix-github-issue/SKILL.md). Follow it for any issue fix in this repository.

## Writing a rule

The short version: a rule is metadata and a function, and four things are enforced by tests rather than review — a citation, advice, a namespaced name (`area/thing`), and no model-authored fixes. The full guide, including the rule metadata shape and what does not get accepted, is on the [documentation site](https://slint.dev/contributing).

## Reporting a security issue

Please do not open a public issue for a security problem. See [SECURITY.md](SECURITY.md) for the policy and the threat model.

## Code of conduct

By participating you agree to abide by the [Contributor Covenant](CODE_OF_CONDUCT.md). Report unacceptable behaviour through a [GitHub issue](https://github.com/MaximeGaudin/slint/issues) on this repository.

## License

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE) that covers the project.
