//! Rules other people write.
//!
//! Two kinds, and neither can crash the linter or needs its author to know any Rust.
//!
//! **A rule pack** is data: a TOML or JSON file listing patterns, what they mean and where the claim
//! comes from. It covers the common case — a house style, a forbidden word, a required section —
//! without a build step.
//!
//! **A WebAssembly plugin** is code, run through [`extism`]. It is handed the parsed skill as JSON
//! and answers with messages, so a plugin can be written in whatever language its author already
//! has — and it runs sandboxed, with no filesystem and no network, so a plugin that misbehaves
//! takes only itself down.
//!
//! Both are subject to the same rule as the built-in catalogue: no citation, no rule.

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{Config, PluginRef};
use crate::diagnostics::{Location, Message, Reference, Severity, Source, strip_control};
use crate::skill::Skill;

/// Which part of a skill a pack rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    #[default]
    Body,
    Description,
    Name,
    /// Every bundled text file.
    Files,
}

/// One rule from a pack.
#[derive(Debug, Clone, Deserialize)]
pub struct PackRule {
    /// Namespaced, like every other rule: `house/no-todo`.
    pub name: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub summary: String,
    pub rationale: String,
    pub advice: String,
    pub reference: Reference,
    /// A regular expression. A match is a finding.
    pub pattern: String,
    #[serde(default)]
    pub target: Target,
    /// The message, with `{match}` replaced by what was found.
    #[serde(default)]
    pub message: Option<String>,
    /// Invert the rule: a finding when the pattern is *not* found anywhere in the target.
    #[serde(default)]
    pub required: bool,
}

fn default_severity() -> String {
    "warning".to_string()
}

#[derive(Debug, Deserialize)]
struct Pack {
    #[serde(default)]
    rules: Vec<PackRule>,
}

/// A loaded plugin.
#[derive(Debug)]
pub enum Plugin {
    Pack {
        path: PathBuf,
        rules: Vec<CompiledRule>,
    },
    /// A WebAssembly module, run through extism's sandbox.
    Wasm { path: PathBuf },
}

#[derive(Debug)]
pub struct CompiledRule {
    pub rule: PackRule,
    pub pattern: Regex,
    pub severity: Severity,
}

impl Plugin {
    pub fn describe(&self) -> String {
        match self {
            Plugin::Pack { path, rules } => {
                format!("{} ({} rules)", path.display(), rules.len())
            }
            Plugin::Wasm { path } => format!("{} (wasm)", path.display()),
        }
    }

    pub fn rule_names(&self) -> Vec<String> {
        match self {
            Plugin::Pack { rules, .. } => rules.iter().map(|one| one.rule.name.clone()).collect(),
            // A module declares its rules by reporting them; there is nothing to list up front.
            Plugin::Wasm { .. } => Vec::new(),
        }
    }
}

/// Loads every plugin a config names, resolving paths against the config file itself.
pub fn load_all(config: &Config) -> Result<Vec<Plugin>> {
    let base = config
        .source
        .as_ref()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    config
        .plugins
        .iter()
        .map(|reference| load(reference, &base))
        .collect()
}

pub fn load(reference: &PluginRef, base: &Path) -> Result<Plugin> {
    let path = base.join(&reference.path);

    // The extension decides which kind it is, so a config never has to say twice.
    if path.extension().and_then(|one| one.to_str()) == Some("wasm") {
        if !path.is_file() {
            bail!("plugin {} does not exist", path.display());
        }
        return Ok(Plugin::Wasm { path });
    }

    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading plugin {}", path.display()))?;

    let pack: Pack = if path.extension().and_then(|one| one.to_str()) == Some("json") {
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    };

    let rules = compile(pack.rules, &path)?;

    Ok(Plugin::Pack { path, rules })
}

