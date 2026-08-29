# slint — Agent Skills linter for VS Code

Lints `SKILL.md` files as you save them, with the source of every rule one hover away.

This is the official editor integration for [slint](https://github.com/MaximeGaudin/slint), the linter for Agent Skills. Findings appear as diagnostics in the Problems panel and as squiggles in the editor, each carrying a citation that explains the rule.

## Features

- **Lint on save** — every time you save a `SKILL.md`, the extension runs `slint` on it and turns the findings into diagnostics. Three modes: do nothing, static rules only (fast, free), or static rules followed by the model pass.
- **Lint while typing** — optional debounced static linting as you type. Never runs the model.
- **Citations on hover** — hover any diagnostic to read the exact passage of the rule's source document that motivates it.
- **Quick fixes** — apply the computed fix for a finding, or ignore a rule with `<!-- slint-disable -->` / `<!-- slint-disable-next-line -->` comments straight from the light-bulb menu.
- **Workspace lint** — lint every skill in the workspace in one command, with the model pass available for the current file or the whole workspace.

## Requirements

The extension shells out to the `slint` binary — install it first:

```bash
brew install MaximeGaudin/tap/slint
# or
cargo install slint-cli
```

Check that `slint --version` works in a terminal. If the binary is not on your `PATH`, point `slint.path` at it.

## Getting started

1. Install the extension and the `slint` binary.
2. Open a workspace containing `SKILL.md` files (the extension activates on markdown files and any workspace that contains a `SKILL.md`).
3. Save a skill — findings show up as diagnostics.

## Commands

| Command                                | What it does                                  |
| -------------------------------------- | --------------------------------------------- |
| `slint: Lint every skill in the workspace` | Runs static rules across the workspace.   |
| `slint: Lint with model (current skill)`   | Static rules, then the model pass, for the open file. |
| `slint: Lint workspace with model`         | Static rules, then the model pass, everywhere. |
| `slint: Apply the computed fixes to this skill` | Writes the fixes the CLI computed for the open file. |
| `slint: Show Output`                       | Opens the slint output channel.           |

## Extension settings

| Setting             | Default    | What it does                                                                                                                                                             |
| ------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `slint.path`        | `slint`    | The slint binary. An absolute path, or a name found on `PATH`.                                                                                                           |
| `slint.onSave`      | `no-llm`   | What to run when you save a `SKILL.md`: `nothing`, `no-llm` (static rules only), or `llm` (static rules, then the model pass).                                            |
| `slint.onType`      | `false`    | Run static rules while typing a `SKILL.md` (debounced). Never runs the model.                                                                                            |
| `slint.llm.provider`| *(config)* | Model provider for the LLM rules (`openai`, `openrouter`, `groq`, `ollama`, `gemini`, `anthropic`). Overrides `[llm].provider` in `slint.toml`.                          |
| `slint.llm.model`   | *(config)* | Model id the provider understands (e.g. `llama-3.1-8b-instant` on Groq). Overrides `[llm].model` in `slint.toml`.                                                        |
| `slint.llm.apiKey`  | *(empty)*  | API key for the provider. Prefer User settings (not workspace) so it is not committed. Passed to slint via a process env var, never as a CLI flag.                        |
| `slint.arguments`   | `[]`       | Extra arguments passed to slint, such as `--rule name=off`.                                                                                                              |

## Remote development

The extension is pinned to the **workspace** extension host, so it runs where your files and the `slint` binary live. It works in Remote-SSH, WSL, dev containers, and Codespaces as long as `slint` is installed on the remote host (or `slint.path` points at it there).

## Compatibility

Tested against VS Code `^1.90.0`. The extension only uses long-stable APIs (diagnostics, status bar, code actions, workspace edits), so VS Code forks such as Cursor are expected to work — sideload the `.vsix` with **Extensions: Install from VSIX…** — but they are not tested in CI.

## Development

```bash
pnpm install
pnpm build:vscode     # compile
pnpm lint:vscode      # biome + tsc + comment checks
pnpm --filter slint-vscode test    # unit tests (node:test)
pnpm package:vscode   # produce a .vsix
```

The `.vsix` lands in `apps/vscode/`. Install it with **Extensions: Install from VSIX…**.

## License

[MIT](LICENSE)
