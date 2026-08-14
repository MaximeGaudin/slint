# slint

The linter for Agent Skills.

A skill is an instruction document an agent picks from a description and then follows without being
able to ask a question. Two things go wrong with them, and neither is visible in a diff: the skill is
**never selected**, or it is **selected and followed badly**. slint is a linter for both.

```
$ slint skills

skills/helper  1 error, 2 warnings
  SKILL.md:10:1  error    bundle/no-dangling-path  The instructions name scripts/cull.py, which is not in the bundle
                          Add the file to the bundle, or take the reference out of the instructions.
                          https://agentskills.io/specification

   SKILL.md:3:1  warning  description/says-when    The description never says when to use this
                          Append a trigger clause — "Use when …" followed by the situation.
                          https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices

   SKILL.md:9:9  warning  body/posix-paths         "scripts\notes.md" is a Windows path
                          Use forward slashes. Bundled paths are POSIX wherever the agent unpacks them.

3 problem(s): 1 error(s), 2 warning(s), 0 note(s) across 1 skill(s).
1 of them are computed fixes: run again with --fix.
```

## What makes it different

**Static first.** Everything answerable from the text is answered from the text: no network, no
tokens, no waiting. Twenty-nine of the thirty-seven rules never touch a model, and the eight that do
are one request per skill, only when you ask for them, and always reported as their own pass.

**Every finding cites a source.** A linter that says "wrong" without saying who says so gets argued
with. Every rule — including yours, in a plugin — names the document its claim comes from, and the
citation travels with the finding into every output format.

**Fixes are computed, never generated.** `--fix` normalises path separators, sets executable bits and
writes contents lists from headings already in the file. A model never edits your files.

**It knows what it did not do.** The rules that need a model say so and tell you how to run them; a
provider that fails says what it said and what to check. A review that quietly did not run must
never read like a review that found nothing.

## Install

```
cargo install --path apps/cli
```

## Use

```
slint                         # lint every skill under the current directory
slint skills/photo-culling    # or one of them
slint --fix                   # apply the computed fixes, then lint again
slint --format json | jq      # data on stdout, everything else on stderr
slint rules                   # the catalogue, with what each rule checks and why
slint --llm                   # also run the rules that need a model, once [llm] is configured
```

Exit codes are the convention the other skill linters settled on, so a CI script written for one
works here: `0` clean, `1` errors, `2` warnings only, `3` slint itself failed.

| Flag | What it does |
|---|---|
| `--fix` | Apply every computed fix, then lint again so the report describes the files as they now are. |
| `--format` | `stylish` (default), `json`, `github`, `compact`. |
| `--rule name=level` | Override one rule, repeatable. `off`, `info`, `warn`, `error`. |
| `--max-warnings N` | Fail the run when there are more warnings than this. |
| `--llm` | Run the rules that need a model. Off by default: a linter must not spend money uninvited. |
| `--no-plugins` | Skip plugins, whatever the config says. |
| `--quiet` | Errors only. |

## Configure

`slint init` writes a starter `slint.toml`. Every rule is already on; the file is for the ones you
disagree with.

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

`slint.config.json` and `.slintrc.json` work too, with the same fields. The file is found by walking
up from whatever you asked slint to lint, so running it in a subdirectory behaves the way you expect.

A document can also opt out of a rule for itself:

```markdown
<!-- slint-disable name/not-generic -->
<!-- slint-disable-next-line body/posix-paths -->
```

## The model half

The rules a regular expression cannot answer — ambiguity, terminology drift, whether a plausible
request would actually route here — need a reader. slint uses [genai](https://github.com/jeremychone/rust-genai)
for that, so OpenAI, Anthropic, Gemini, Ollama, OpenRouter and Groq are all supported natively, and
`base_url` points any of those wire formats at a gateway or a self-hosted server.

One request per skill covers all eight rules at once, and **only when you ask for it**: the model
pass is off unless `--llm` is given, so no run of slint ever reaches a paid provider by surprise.
File contents are never sent — only names and sizes.

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

**A WebAssembly plugin** is code, run through [Extism](https://extism.org). Point the config at a
`.wasm` file; slint calls its exported `lint` function with the parsed skill as JSON and reads
messages back. It runs sandboxed — no filesystem, no network — so adding one is not a supply-chain
decision.

Both are held to the same standard as the built-in catalogue: a namespaced rule name, and a citation.
A plugin reporting without either is refused rather than trusted.

## Editors

`editors/vscode` is a VS Code extension that runs slint on save and turns its findings into
diagnostics, with the citation on the hover.

## Rules

`slint rules` prints the catalogue. `slint rules --json` is what the documentation site is built
from, so the site and the binary can never disagree about what a rule does.