fn compile(rules: Vec<PackRule>, path: &Path) -> Result<Vec<CompiledRule>> {
    let mut compiled = Vec::new();

    for rule in rules {
        // Everything a pack author wrote is untrusted text a reporter will print, and the rule's
        // own name becomes a config key — so the control characters come out before any of it is
        // kept.
        let mut rule = rule;
        rule.name = strip_control(&rule.name);
        rule.summary = strip_control(&rule.summary);
        rule.advice = strip_control(&rule.advice);
        rule.message = rule.message.as_deref().map(strip_control);
        rule.reference.title = strip_control(&rule.reference.title);
        rule.reference.url = strip_control(&rule.reference.url);

        if !rule.name.contains('/') {
            bail!(
                "{}: rule \"{}\" is not namespaced — use something like house/{}",
                path.display(),
                rule.name,
                rule.name
            );
        }

        if crate::rules::meta_for(&rule.name).is_some() {
            bail!(
                "{}: rule \"{}\" collides with a built-in rule of the same name, and one config \
                 switch would then control both — namespace it under your own house",
                path.display(),
                rule.name
            );
        }

        if !rule.reference.url.starts_with("https://") && !rule.reference.url.starts_with("http://")
        {
            bail!(
                "{}: rule \"{}\" has no citation. Every finding has to say where its claim comes from.",
                path.display(),
                rule.name
            );
        }

        let severity = Severity::parse(&rule.severity).with_context(|| {
            format!(
                "{}: \"{}\" is not a severity",
                path.display(),
                rule.severity
            )
        })?;

        let pattern = Regex::new(&rule.pattern).with_context(|| {
            format!(
                "{}: rule \"{}\" has a pattern that does not compile",
                path.display(),
                rule.name
            )
        })?;

        compiled.push(CompiledRule {
            rule,
            pattern,
            severity,
        });
    }

    Ok(compiled)
}

/// Runs every plugin against one skill.
pub fn run(plugins: &[Plugin], skill: &Skill, config: &Config) -> (Vec<Message>, Vec<String>) {
    let mut messages = Vec::new();
    let mut notes = Vec::new();

    for plugin in plugins {
        match plugin {
            Plugin::Pack { rules, .. } => {
                for compiled in rules {
                    messages.extend(run_pack_rule(compiled, skill, config));
                }
            }
            Plugin::Wasm { path } => match run_wasm(path, skill, config) {
                Ok(found) => messages.extend(found),
                Err(failure) => notes.push(format!(
                    "The plugin {} did not run, so its rules did not either: {failure}",
                    path.display()
                )),
            },
        }
    }

    (messages, notes)
}

fn run_pack_rule(compiled: &CompiledRule, skill: &Skill, config: &Config) -> Vec<Message> {
    let Some(severity) = config.severity_for(&compiled.rule.name, compiled.severity) else {
        return Vec::new();
    };

    let build = |text: &str, file: String, line: usize, found: &str| Message {
        rule: compiled.rule.name.clone(),
        severity,
        message: strip_control(
            &compiled
                .rule
                .message
                .clone()
                .unwrap_or_else(|| compiled.rule.summary.clone())
                .replace("{match}", found)
                .replace("{name}", &skill.name)
                .replace("{text}", text),
        ),
        advice: compiled.rule.advice.clone(),
        location: Location::at(line, 1),
        source: Source::Plugin,
        file,
        fix: None,
        reference: compiled.rule.reference.clone(),
        confidence: 1.0,
    };

    let mut messages = Vec::new();

    match compiled.rule.target {
        Target::Name => {
            let matched = compiled.pattern.find(&skill.name);
            report_scalar(
                &compiled.rule,
                matched.map(|one| one.as_str().to_string()),
                &mut messages,
                || {
                    build(
                        &skill.name,
                        skill.document.clone(),
                        skill.frontmatter_line("name"),
                        matched.map(|one| one.as_str()).unwrap_or_default(),
                    )
                },
            );
        }
        Target::Description => {
            let matched = compiled.pattern.find(&skill.description);
            report_scalar(
                &compiled.rule,
                matched.map(|one| one.as_str().to_string()),
                &mut messages,
                || {
                    build(
                        &skill.description,
                        skill.document.clone(),
                        skill.frontmatter_line("description"),
                        matched.map(|one| one.as_str()).unwrap_or_default(),
                    )
                },
            );
        }
        Target::Body => {
            if compiled.rule.required {
                if !compiled.pattern.is_match(&skill.body) {
                    messages.push(build(&skill.body, skill.document.clone(), 1, ""));
                }
            } else {
                for (index, line) in skill.body.lines().enumerate() {
                    if let Some(found) = compiled.pattern.find(line) {
                        messages.push(build(
                            line,
                            skill.document.clone(),
                            skill.document_line(index + 1),
                            found.as_str(),
                        ));
                    }
                }
            }
        }
        Target::Files => {
            for file in &skill.files {
                let Some(text) = &file.text else { continue };
                let path = format!("{}/{}", skill.directory.display(), file.path);

                if compiled.rule.required {
                    if !compiled.pattern.is_match(text) {
                        messages.push(build(text, path, 1, ""));
                    }
                    continue;
                }

                for (index, line) in text.lines().enumerate() {
                    if let Some(found) = compiled.pattern.find(line) {
                        messages.push(build(line, path.clone(), index + 1, found.as_str()));
                    }
                }
            }
        }
    }

    messages
}

