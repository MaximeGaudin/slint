# Security Policy

slint reads third-party-authored documents (Agent Skills), executes WebAssembly
plugins, and can transmit content to third-party LLM providers. This document
is the security policy and the threat model for all three.

## Reporting a vulnerability

Report privately through [GitHub's private vulnerability reporting]
(https://github.com/MaximeGaudin/slint/security/advisories/new).

Please do not open a public issue for anything you believe is exploitable.
You will get an initial response within 7 days. Once a fix is released, the
reporter is credited in the release notes unless they prefer otherwise.

The following are **not** security vulnerabilities, and should be regular
issues instead:

- A rule producing a wrong finding (that is a quality bug).
- A plugin reporting something offensive or wrong — plugins are third-party
  code by definition; their content is not slint's.
- A model producing a wrong or unhelpful reading.

## Threat model

slint has three trust boundaries. For each: what crosses it, what slint does
about it, and what remains the user's responsibility.

### 1. The skills being linted (untrusted input)

A skill is a document someone else wrote, and slint parses it and runs
regular expressions over it.

- Parsing is deliberately forgiving: a malformed skill produces findings or
  notes, never a crash that takes the run down.
- Regexes run through the `regex` crate — linear time, no backtracking, so a
  pathological skill cannot hang the linter with a catastrophic-backtracking
  rule.
- Model rules, when explicitly requested with `--llm`, send skill content to
  a provider. See boundary 3 before doing that on content you do not control.
- A skill's own `slint-disable` comments suppress findings for that skill.
  This is a documented feature; a skill asking not to be linted is a signal a
  human should read, not a hole.

**Residual risk:** a crafted skill can spend a model's tokens (with `--llm`)
or make noise. It cannot execute code, read files beyond its own directory's
bundled files, or affect the rest of the run beyond its own report entry.

### 2. WebAssembly plugins (untrusted code)

A config can point at `.wasm` files, which slint loads and runs through
[Extism](https://extism.org) once per skill.

What the sandbox actually denies — this is enforced by the runtime setup, not
by convention:

- **No host functions** are registered. The plugin cannot call back into
  slint.
- **WASI is off.** No filesystem access, no network, no environment
  variables, no clock.
- The manifest contains only the module itself.

Failure isolation: a plugin that fails to load, crashes, refuses the wire
format, or reports findings without citations becomes a note on that skill's
report. It cannot fail the run, affect other skills, or alter findings from
built-in rules.

**Residual risk:** the sandbox constrains capability, not quality. A plugin
still chooses what findings to report, and its findings become part of the
report. Treat adding a plugin like adding a dependency: it is code running on
every lint of every skill, supplied by whoever wrote the config.

**User responsibility:** a `slint.toml` found by walking up from the linted
path — including one that shipped inside a cloned repository — is loaded
automatically, and its plugins run unless `--no-plugins` is passed. Before
linting a repository you did not write, read its `slint.toml` (and any file a
`[[plugins]]` entry points at). The [`--no-plugins`
flag](https://slint.dev/config/) exists for exactly this.

### 3. LLM providers (third-party data transmission)

The model pass is **off by default** and runs only when `--llm` is passed (or
the editor extension is configured to run it). Nothing leaves the machine
without that explicit request.

When the model pass runs, one request per skill goes to the configured
provider. What is sent:

- The skill's `name` and `description`.
- The SKILL.md **body**, truncated to `max_input_bytes` (a truncation note
  appears on the report when it happens).
- The bundled files' **paths and sizes only** — file contents are never sent.
- The rule catalogue (names, summaries, rationale, advice), so the model
  knows what it is looking for.

What is never sent: bundled file contents, the raw SKILL.md frontmatter, any
file outside the skills being linted, environment variables, or the config
file.

Where it goes is the `[llm]` block of the config: `provider` (openai,
openrouter, groq, ollama, gemini, anthropic), `model`, and optionally
`base_url`. An OpenAI-compatible `base_url` means the configured address
receives everything above — that is what makes local models and gateways
work, and it is also the setting to read carefully when the config came from
a cloned repository.

Authentication uses the environment variable named by `api_key_env`. API
keys are read from the environment at call time; they are never written to
config files, reports, logs, or the JSON envelope.

**Residual risk:** skill content arrives at the provider and is subject to
that provider's retention and training policies. For content that must not
leave the machine, use a local provider (`ollama`, or any `base_url` on
localhost).

### The configuration file itself

`slint.toml` (or `.slintrc.json`) is config, not code — it cannot execute
anything by itself. What it *controls* is where the two boundaries above sit:
which plugins run, and which provider model traffic goes to. A malicious
config from a cloned repository can therefore point plugins at a `.wasm` it
ships and `base_url` at a server it owns. The plugin sandbox caps what the
`.wasm` can do (see boundary 2); the `base_url` only ever receives what an
explicit `--llm` request sends (see boundary 3). slint never fetches remote
configs and never updates them.

## Scope of this policy

This covers the `slint` CLI, the core library, the VS Code/Cursor extension,
and the docs site, at the current `main` branch and supported releases.
Reported vulnerabilities in bundled example skills or third-party plugins
should still be reported here when they involve the boundaries above.
