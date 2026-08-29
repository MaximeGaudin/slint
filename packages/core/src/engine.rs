//! Running the rules.
//!
//! Static rules first and always (in parallel across skills), then plugins, then the model pass if
//! one is configured — also in parallel across skills. Everything cheap runs before anything
//! expensive is asked for, and a model that is unreachable must never take the rest of the review
//! down with it.

use anyhow::Result;
use rayon::prelude::*;
use regex::Regex;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::config::Config;
use crate::diagnostics::{Message, Report, SkillReport};
use crate::llm;
use crate::plugin::Plugin;
use crate::rules::{self, RuleContext};
use crate::skill::{self, Skill};

/// How much of the catalogue to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passes {
    pub plugins: bool,
    /// The model pass. Off unless a provider is configured *and* the caller wants it.
    pub model: bool,
}

impl Default for Passes {
    fn default() -> Self {
        Passes {
            plugins: true,
            model: false,
        }
    }
}

/// `<!-- slint-disable rule -->` anywhere in the document, and its per-line form.
static DISABLE_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<!--\s*slint-disable\s+([^\s>-][^>]*?)\s*-->")
        .expect("the disable pattern compiles")
});

static DISABLE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<!--\s*slint-disable-next-line\s+([^\s>-][^>]*?)\s*-->")
        .expect("the disable-next-line pattern compiles")
});

/// Why the model pass failed, and what the reader can do about it.
///
/// An error from a provider is usually one of three things — no key, a model id it does not know,
/// or something transient — and the difference matters to whoever has to fix it. The whole chain is
/// printed, because the useful sentence is nearly always the innermost one.
fn model_failure(llm: &crate::config::LlmConfig, failure: &anyhow::Error) -> String {
    let mut note = format!(
        "The rules that need a model did not run. {llm_provider}/{model} said: {failure:#}",
        llm_provider = format!("{:?}", llm.provider).to_lowercase(),
        model = llm.model,
    );

    match &llm.api_key_env {
        Some(variable) if std::env::var(variable).is_err() => {
            note.push_str(&format!(
                " — {variable} is not set in this shell. Export it, or point api_key_env at the variable that holds the key."
            ));
        }
        Some(variable) => {
            note.push_str(&format!(
                " — {variable} is set, so check the model id and, if it is a gateway, base_url."
            ));
        }
        None => {
            note.push_str(
                " — no api_key_env is configured, so nothing was sent with the request. Name the environment variable holding the key.",
            );
        }
    }

    note
}

/// What a document says it does not want to hear about.
#[derive(Debug, Default, Clone)]
pub struct Suppressions {
    /// Rules turned off for the whole file.
    pub file: BTreeSet<String>,
    /// Rules turned off for one line, keyed by the line they apply to.
    pub lines: Vec<(usize, String)>,
}

impl Suppressions {
    pub fn read(source: &str) -> Self {
        let mut suppressions = Suppressions::default();
        let lines: Vec<&str> = source.lines().collect();

        for (index, line) in lines.iter().enumerate() {
            if let Some(found) = DISABLE_LINE.captures(line) {
                let rules = split_rules(&found[1]);
                // Normally the next line only. When that line opens a fenced
                // code block, cover every line inside the fence too — example
                // paths live on the lines after ```, not on the fence marker.
                for covered in lines_covered_by_disable_next(&lines, index) {
                    for rule in &rules {
                        suppressions.lines.push((covered, rule.clone()));
                    }
                }
                continue;
            }

            if let Some(found) = DISABLE_FILE.captures(line) {
                for rule in split_rules(&found[1]) {
                    suppressions.file.insert(rule);
                }
            }
        }

        suppressions
    }

    pub fn allows(&self, message: &Message) -> bool {
        if self.file.contains(&message.rule) {
            return false;
        }

        !self
            .lines
            .iter()
            .any(|(line, rule)| *line == message.location.line && rule == &message.rule)
    }
}