/// A rule over a single value fires once, and an inverted one fires when nothing matched.
fn report_scalar(
    rule: &PackRule,
    matched: Option<String>,
    messages: &mut Vec<Message>,
    build: impl Fn() -> Message,
) {
    let fired = if rule.required {
        matched.is_none()
    } else {
        matched.is_some()
    };

    if fired {
        messages.push(build());
    }
}

/// What a plugin is sent, and what it answers with.
#[derive(Debug, Serialize)]
struct PluginInput<'a> {
    /// The wire format's version, so a plugin can refuse a shape it does not know.
    version: u32,
    skill: &'a Skill,
}

#[derive(Debug, Deserialize)]
struct PluginOutput {
    #[serde(default)]
    messages: Vec<PluginMessage>,
}

#[derive(Debug, Deserialize)]
struct PluginMessage {
    rule: String,
    #[serde(default = "default_severity")]
    severity: String,
    message: String,
    #[serde(default)]
    advice: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    file: Option<String>,
    reference: Reference,
}

/// The function a plugin exports. One entry point, so a plugin is a function of a skill.
pub const PLUGIN_EXPORT: &str = "lint";

fn run_wasm(path: &Path, skill: &Skill, config: &Config) -> Result<Vec<Message>> {
    let input = serde_json::to_string(&PluginInput { version: 1, skill })?;

    // No filesystem, no network, no environment: a rule about a document does not need any of
    // them, and a plugin that cannot reach them is a plugin nobody has to audit before adding.
    let manifest = extism::Manifest::new([extism::Wasm::file(path)]);
    let mut plugin = extism::Plugin::new(&manifest, [], false)
        .with_context(|| format!("loading {}", path.display()))?;

    let answer = plugin
        .call::<&str, &str>(PLUGIN_EXPORT, &input)
        .with_context(|| format!("calling {PLUGIN_EXPORT}"))?;

    let parsed: PluginOutput =
        serde_json::from_str(answer).context("its answer was not the JSON this expects")?;

    into_messages(parsed, skill, config)
}

/// Validates what a plugin said, and turns it into findings.
///
/// The same rules the built-in catalogue is held to: a namespaced name that is not already taken
/// by a built-in, and a citation. A plugin that skips either is reporting an opinion nobody can
/// check, which is the thing this tool exists to prevent.
fn into_messages(parsed: PluginOutput, skill: &Skill, config: &Config) -> Result<Vec<Message>> {
    let mut messages = Vec::new();

    for message in parsed.messages {
        // The same treatment the pack loader gives its own rules: a namespaced name that cannot
        // shadow the built-in catalogue, and freeform strings with the control characters taken
        // out — a reporter prints all of it, and none of it is trusted.
        let rule = strip_control(&message.rule);

        if !rule.contains('/') {
            bail!("it reported \"{rule}\", which is not a namespaced rule name");
        }

        if crate::rules::meta_for(&rule).is_some() {
            bail!(
                "it reported \"{rule}\", which is a built-in rule id, and one config switch would \
                 then control both — a plugin cannot report under a rule the catalogue already owns"
            );
        }

        if !message.reference.url.starts_with("http") {
            bail!("it reported \"{rule}\" with no citation");
        }

        let declared = Severity::parse(&message.severity).unwrap_or(Severity::Warning);
        let Some(severity) = config.severity_for(&rule, declared) else {
            continue;
        };

        messages.push(Message {
            rule,
            severity,
            message: strip_control(&message.message),
            advice: strip_control(
                &message
                    .advice
                    .unwrap_or_else(|| "See the plugin's own documentation.".to_string()),
            ),
            location: Location::at(message.line.unwrap_or(1), 1),
            source: Source::Plugin,
            file: strip_control(&message.file.unwrap_or_else(|| skill.document.clone())),
            fix: None,
            reference: Reference {
                title: strip_control(&message.reference.title),
                url: strip_control(&message.reference.url),
            },
            confidence: 1.0,
        });
    }

    Ok(messages)
}

