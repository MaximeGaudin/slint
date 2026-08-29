# slint

![slint - the linter for Agent Skills](docs/assets/readme-banner.svg)

[License: MIT](LICENSE)
[Rust](Cargo.toml)

**The linter for Agent Skills.** A skill is an instruction document an agent picks from a description and then follows without being able to ask a question. Two things go wrong with them, and neither is visible in a diff: the skill is **never selected**, or it is **selected and followed badly**. `slint` is a linter for both.

It is built for terminals, CI, and editors:

- **Static first** — thirty-four of forty-two rules never touch a model; no network, no tokens, no waiting
- **Cited findings** — every rule names the document its claim comes from; the citation travels into every output format
- **Computed fixes** — `--fix` normalises paths, sets executable bits, writes contents lists; a model never edits your files
- **Optional LLM pass** — eight rules that need a reader run only when you pass `--llm`, as their own pass
- **Honest about skips** — rules that need a model say so; a provider that fails says what it said
- **Plugins** — TOML rule packs or sandboxed Wasm (Extism); same citation standard as built-ins
- **Editor integration** — VS Code extension turns findings into diagnostics on save

**Start here:** [Install](#install) · [Quick start](#quick-start) · [Configuration](#configure) · [Model pass](#the-model-half) · [Plugins](#plugins) · [Monorepo layout](#monorepo-layout) · [Docs site](apps/docs)

## Install

```bash
# From this repo (installs `slint` into ~/.cargo/bin)
pnpm install:cli
# or: ./scripts/build-install.sh

# Ensure ~/.cargo/bin is on your PATH
slint --help
```

Release binaries (latest release: [v0.2.0](https://github.com/MaximeGaudin/slint/releases/tag/0.2.0); macOS (Apple Silicon, Intel), Linux (x86_64, arm64), and Windows (`slint-windows-amd64.zip`) assets are published for each release):

```bash
# macOS (Apple Silicon)
curl -fsSL https://github.com/MaximeGaudin/slint/releases/latest/download/slint-darwin-arm64.tar.gz | sudo tar xz -C /usr/local/bin

# Linux (x86_64)
curl -fsSL https://github.com/MaximeGaudin/slint/releases/latest/download/slint-linux-amd64.tar.gz | sudo tar xz -C /usr/local/bin
```



## Quick start

```bash
slint                         # lint every skill under the current directory
slint skills/photo-culling    # or one of them
slint --fix                   # apply computed fixes, then lint again
slint --format json | jq      # data on stdout, everything else on stderr
slint rules                   # the catalogue: what each rule checks and why
slint --llm                   # also run model rules (once [llm] is configured)
slint --print-config          # the resolved config, file and flags together
slint --explain body/max-lines  # one rule, not the whole catalogue
slint completions zsh         # shell completions (bash, zsh, fish, powershell)
```
```
$ slint skills

skills/photo-culling  1 error, 2 warnings
   SKILL.md:10:1  error    bundle/no-dangling-path  The instructions name scripts/cull.py, which is not in the bundle
                           What to do: Either add the missing file under the skill directory, or remove that path from the instructions.
                           https://agentskills.io/specification

    SKILL.md:3:1  warning  description/says-when    The description never says when to use this
                           What to do: Add a clause like "Use when …" with the situation, using phrasing a real user request would use.
                           https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices

  SKILL.md:11:10  warning  body/posix-paths         "scripts\notes.md" is a Windows path
                           What to do: Replace \ with / in every bundled path.
                           https://agentskills.io/specification

  note  Skipped 8 model rules (not requested). Pass --llm to run them.

3 problem(s): 1 error(s), 2 warning(s), 0 note(s) across 1 skill(s).
1 of them are computed fixes: run again with --fix.
```

Exit codes match the convention other skill linters settled on, so a CI script written for one works here: `0` clean, `1` errors, `2` warnings only, `3` slint itself failed, `4` nothing was linted (no `SKILL.md` was found under the given path — check the path before trusting a green run). Running `slint init` when a config already exists is an idempotent no-op and exits `0`; nothing is overwritten.


| Flag                    | What it does                                                       |
| ----------------------- | ------------------------------------------------------------------ |
| `--fix`                 | Apply every computed fix, then lint again.                         |
| `--format`              | `stylish` (default), `json`, `github`, `sarif`, `compact`.         |
| `--stdin`               | Lint the document on stdin (static rules only).                    |
| `--stdin-filename PATH` | The name to report for the stdin document.                         |
| `--print-config`        | Print the resolved config as JSON and stop.                        |
| `--explain RULE`        | Print one rule's catalogue entry and stop.                         |
| `--config CONFIG`       | Use this config rather than looking for one.                       |
| `--ignore-path F`       | Extra ignore patterns, one glob per line.                          |
| `--no-ignore`           | Lint everything, even what the config ignores.                     |
| `--rule name=level`     | Override one rule (`off`, `info`, `warn`, `error`).                |
| `--max-warnings N`      | Fail when there are more warnings than this.                       |
| `--llm`                 | Run the rules that need a model. Off by default.                   |
| `--llm-provider NAME`   | Override `[llm].provider` from the config.                         |
| `--llm-model ID`        | Override `[llm].model` from the config.                            |
| `--llm-base-url URL`    | Override `[llm].base_url` from the config.                         |
| `--llm-api-key-env VAR` | Override `[llm].api_key_env` from the config.                      |
| `--no-plugins`          | Skip plugins, whatever the config says.                            |
| `--quiet`               | Errors only.                                                       |
| `-v` / `--verbose`      | Say which config and how many plugins, on stderr.                  |
| `--no-color`            | Never colour the output.                                           |




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

`slint.config.json` and `.slintrc.json` work too. The file is found by walking up from whatever you asked slint to lint. When no project config exists anywhere up the tree, a user-global one is used: `$XDG_CONFIG_HOME/slint/config.toml`, or `~/.config/slint/config.toml` — the place for a personal provider or a severity you always disagree with. A project config always wins.

For editor autocomplete over the JSON shapes, point `$schema` at the published schema:

```json
{
  "$schema": "https://slint.dev/schemas/slint-config.json",
  "rules": { "description/says-when": "error" }
}
```

A document can also opt out of a rule for itself:

```markdown
<!-- slint-disable name/not-generic -->
<!-- slint-disable-next-line body/posix-paths -->
```



## The model half

The rules a regular expression cannot answer — ambiguity, terminology drift, whether a plausible request would actually route here — need a reader. slint uses [genai](https://github.com/jeremychone/rust-genai) for that, so OpenAI, Anthropic, Gemini, Ollama, OpenRouter and Groq are supported natively, and `base_url` points any of those wire formats at a gateway or a self-hosted server.

One request per skill covers all eight rules at once, and **only when you ask for it**: the model pass is off unless `--llm` is given. Bundled file contents are never sent — only their names and sizes. The SKILL.md body itself is sent (truncated to `max_input_bytes`, 64 KB by default), since it is the text the model rules review, and it is framed to the model as untrusted data to review rather than instructions to follow.

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

`[apps/vscode](apps/vscode)` is a VS Code extension that runs slint on save (and optionally the model pass), merges static and model diagnostics, and shows citations on hover. It targets VS Code 1.90+; forks such as Cursor are expected to work (the extension sticks to long-stable APIs) but are not CI-verified.

```bash
pnpm install
pnpm build:vscode
# package + install the .vsix into VS Code
```



## Monorepo layout


| Path                             | What it is                                                       |
| -------------------------------- | ---------------------------------------------------------------- |
| `[apps/cli](apps/cli)`           | `slint` binary (Rust)                                            |
| `[apps/vscode](apps/vscode)`     | VS Code extension                                                |
| `[apps/docs](apps/docs)`         | Astro documentation site (rule catalogue synced from the binary) |
| `[packages/core](packages/core)` | Shared Rust library (`slint`) used by the CLI                    |


JS/TS apps use **pnpm workspaces** + **Turborepo**. Rust stays on a **Cargo workspace**.

## Development

```bash
pnpm install

./scripts/check.sh        # fmt + clippy + cargo-deny + todos + pnpm lint + tests (mirrors CI)
./scripts/build-install.sh
pnpm install:cli          # cargo install --path apps/cli → ~/.cargo/bin/slint
pnpm coverage:cli         # line/region/function coverage (needs cargo-llvm-cov)
pnpm coverage:cli:html    # HTML report under coverage/html
pnpm coverage:cli:lcov    # LCOV at coverage/lcov.info
cargo build --release --package slint-cli

pnpm lint                 # CLI + vscode + docs (+ no TODO/FIXME/stubs)
pnpm lint:cli             # rustfmt + clippy + cargo-deny (-D warnings, deny todo!/unimplemented!/dbg!)
pnpm lint:vscode          # biome + tsc --noEmit
pnpm lint:docs            # biome + astro check
pnpm format               # rustfmt + biome --write (cli/vscode/docs)
pnpm format:cli           # rustfmt --all
pnpm format:vscode        # biome --write for the extension
pnpm format:docs          # biome --write for the docs site
pnpm build                # Turbo: CLI + vscode + docs
pnpm build:vscode
pnpm sync:docs            # refresh apps/docs/src/data/rules.json from the CLI
pnpm dev:docs             # Astro docs at http://localhost:4321
```

Coverage needs `[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)` and the `llvm-tools-preview` rustup component (`rustup component add llvm-tools-preview && cargo install cargo-llvm-cov --locked`).

`slint rules` prints the catalogue. `slint rules --json` feeds the docs site, and CI fails when `apps/docs/src/data/rules.json` no longer matches the binary's catalogue, so the site and the binary never disagree about what a rule does.

More detail: `[apps/docs/README.md](apps/docs/README.md)`.

## License

Copyright (C) 2026 Maxime Gaudin

MIT — see [LICENSE](LICENSE).