fn split_rules(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

/// 1-based document lines silenced by a disable-next-line on `comment_index` (0-based).
fn lines_covered_by_disable_next(lines: &[&str], comment_index: usize) -> Vec<usize> {
    let next = comment_index + 1;
    if next >= lines.len() {
        return Vec::new();
    }

    let opener = lines[next].trim_start();
    let Some(marker) = fence_opener(opener) else {
        return vec![next + 1];
    };

    let mut covered = vec![next + 1];
    for (offset, line) in lines.iter().enumerate().skip(next + 1) {
        covered.push(offset + 1);
        if line.trim_start().starts_with(marker) {
            break;
        }
    }
    covered
}

fn fence_opener(line: &str) -> Option<&'static str> {
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// Every static rule that is on, against one skill.
pub fn lint_skill(skill: &Skill, config: &Config) -> Vec<Message> {
    let mut messages = Vec::new();

    for rule in rules::registry() {
        let meta = rule.meta();
        let Some(severity) = config.severity_for(meta.name, meta.default_severity) else {
            continue;
        };

        let mut context = RuleContext::new(skill, config, meta, severity);
        rule.check(&mut context);
        messages.extend(context.finish());
    }

    messages
}

/// The rules that need every skill at once.
pub fn lint_project(skills: &[Skill], config: &Config) -> Vec<Message> {
    let mut messages = Vec::new();

    for rule in rules::project_registry() {
        let meta = rule.meta();
        let Some(severity) = config.severity_for(meta.name, meta.default_severity) else {
            continue;
        };

        messages.extend(rule.check(skills, config, severity));
    }

    messages
}

/// Reads, lints and reports on every skill under the given paths.
pub fn run(
    paths: &[PathBuf],
    config: &Config,
    plugins: &[Plugin],
    passes: Passes,
) -> Result<Report> {
    let ignore = skill::build_ignore(&config.ignore)?;
    let directories = skill::discover(paths, &ignore)?;

    let mut skills = Vec::new();
    let mut unreadable = Vec::new();

    for directory in directories {
        match skill::read(&directory) {
            Ok(one) => skills.push(one),
            Err(failure) => unreadable.push((directory, failure.to_string())),
        }
    }

    // Static rules, in parallel: they are pure functions of a skill, which is the property that
    // makes this safe and the reason the whole run is fast enough to sit in an editor's save hook.
    let mut per_skill: Vec<SkillReport> = skills
        .par_iter()
        .map(|one| {
            let mut messages = lint_skill(one, config);
            let mut notes = one.notes.clone();

            if passes.plugins {
                let (found, plugin_notes) = crate::plugin::run(plugins, one, config);
                messages.extend(found);
                notes.extend(plugin_notes);
            }

            SkillReport {
                path: one.directory.display().to_string(),
                name: one.name.clone(),
                messages,
                notes,
            }
        })
        .collect();

    for message in lint_project(&skills, config) {
        if let Some(report) = per_skill
            .iter_mut()
            .find(|one| message.file.starts_with(&one.path))
        {
            report.messages.push(message);
        }
    }

    // The model pass, last and optional. One request per skill, in parallel: the skills do not
    // share state, and waiting on them one after another is how a workspace review feels broken.
    // A provider that is unreachable leaves a note on that skill rather than an error on the run —
    // the static half already produced something worth reading. An unparseable reply after retry,
    // when the model pass was explicitly requested, hard-fails the run instead of silently
    // dropping every finding.
    if passes.model && config.llm.is_configured() {
        enum ModelOutcome {
            Ok(Vec<Message>, Vec<String>),
            Soft(String),
            Hard(anyhow::Error),
        }

        let outcomes: Vec<ModelOutcome> = skills
            .par_iter()
            .map(|one| match llm::review(one, config) {
                Ok((messages, notes)) => ModelOutcome::Ok(messages, notes),
                Err(failure) if llm::is_unparseable_findings(&failure) => {
                    ModelOutcome::Hard(failure)
                }
                // The whole chain, and what to do about it. "asking openrouter::…" on its own
                // tells the reader that something failed and nothing about which part.
                Err(failure) => ModelOutcome::Soft(model_failure(&config.llm, &failure)),
            })
            .collect();

        let mut hard_fail = None;
        for (report, outcome) in per_skill.iter_mut().zip(outcomes) {
            match outcome {
                ModelOutcome::Ok(messages, notes) => {
                    report.messages.extend(messages);
                    report.notes.extend(notes);
                }
                ModelOutcome::Soft(note) => report.notes.push(note),
                ModelOutcome::Hard(failure) => {
                    report.notes.push(model_failure(&config.llm, &failure));
                    hard_fail = Some(failure);
                }
            }
        }

        if let Some(failure) = hard_fail {
            return Err(failure);
        }
    } else if passes.model && !config.llm.is_configured() {
        // Asked for, and impossible: say exactly what is missing and where it goes.
        for report in &mut per_skill {
            report.notes.push(
                "The model rules were asked for and no provider is configured. Add an [llm] block to slint.toml with a provider, a model and api_key_env — see https://slint.dev/config/."
                    .to_string(),
            );
        }
    } else if !passes.model {
        // Intentional skip, not a failure. Keep it short: the editor runs --no-llm on every
        // static pass and a lecture about slint.toml there reads like something went wrong.
        let count = llm::rules::all().len();

        if let Some(first) = per_skill.first_mut() {
            first.notes.push(format!(
                "Skipped {count} model rules (not requested). Pass --llm to run them."
            ));
        }
    }

    // Anything the document itself asked not to hear about.
    for (report, one) in per_skill.iter_mut().zip(skills.iter()) {
        let suppressions = Suppressions::read(&one.source);
        report
            .messages
            .retain(|message| suppressions.allows(message));
    }

    for (directory, failure) in unreadable {
        per_skill.push(SkillReport {
            path: directory.display().to_string(),
            name: directory
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
            messages: Vec::new(),
            notes: vec![format!(
                "This could not be read, so nothing was checked: {failure}"
            )],
        });
    }

    Ok(Report {
        skills: per_skill,
        fixed: 0,
    }
    .sorted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuleSetting;
    use crate::diagnostics::Severity;
    use crate::rules::testing::{good_skill, skill_with_body};
    use std::fs;

    fn write_skill(root: &std::path::Path, name: &str, document: &str) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), document).unwrap();
        directory
    }

    const GOOD: &str = "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Import the RAW files.\n2. Flag keepers with P.\n";

    #[test]
    fn a_good_skill_produces_nothing() {
        assert!(lint_skill(&good_skill(), &Config::default()).is_empty());
    }

    #[test]
    fn default_passes_do_not_opt_into_the_model_pass() {
        assert!(
            !Passes::default().model,
            "default Passes must keep the paid model pass off"
        );
        assert!(Passes::default().plugins);
    }

    #[test]
    fn a_rule_turned_off_does_not_run_at_all() {
        let skill = skill_with_body("\n## Culling\n\nRead scripts\\notes.md.\n");

        assert_eq!(lint_skill(&skill, &Config::default()).len(), 1);

        let mut config = Config::default();
        config
            .rules
            .insert("body/posix-paths".into(), RuleSetting::Off);

        assert!(lint_skill(&skill, &config).is_empty());
    }

    #[test]
    fn a_disable_comment_silences_a_rule_for_the_file() {
        let source =
            format!("{GOOD}\n<!-- slint-disable body/posix-paths -->\nRead scripts\\notes.md.\n");
        let suppressions = Suppressions::read(&source);

        assert!(suppressions.file.contains("body/posix-paths"));
    }

    #[test]
    fn a_disable_next_line_comment_silences_only_the_line_below_it() {
        let suppressions = Suppressions::read(
            "one\n<!-- slint-disable-next-line body/posix-paths -->\ntwo\nthree\n",
        );

        assert_eq!(
            suppressions.lines,
            vec![(3, "body/posix-paths".to_string())]
        );
        assert!(suppressions.file.is_empty());
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/8 —
    /// example-only paths live inside fenced blocks; a disable-next-line
    /// immediately before the opening fence must cover every line in that fence.
    #[test]
    fn a_disable_next_line_before_a_fence_covers_lines_inside_the_fence() {
        let suppressions = Suppressions::read(
            "one\n<!-- slint-disable-next-line bundle/no-dangling-path -->\n```bash\nscripts/run.sh --help\n```\n",
        );

        assert!(
            suppressions
                .lines
                .iter()
                .any(|(line, rule)| *line == 4 && rule == "bundle/no-dangling-path"),
            "expected the path line inside the fence to be suppressed, got {suppressions:?}"
        );
        assert!(suppressions.file.is_empty());
    }

    #[test]
    fn a_disable_next_line_before_a_fence_silences_dangling_path_in_a_real_run() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "demo-example-path-ignore",
            "---\nname: demo-example-path-ignore\ndescription: Demo skill showing an example-only path with an ESLint-style ignore comment. Use when testing inline rule suppression.\n---\n\n# Demo example path ignore\n\n## Workflow\n\n<!-- slint-disable-next-line bundle/no-dangling-path -->\n```bash\nscripts/run.sh --help\n```\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert!(
            !report.skills[0]
                .messages
                .iter()
                .any(|one| one.rule == "bundle/no-dangling-path"),
            "expected fence-scoped disable-next-line to silence dangling path, got {:?}",
            report.skills[0].messages
        );
    }

    #[test]
    fn several_rules_can_be_named_in_one_comment() {
        let suppressions =
            Suppressions::read("<!-- slint-disable body/posix-paths, name/not-generic -->\n");

        assert!(suppressions.file.contains("body/posix-paths"));
        assert!(suppressions.file.contains("name/not-generic"));
    }

    #[test]
    fn running_over_a_tree_reports_every_skill_it_finds() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(temporary.path(), "photo-culling", GOOD);
        write_skill(
            temporary.path(),
            "helper",
            "---\nname: helper\ndescription: Does things.\n---\n\n## Helper\n\nDo them.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert_eq!(report.skills.len(), 2);

        let helper = report
            .skills
            .iter()
            .find(|one| one.name == "helper")
            .unwrap();
        assert!(
            helper
                .messages
                .iter()
                .any(|one| one.rule == "name/not-generic")
        );
        assert!(
            helper
                .messages
                .iter()
                .any(|one| one.rule == "description/min-length")
        );
    }

    #[test]
    fn a_project_rule_attaches_its_findings_to_the_file_they_are_about() {
        let temporary = tempfile::tempdir().unwrap();
        let shared = "---\nname: {name}\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Import.\n";

        write_skill(temporary.path(), "one", &shared.replace("{name}", "one"));
        write_skill(temporary.path(), "two", &shared.replace("{name}", "two"));

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        for skill in &report.skills {
            assert!(
                skill
                    .messages
                    .iter()
                    .any(|one| one.rule == "project/distinct-descriptions"),
                "{} heard nothing about its twin",
                skill.name
            );
        }
    }

    #[test]
    fn a_static_only_run_says_how_to_run_the_rest() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(temporary.path(), "photo-culling", GOOD);

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        let note = report.skills[0].notes.join(" ");

        assert!(note.contains("Skipped"), "{note}");
        assert!(note.contains("model rules"), "{note}");
        assert!(note.contains("--llm"), "{note}");
    }

    #[test]
    fn asking_for_the_model_pass_with_no_provider_says_what_is_missing() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(temporary.path(), "photo-culling", GOOD);

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: true,
            },
        )
        .unwrap();

        let note = report.skills[0].notes.join(" ");

        assert!(note.contains("no provider is configured"), "{note}");
        assert!(note.contains("api_key_env"), "{note}");
    }

    #[test]
    fn a_provider_that_fails_says_what_it_said_and_what_to_check() {
        let failure =
            anyhow::anyhow!("model not found").context("asking openrouter::deepseek/deepseek-v4");

        let llm = crate::config::LlmConfig {
            provider: crate::config::Provider::Openrouter,
            model: "deepseek/deepseek-v4".into(),
            api_key_env: Some("SLINT_TEST_ABSENT_KEY".into()),
            ..crate::config::LlmConfig::default()
        };

        let note = model_failure(&llm, &failure);

        // The whole chain, because the useful sentence is nearly always the innermost one.
        assert!(note.contains("model not found"), "{note}");
        assert!(
            note.contains("asking openrouter::deepseek/deepseek-v4"),
            "{note}"
        );
        assert!(note.contains("SLINT_TEST_ABSENT_KEY is not set"), "{note}");
    }

    #[test]
    fn a_failure_with_the_key_present_points_at_the_model_id_instead() {
        // SAFETY: single-threaded within this test, and the variable is only read by this call.
        unsafe { std::env::set_var("SLINT_TEST_PRESENT_KEY", "value") };

        let llm = crate::config::LlmConfig {
            provider: crate::config::Provider::Openai,
            model: "gpt-nonexistent".into(),
            api_key_env: Some("SLINT_TEST_PRESENT_KEY".into()),
            ..crate::config::LlmConfig::default()
        };

        let note = model_failure(&llm, &anyhow::anyhow!("unknown model"));

        assert!(note.contains("is set, so check the model id"), "{note}");

        unsafe { std::env::remove_var("SLINT_TEST_PRESENT_KEY") };
    }

    #[test]
    fn a_suppression_comment_is_honoured_by_a_real_run() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "helper",
            "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n<!-- slint-disable name/not-generic -->\n\n## Helper\n\nDo them.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert!(
            !report.skills[0]
                .messages
                .iter()
                .any(|one| one.rule == "name/not-generic")
        );
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/87 —
    /// a disable comment inside a fenced code block documents the syntax;
    /// it is an example, not a live directive for the rest of the document.
    #[test]
    fn a_disable_comment_inside_a_fence_is_documentation_not_a_directive() {
        for (open, close) in [("```markdown", "```"), ("~~~", "~~~")] {
            let source = format!(
                "one\n{open}\n<!-- slint-disable body/posix-paths -->\nRead scripts\\notes.md.\n{close}\ntwo\n"
            );
            let suppressions = Suppressions::read(&source);

            assert!(suppressions.file.is_empty(), "{open}: {suppressions:?}");
            assert!(suppressions.lines.is_empty(), "{open}: {suppressions:?}");
        }
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/87 —
    /// the documented-but-not-live comment must leave the real findings alone.
    #[test]
    fn a_disable_comment_inside_a_fence_does_not_silence_the_live_document() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "fence-documented-disable",
            "---\nname: fence-documented-disable\ndescription: Documents how to write slint-disable comments for skill authors. Use when writing docs about suppression syntax.\n---\n\n## How to suppress a rule\n\n```markdown\n<!-- slint-disable body/posix-paths -->\n```\n\n## Actual instructions\n\nRead scripts\\notes.md.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert!(
            report.skills[0]
                .messages
                .iter()
                .any(|one| one.rule == "body/posix-paths"),
            "a disable comment documented inside a fence must not silence the live document, got {:?}",
            report.skills[0].messages
        );
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/88 —
    /// a disable comment is scoped to the document it is written in; it must
    /// not reach into files bundled beside that document.
    #[test]
    fn a_disable_comment_in_the_document_does_not_reach_into_bundled_files() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = write_skill(
            temporary.path(),
            "cross-file-suppress",
            "---\nname: cross-file-suppress\ndescription: Culls a photo shoot in Lightroom by flagging keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n<!-- slint-disable bundle/contents-list -->\n\n## Steps\n\nSee references/big.md for details on this whole workflow end to end.\n",
        );

        let mut long = String::from("# Big reference\n\nProse about the workflow, at length.\n");
        for index in 1..=120 {
            long.push_str(&format!("\n## Section {index}\n\nSome prose about step {index} of the workflow.\n"));
        }
        fs::create_dir_all(directory.join("references")).unwrap();
        fs::write(directory.join("references/big.md"), long).unwrap();

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert!(
            report.skills[0].messages.iter().any(|one| one.rule == "bundle/contents-list"
                && one.file.ends_with("references/big.md")),
            "a disable comment written in SKILL.md must not silence findings on bundled files, got {:?}",
            report.skills[0].messages
        );
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/113 —
    /// a directive that suppressed nothing is dead weight at best and a typo
    /// at worst, so it is reported the way eslint reports unused directives.
    #[test]
    fn an_unused_disable_comment_is_reported() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "typo-rule",
            "---\nname: typo-rule\ndescription: Demonstrates an unused suppression comment with a misspelled rule name in it here. Use when checking suppression reporting.\n---\n\n<!-- slint-disable body/posix-path -->\n\n## Actual instructions\n\nNothing wrong here at all really, just prose about the workflow steps involved.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        let unused = report.skills[0]
            .messages
            .iter()
            .find(|one| one.rule == "suppression/unused");
        let unused = unused.unwrap_or_else(|| {
            panic!(
                "expected an unused-suppression finding, got {:?}",
                report.skills[0].messages
            )
        });

        assert_eq!(unused.severity, Severity::Warning);
        assert!(unused.message.contains("body/posix-path"), "{:?}", unused.message);
        assert_eq!(unused.location.line, 6, "{:?}", unused.message);
    }

    #[test]
    fn a_used_disable_comment_is_not_reported() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "helper",
            "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n<!-- slint-disable name/not-generic -->\n\n## Helper\n\nDo them.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert!(
            !report.skills[0]
                .messages
                .iter()
                .any(|one| one.rule == "suppression/unused"),
            "a directive that silenced a real finding is not unused, got {:?}",
            report.skills[0].messages
        );
    }

    #[test]
    fn a_directory_that_cannot_be_read_is_reported_rather_than_failing_the_run() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("broken");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), [0xff, 0xfe, 0xfd]).unwrap();

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        assert_eq!(report.skills.len(), 1);
        assert!(report.skills[0].notes[0].contains("could not be read"));
    }

    #[test]
    fn the_report_is_ordered_worst_first() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "helper",
            "---\nname: helper\ndescription: Does things.\n---\n\n## Helper\n\nRun scripts/missing.py.\n",
        );

        let report = run(
            &[temporary.path().to_path_buf()],
            &Config::default(),
            &[],
            Passes {
                plugins: false,
                model: false,
            },
        )
        .unwrap();

        let severities: Vec<Severity> = report.skills[0]
            .messages
            .iter()
            .map(|one| one.severity)
            .collect();
        let mut sorted = severities.clone();
        sorted.sort();

        assert_eq!(severities, sorted);
    }
}