/// The file `slint --init-plugin` writes, and the worked example the documentation shows.
pub const STARTER_PACK: &str = r#"# A slint rule pack. Drop it beside your config and point at it:
#
#   [[plugins]]
#   path = "./slint-house-rules.toml"
#
# Every rule needs a citation. "The linter says so" is the failure this tool exists to avoid.

[[rules]]
name = "house/no-todo"
severity = "warning"
summary = "Instructions carry no TODO markers."
rationale = "An agent follows what is written. A TODO is a note to a human that reads as an instruction to a machine."
advice = "Finish the step, or take the line out until it is ready."
pattern = "TODO|FIXME|XXX"
target = "body"
message = "\"{match}\" is a note to a person, in a document a machine follows."
reference = { title = "Skill authoring best practices", url = "https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices" }

[[rules]]
name = "house/names-the-owner"
severity = "info"
summary = "Every skill says who owns it."
rationale = "A skill nobody owns is a skill nobody updates when the tool it drives changes."
advice = "Add an Owner line naming a team."
pattern = "(?i)owner:"
target = "body"
required = true
reference = { title = "Skill authoring best practices", url = "https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices" }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuleSetting;
    use crate::rules::testing::{good_skill, skill_with_body};
    use std::fs;

    fn pack(text: &str) -> (tempfile::TempDir, Plugin) {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pack.toml");
        fs::write(&path, text).unwrap();

        let plugin = load(
            &PluginRef {
                path: "pack.toml".into(),
            },
            temporary.path(),
        )
        .unwrap();

        (temporary, plugin)
    }

    const TODO_PACK: &str = r#"
