# Security Policy

slint is a developer tool: a linter that reads skills where they live and, when explicitly asked, sends a bounded excerpt of them to a model provider you configure. This page describes how to report a vulnerability, what slint's trust boundaries are, and — precisely — what data can leave your machine and when.

## Supported versions

Only the latest release is supported with security fixes. There is no LTS: if you are on an older version, update first and re-check whether the problem still exists.

## Reporting a vulnerability

**Do not open a public issue for a security report.**

Report privately through [GitHub Security Advisories](https://github.com/MaximeGaudin/slint/security/advisories/new) on this repository. That keeps the report, the discussion, and the fix coordinated in one private place.

Please include what you can of:

- The version or commit you tested (`slint --version`, or the commit hash)
- A minimal reproduction (config, plugin, skill, command line)
- Which trust boundary you believe was crossed (see the threat model below)
- Your assessment of impact

You will get an acknowledgement, and updates as the report is triaged. Credit in the advisory is yours to accept or decline; say which when you report.

## Threat model

What follows is the model the code is actually built against. Each claim here corresponds to a specific place in the source, cited so it can be checked rather than trusted.

### What slint processes

- **Skill content** — `SKILL.md` files and any files bundled with them — is **untrusted input**. slint parses it, lints it, and reports on it. It never executes a bundled file, never follows a path out of the bundle, and never treats skill text as instructions to slint itself. Model-authored finding text is stripped of control characters before it is shown (see `strip_control` in `packages/core/src/diagnostics.rs`).
- **Config files** (`slint.toml`, `slint.config.json`, `.slintrc.json`) are trusted configuration: whoever can write one can already run arbitrary code on your machine by other means. They decide which plugins load and where model requests go, so treat a config from an untrusted source with the same suspicion as a plugin.
- **Suppression comments** (`<!-- slint-disable … -->`) are honoured only for the rules and scopes they name; a skill cannot disable slint's own guardrails for other documents.

### Wasm plugins (Extism sandbox)

A `.wasm` plugin is third-party-authored code, executed through [Extism](https://extism.org). The boundary:

- The plugin is instantiated with **no filesystem access, no network access, and no environment variables** — the manifest in `packages/core/src/plugin.rs` (`extism::Manifest::new`) declares no host functions and no WASI. A plugin that misbehaves takes only itself down.
- The host hands the plugin one thing: the parsed skill as JSON (`lint` export, one call). The plugin answers with messages, which are sanitised before display, and rule ids that may not shadow built-in rules.
- What a plugin **can** do: read the skill content it is given, and be slow or crash — slint reports the failure as a per-plugin error instead of aborting the run.
- What remains **your** decision: whether to load the plugin at all. The sandbox bounds what a plugin can do while it runs; it does not make a plugin trustworthy. Only add `.wasm` plugins from sources you would trust with the content of the skills you lint, because the plugin sees exactly that content. Rule packs (`.toml`/`.json`) are inert data — they define regex rules but execute nothing.

Known limits of the boundary, stated plainly: Extism's Wasm sandbox is a well-tested but third-party runtime; a sandbox escape in Extism/wasmtime itself would not be slint's bug to fix but would be in scope for this policy's reporting path if it affects slint's usage. Plugin behaviour is not otherwise audited by slint, and plugins are not signed.

### The model pass (`--llm`)

The model pass is **off unless you pass `--llm`** (or the equivalent editor setting). When it runs, one request per skill goes to the provider configured in `[llm]` (`provider`, `model`, optional `base_url`) — OpenAI, Anthropic, Gemini, OpenRouter, Groq, or a self-hosted Ollama/gateway.

**What is sent:**

- The skill's **name** and **description**
- **Bundled file names and sizes** — never their contents
- The **`SKILL.md` body itself**, truncated to `max_input_bytes` (64 KB by default, configurable) — it is the text the model rules review. It is framed to the model as untrusted data between randomised boundary markers, with explicit instruction-hierarchy framing: the model is told never to follow directives found inside it (see `user_prompt` in `packages/core/src/llm/review.rs`).

**What is never sent:** bundled file contents, your `slint.toml`, environment variables, or anything from your filesystem beyond the skills you asked slint to lint.

**Credentials:** API keys are read from the environment variable named by `api_key_env` — the config file holds the *name*, never the key. There is no key material in config files, and slint does not write logs containing request payloads; failures are reported with the provider's own error text.

**Endpoints:** requests go only to the provider endpoint for the configured `provider`, or to `base_url` if you set one. There is no telemetry, no analytics, and no other network traffic. The eight static passes never touch a network at all.

**Residual risk:** the SKILL.md body does leave your machine when `--llm` is on — that is inherent to a model review. If a skill's body contains secrets, a model provider will see them. Choose providers accordingly (a local Ollama keeps everything on-machine), and remember the model is reviewing untrusted text: slint frames it as data, but prompt-injection defences are probabilistic, not guarantees. Model-authored findings are also capped (`max_tokens`) and sanitised before display.

## What is in scope

- Crossings of the boundaries above: plugin reaching filesystem/network/environment, or skill content being treated as instructions
- Data leaving the machine beyond what is described here (any telemetry, unexpected endpoints, payloads larger than documented)
- Secrets or credentials ending up in logs, errors, or output files
- Injection through the GitHub-format, JSON, SARIF, or compact reporters
- The CLI itself being crashed or subverted by malicious skill content in a way that exceeds per-skill failure reporting

## What is out of scope

- Bugs in third-party runtimes reported to slint first instead of upstream (report Extism/wasmtime issues there too, in parallel)
- A plugin doing something you configured it to do
- The model pass sending skill content when you explicitly ran `--llm` — that is the documented feature, not a leak
- Social engineering of a human operator

## Disclosure

Reports are fixed and disclosed through GitHub Security Advisories, with credit to the reporter if desired. Fixes land on `main` and ship in the next release; supported-versions policy above applies.
