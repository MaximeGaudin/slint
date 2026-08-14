# slint

![slint banner](docs/assets/readme-banner.svg)

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](Cargo.toml)

**The linter for Agent Skills.** A skill is an instruction document an agent picks from a description and then follows without being able to ask a question. Two things go wrong with them, and neither is visible in a diff: the skill is **never selected**, or it is **selected and followed badly**. `slint` is a linter for both.

It is built for terminals, CI, and editors:

- **Static first** — twenty-nine of thirty-seven rules never touch a model; no network, no tokens, no waiting
- **Cited findings** — every rule names the document its claim comes from; the citation travels into every output format
- **Computed fixes** — `--fix` normalises paths, sets executable bits, writes contents lists; a model never edits your files
- **Optional LLM pass** — eight rules that need a reader run only when you pass `--llm`, as their own pass
- **Honest about skips** — rules that need a model say so; a provider that fails says what it said
- **Plugins** — TOML rule packs or sandboxed Wasm (Extism); same citation standard as built-ins
- **Editor integration** — VS Code / Cursor extension turns findings into diagnostics on save

**Start here:** [Install](#install) · [Quick start](#quick-start) · [Configuration](#configure) · [Model pass](#the-model-half) · [Plugins](#plugins) · [Monorepo layout](#monorepo-layout) · [Docs site](apps/docs)

## Install

```bash
# From this repo
cargo install --path apps/cli

# Then ensure ~/.cargo/bin is on your PATH (or symlink to ~/.local/bin/slint)
slint --help
```

## Quick start

```bash
slint                         # lint every skill under the current directory
slint skills/photo-culling    # or one of them
slint --fix                   # apply computed fixes, then lint again
slint --format json | jq      # data on stdout, everything else on stderr
slint rules                   # the catalogue: what each rule checks and why
slint --llm                   # also run model rules (once [llm] is configured)
```

```
$ slint skills

skills/helper  1 error, 2 warnings
  SKILL.md:10:1  error    bundle/no-dangling-path  The instructions name scripts/cull.py, which is not in the bundle
                          Add the file to the bundle, or take the reference out of the instructions.
                          https://agentskills.io/specification

   SKILL.md:3:1  warning  description/says-when    The description never says when to use this
                          Append a trigger clause — "Use when …" followed by the situation.

   SKILL.md:9:9  warning  body/posix-paths         "scripts\notes.md" is a Windows path
                          Use forward slashes. Bundled paths are POSIX wherever the agent unpacks them.

3 problem(s): 1 error(s), 2 warning(s), 0 note(s) across 1 skill(s).
1 of them are computed fixes: run again with --fix.
```

Exit codes match the convention other skill linters settled on, so a CI script written for one works here: `0` clean, `1` errors, `2` warnings only, `3` slint itself failed.

| Flag | What it does |
|------|--------------|
| `--fix` | Apply every computed fix, then lint again. |
| `--format` | `stylish` (default), `json`, `github`, `compact`. |
| `--rule name=level` | Override one rule (`off`, `info`, `warn`, `error`). |
| `--max-warnings N` | Fail when there are more warnings than this. |
| `--llm` | Run the rules that need a model. Off by default. |
| `--no-plugins` | Skip plugins, whatever the config says. |
| `--quiet` | Errors only. |

## What makes it different

**Static first.** Everything answerable from the text is answered from the text. The eight rules that need a model are one request per skill, only when you ask for them, and always reported as their own pass.

**Every finding cites a source.** A linter that says "wrong" without saying who says so gets argued with. Every rule — including yours, in a plugin — names the document its claim comes from.

**Fixes are computed, never generated.** `--fix` does not call a model. It normalises path separators, sets executable bits, and writes contents lists from headings already in the file.

**It knows what it did not do.** Model rules that were skipped say so and tell you how to run them. A review that quietly did not run must never read like a review that found nothing.

## Configure

`slint init` writes a starter `slint.toml`. Every rule is already on; the file is for the ones you disagree with.

```toml
ignore = ["**/fixtures/**"]

[rules]
"description/says-when" = "error"          # raise one
"body/max-lines" = ["warn", { max = 400 }] # tune one
"bundle/unused-file" = "off"               # turn one off

[llm]
provider = "openai"        # none | openai | openrouter | groq | ollama | gemini | anthropic
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"   # the variable holding the key, never the key itself

[[plugins]]
path = "./slint-house-rules.toml"
```

`slint.config.json` and `.slintrc.json` work too. The file is found by walking up from whatever you asked slint to lint.

A document can also opt out of a rule for itself:

```markdown
<!-- slint-disable name/not-generic -->
<!-- slint-disable-next-line body/posix-paths -->
```

## The model half

The rules a regular expression cannot answer — ambiguity, terminology drift, whether a plausible request would actually route here — need a reader. slint uses [genai](https://github.com/jeremychone/rust-genai) for that, so OpenAI, Anthropic, Gemini, Ollama, OpenRouter and Groq are supported natively, and `base_url` points any of those wire formats at a gateway or a self-hosted server.

One request per skill covers all eight rules at once, and **only when you ask for it**: the model pass is off unless `--llm` is given. File contents are never sent — only names and sizes.

## Plugins

Two kinds, neither of which needs its author to know Rust.

**A rule pack** is data. `slint init-plugin` writes a starter:

```toml
[[rules]]
name = "house/no-todo"
severity = "warning"
summary = "Instructions carry no TODO markers."
rationale = "An agent follows what is written. A TODO is a note to a human that reads as an instruction to a machine."
advice = "Finish the step, or take the line out until it is ready."
pattern = "TODO|FIXME|XXX"
target = "body"                     # body | description | name | files
message = "\"{match}\" is a note to a person, in a document a machine follows."
reference = { title = "House style", url = "https://example.com/style" }
```

**A WebAssembly plugin** is code, run through [Extism](https://extism.org). Point the config at a `.wasm` file; slint calls its exported `lint` function with the parsed skill as JSON and reads messages back. It runs sandboxed — no filesystem, no network.

Both are held to the same standard as the built-in catalogue: a namespaced rule name, and a citation.

## Editors

[`apps/vscode`](apps/vscode) is a VS Code / Cursor extension that runs slint on save (and optionally the model pass), merges static and model diagnostics, and shows citations on hover.

```bash
pnpm install
pnpm build:vscode
# package + install the .vsix into Cursor / VS Code
```

## Monorepo layout

| Path | What it is |
|------|------------|
| [`apps/cli`](apps/cli) | `slint` binary (Rust) |
| [`apps/vscode`](apps/vscode) | VS Code / Cursor extension |
| [`apps/docs`](apps/docs) | Astro documentation site (rule catalogue synced from the binary) |
| [`packages/core`](packages/core) | Shared Rust library (`slint`) used by the CLI |

JS/TS apps use **pnpm workspaces** + **Turborepo**. Rust stays on a **Cargo workspace**.

## Development

```bash
pnpm install

cargo test --workspace    # or: pnpm test:cli
cargo build --release --package slint-cli

pnpm lint:cli             # rustfmt check + clippy (-D warnings)
pnpm format:cli           # rustfmt --all
pnpm build                # Turbo: CLI + vscode + docs
pnpm build:vscode
pnpm sync:docs            # refresh apps/docs/src/data/rules.json from the CLI
pnpm dev:docs             # Astro docs at http://localhost:4321
```

`slint rules` prints the catalogue. `slint rules --json` feeds the docs site, so the site and the binary never disagree about what a rule does.

More detail: [`apps/docs/README.md`](apps/docs/README.md).

## License

Copyright (C) 2026 Maxime Gaudin

MIT — see [LICENSE](LICENSE).