[[rules]]
name = "house/no-todo"
severity = "warning"
summary = "No TODO markers."
rationale = "An agent follows what is written, and a TODO reads as an instruction."
advice = "Finish the step or take the line out."
pattern = "TODO"
target = "body"
message = "\"{match}\" is a note to a person."
reference = { title = "House style", url = "https://example.com/style" }
"#;

    #[test]
    fn a_pack_rule_fires_on_the_line_it_matched() {
        let (_temporary, plugin) = pack(TODO_PACK);
        let skill = skill_with_body("\n## Culling\n\n1. Import.\n2. TODO decide the flags.\n");

        let (messages, notes) = run(&[plugin], &skill, &Config::default());

        assert!(notes.is_empty());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].rule, "house/no-todo");
        assert_eq!(messages[0].source, Source::Plugin);
        assert_eq!(messages[0].message, "\"TODO\" is a note to a person.");
        assert_eq!(messages[0].location.line, skill.document_line(5));
    }

    #[test]
    fn a_pack_rule_can_be_turned_off_like_any_other() {
        let (_temporary, plugin) = pack(TODO_PACK);
        let skill = skill_with_body("\n## Culling\n\nTODO.\n");

        let mut config = Config::default();
        config
            .rules
            .insert("house/no-todo".into(), RuleSetting::Off);

        let (messages, _) = run(&[plugin], &skill, &config);
        assert!(messages.is_empty());
    }

    #[test]
    fn a_pack_rule_can_be_raised_to_an_error_like_any_other() {
        let (_temporary, plugin) = pack(TODO_PACK);
        let skill = skill_with_body("\n## Culling\n\nTODO.\n");

        let mut config = Config::default();
        config
            .rules
            .insert("house/no-todo".into(), RuleSetting::On(Severity::Error));

        let (messages, _) = run(&[plugin], &skill, &config);
        assert_eq!(messages[0].severity, Severity::Error);
    }

    #[test]
    fn a_required_pattern_fires_when_it_is_missing_rather_than_when_it_is_found() {
        let (_temporary, plugin) = pack(
            r#"
[[rules]]
name = "house/names-the-owner"
severity = "info"
summary = "Every skill says who owns it."
rationale = "A skill nobody owns is a skill nobody updates."
advice = "Add an Owner line."
pattern = "(?i)owner:"
target = "body"
required = true
reference = { title = "House style", url = "https://example.com/style" }
"#,
        );

        let without = skill_with_body("\n## Culling\n\n1. Import.\n");
        let (messages, _) = run(&[plugin], &without, &Config::default());
        assert_eq!(messages.len(), 1);

        let (_temporary2, plugin2) = pack(
            r#"
[[rules]]
name = "house/names-the-owner"
severity = "info"
summary = "Every skill says who owns it."
rationale = "A skill nobody owns is a skill nobody updates."
advice = "Add an Owner line."
pattern = "(?i)owner:"
target = "body"
required = true
reference = { title = "House style", url = "https://example.com/style" }
"#,
        );

        let with = skill_with_body("\n## Culling\n\nOwner: photography\n\n1. Import.\n");
        let (messages, _) = run(&[plugin2], &with, &Config::default());
        assert!(messages.is_empty());
    }

    #[test]
    fn a_pack_rule_can_read_the_description_and_the_bundled_files() {
        let (_temporary, plugin) = pack(
            r#"
[[rules]]
name = "house/no-shouting"
severity = "warning"
summary = "Descriptions do not shout."
rationale = "A description is read as part of a system prompt, where emphasis is noise."
advice = "Write it in a normal voice."
pattern = "!!"
target = "description"
reference = { title = "House style", url = "https://example.com/style" }

[[rules]]
name = "house/no-print-debugging"
severity = "info"
summary = "Scripts do not ship with print debugging."
rationale = "Output an agent did not ask for is output it has to interpret."
advice = "Take the print out."
pattern = "print\\("
target = "files"
reference = { title = "House style", url = "https://example.com/style" }
"#,
        );

        let mut skill = good_skill();
        skill.description = "Culls a shoot!! Use when triaging RAW files.".into();
        skill.files.push(crate::skill::BundledFile {
            path: "scripts/cull.py".into(),
            bytes: 20,
            executable: true,
            text: Some("print(1)\n".into()),
        });

        let (messages, _) = run(&[plugin], &skill, &Config::default());

        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|one| one.rule == "house/no-shouting"));
        assert!(
            messages
                .iter()
                .any(|one| one.rule == "house/no-print-debugging" && one.file.ends_with("cull.py"))
        );
    }

    #[test]
    fn a_pack_rule_with_no_citation_is_refused_at_load() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pack.toml");
        fs::write(
            &path,
            r#"
[[rules]]
name = "house/no-todo"
summary = "No TODOs."
rationale = "An agent follows what is written."
advice = "Take it out."
pattern = "TODO"
reference = { title = "Nothing", url = "made up" }
"#,
        )
        .unwrap();

        let failure = load(
            &PluginRef {
                path: "pack.toml".into(),
            },
            temporary.path(),
        )
        .unwrap_err()
        .to_string();

        assert!(failure.contains("citation"));
    }

    #[test]
    fn a_pack_rule_that_is_not_namespaced_is_refused_at_load() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("pack.toml"),
            r#"
[[rules]]
name = "no-todo"
summary = "No TODOs."
rationale = "An agent follows what is written."
advice = "Take it out."
pattern = "TODO"
reference = { title = "House style", url = "https://example.com/style" }
"#,
        )
        .unwrap();

        let failure = load(
            &PluginRef {
                path: "pack.toml".into(),
            },
            temporary.path(),
        )
        .unwrap_err()
        .to_string();

        assert!(failure.contains("namespaced"));
    }

    #[test]
    fn a_pattern_that_does_not_compile_is_refused_at_load() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("pack.toml"),
            r#"
[[rules]]
name = "house/broken"
summary = "A rule with a broken pattern."
rationale = "It should not load at all."
advice = "Fix the pattern."
pattern = "([unclosed"
reference = { title = "House style", url = "https://example.com/style" }
"#,
        )
        .unwrap();

        assert!(
            load(
                &PluginRef {
                    path: "pack.toml".into()
                },
                temporary.path()
            )
            .is_err()
        );
    }

    #[test]
    fn the_starter_pack_loads_and_every_rule_in_it_is_citable() {
        let (_temporary, plugin) = pack(STARTER_PACK);

        let names = plugin.rule_names();
        assert_eq!(names.len(), 2);
        assert!(names.iter().all(|name| name.starts_with("house/")));
    }

    #[test]
    fn what_a_plugin_reports_becomes_findings_like_any_other() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"team/needs-review","severity":"info","message":"Not reviewed by the team yet.","reference":{"title":"Team handbook","url":"https://example.com/handbook"}}]}"#,
        )
        .unwrap();

        let messages = into_messages(parsed, &good_skill(), &Config::default()).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].rule, "team/needs-review");
        assert_eq!(messages[0].severity, Severity::Info);
        assert_eq!(messages[0].source, Source::Plugin);
    }

    #[test]
    fn a_plugin_reporting_without_a_citation_is_refused() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"team/needs-review","message":"Because I say so.","reference":{"title":"","url":"nowhere"}}]}"#,
        )
        .unwrap();

        let failure = into_messages(parsed, &good_skill(), &Config::default())
            .unwrap_err()
            .to_string();

        assert!(failure.contains("citation"));
    }

    #[test]
    fn a_plugin_reporting_an_unnamespaced_rule_is_refused() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"needsreview","message":"A thing.","reference":{"title":"Handbook","url":"https://example.com"}}]}"#,
        )
        .unwrap();

        assert!(into_messages(parsed, &good_skill(), &Config::default()).is_err());
    }

    #[test]
    fn a_plugin_rule_can_be_turned_off_in_the_config_like_any_other() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"team/needs-review","message":"A thing.","reference":{"title":"Handbook","url":"https://example.com"}}]}"#,
        )
        .unwrap();

        let mut config = Config::default();
        config
            .rules
            .insert("team/needs-review".into(), RuleSetting::Off);

        assert!(
            into_messages(parsed, &good_skill(), &config)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_wasm_plugin_that_cannot_be_loaded_becomes_a_note_rather_than_taking_the_run_down() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("broken.wasm");
        fs::write(&path, b"not webassembly at all").unwrap();

        let plugin = load(
            &PluginRef {
                path: "broken.wasm".into(),
            },
            temporary.path(),
        )
        .unwrap();
        let (messages, notes) = run(&[plugin], &good_skill(), &Config::default());

        assert!(messages.is_empty());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("did not run"));
    }

    #[test]
    fn a_plugin_the_config_names_and_that_does_not_exist_is_an_error() {
        let temporary = tempfile::tempdir().unwrap();

        assert!(
            load(
                &PluginRef {
                    path: "missing.wasm".into()
                },
                temporary.path()
            )
            .is_err()
        );
        assert!(
            load(
                &PluginRef {
                    path: "missing.toml".into()
                },
                temporary.path()
            )
            .is_err()
        );
    }

    // Regression tests for the audit findings in #85 and #111: a pack's captured text and a
    // plugin's freeform strings reach a terminal renderer, and a rule id that collides with a
    // built-in makes one config switch control two unrelated rules.

    const ANSI_PROBE_PACK: &str = r#"
[[rules]]
name = "house/ansi"
severity = "warning"
summary = "ANSI injection probe.\u001B[2J"
rationale = "Checking whether captured text is stripped before it hits the renderer."
advice = "Remove the marker.\u001B[31m"
pattern = "MARK.*MARK"
target = "body"
message = "found: {match}\u001B[7m"
reference = { title = "PoC\u001B[0m", url = "https://example.com/poc" }
"#;

    #[test]
    fn captured_text_a_pack_echoes_is_stripped_of_control_characters() {
        let (_temporary, plugin) = pack(ANSI_PROBE_PACK);
        let skill = skill_with_body("\nMARK\u{1b}[31mFAKE ERROR\u{1b}[0m\u{1b}[2JMARK\n");

        let (messages, _) = run(&[plugin], &skill, &Config::default());

        assert_eq!(messages.len(), 1);
        assert!(
            !messages[0].message.contains('\u{1b}'),
            "captured text must be stripped before interpolation, got: {:?}",
            messages[0].message
        );
        assert!(
            messages[0]
                .message
                .contains("found: MARK[31mFAKE ERROR[0m[2JMARK[7m")
        );
    }

    #[test]
    fn the_strings_a_pack_author_wrote_are_stripped_of_control_characters() {
        let (_temporary, plugin) = pack(ANSI_PROBE_PACK);
        let skill = skill_with_body("\nMARKcleanMARK\n");

        let (messages, _) = run(&[plugin], &skill, &Config::default());

        assert_eq!(messages.len(), 1);
        let finding = &messages[0];
        for field in [
            ("summary", finding.message.clone()),
            ("advice", finding.advice.clone()),
            ("reference title", finding.reference.title.clone()),
        ] {
            assert!(
                !field.1.contains('\u{1b}'),
                "{} must be stripped of control characters, got: {:?}",
                field.0,
                field.1
            );
        }
    }

    #[test]
    fn text_a_wasm_plugin_reports_is_stripped_of_control_characters() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"team/needs-review","message":"\u001b[2J\u001b[H PWNED","advice":"\u001b[31mrun this\u001b[0m","file":"\u001b[1mSKILL.md","reference":{"title":"Handbook\u001b[5m","url":"https://example.com/po\u001bc"}}]}"#,
        )
        .unwrap();

        let messages = into_messages(parsed, &good_skill(), &Config::default()).unwrap();

        assert_eq!(messages.len(), 1);
        let finding = &messages[0];
        for field in [
            ("message", finding.message.as_str()),
            ("advice", finding.advice.as_str()),
            ("file", finding.file.as_str()),
            ("reference title", finding.reference.title.as_str()),
            ("reference url", finding.reference.url.as_str()),
        ] {
            assert!(
                !field.1.contains('\u{1b}'),
                "{} must be stripped of control characters, got: {:?}",
                field.0,
                field.1
            );
        }
    }

    #[test]
    fn a_pack_rule_named_after_a_builtin_is_refused_at_load() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("pack.toml"),
            r#"
[[rules]]
name = "body/posix-paths"
summary = "Collides with a built-in rule id."
rationale = "Two unrelated rules would share one on/off switch in the config."
advice = "Namespace it under your own house."
pattern = "collide"
reference = { title = "House style", url = "https://example.com/style" }
"#,
        )
        .unwrap();

        let failure = load(
            &PluginRef {
                path: "pack.toml".into(),
            },
            temporary.path(),
        )
        .unwrap_err()
        .to_string();

        assert!(
            failure.contains("body/posix-paths") && failure.contains("built-in"),
            "the load must fail naming the colliding id, got: {failure}"
        );
    }

    #[test]
    fn a_wasm_plugin_cannot_report_under_a_builtin_rule_id() {
        let parsed: PluginOutput = serde_json::from_str(
            r#"{"messages":[{"rule":"body/posix-paths","message":"A thing.","reference":{"title":"Handbook","url":"https://example.com"}}]}"#,
        )
        .unwrap();

        let failure = into_messages(parsed, &good_skill(), &Config::default())
            .unwrap_err()
            .to_string();

        assert!(
            failure.contains("body/posix-paths") && failure.contains("built-in"),
            "reporting under a built-in id must fail naming it, got: {failure}"
        );
    }
}
