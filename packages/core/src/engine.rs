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
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::config::Config;
use crate::diagnostics::{Location, Message, Reference, Report, Severity, SkillReport, Source};
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
    /// Rules turned off for the whole document the comment is written in.
    pub file: BTreeSet<String>,
    /// Rules turned off for one line, keyed by the line they apply to.
    pub lines: Vec<(usize, String)>,
    /// Every directive as written, so one that silenced nothing can be named.
    directives: Vec<Directive>,
}

/// One directive as the document wrote it.
#[derive(Debug, Clone)]
struct Directive {
    rule: String,
    /// The 1-based line the comment sits on, for a diagnostic that points at it.
    comment_line: usize,
    /// The 1-based lines the directive covers. Empty means every line of the
    /// document the comment is written in.
    covers: Vec<usize>,
}

impl Directive {
    /// Whether a finding on `line` is inside this directive's scope.
    fn covers(&self, line: usize) -> bool {
        self.covers.is_empty() || self.covers.contains(&line)
    }
}

/// A directive that named a rule nothing ever fired for.
///
/// A comment that silenced nothing is either a typo or stale — either way the
/// author believes something is suppressed that is not, so it is reported the
/// way eslint reports unused disable directives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedSuppression {
    /// The rule the comment named.
    pub rule: String,
    /// The 1-based line the comment sits on.
    pub line: usize,
    /// Whether the comment was `slint-disable-next-line` rather than `slint-disable`.
    pub next_line: bool,
}

impl Suppressions {
    pub fn read(source: &str) -> Self {
        let mut suppressions = Suppressions::default();
        let lines: Vec<&str> = source.lines().collect();
        let mut inside_fence: Option<&'static str> = None;

        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();

            // A directive inside a fenced code block is documentation of the
            // syntax, not a live instruction: the fence is an example, so
            // nothing in it — comment included — is read as a directive.
            if let Some(marker) = inside_fence {
                if trimmed.starts_with(marker) {
                    inside_fence = None;
                }
                continue;
            }

            if let Some(marker) = fence_opener(trimmed) {
                inside_fence = Some(marker);
                continue;
            }

            if let Some(found) = DISABLE_LINE.captures(line) {
                let rules = split_rules(&found[1]);
                // Normally the next line only. When that line opens a fenced
                // code block, cover every line inside the fence too — example
                // paths live on the lines after ```, not on the fence marker.
                let covered = lines_covered_by_disable_next(&lines, index);
                for rule in rules {
                    suppressions
                        .lines
                        .extend(covered.iter().map(|line| (*line, rule.clone())));
                    suppressions.directives.push(Directive {
                        rule: rule.clone(),
                        comment_line: index + 1,
                        covers: covered.clone(),
                    });
                }
                continue;
            }

            if let Some(found) = DISABLE_FILE.captures(line) {
                for rule in split_rules(&found[1]) {
                    suppressions.file.insert(rule.clone());
                    suppressions.directives.push(Directive {
                        rule,
                        comment_line: index + 1,
                        covers: Vec::new(),
                    });
                }
            }
        }

        suppressions
    }

    /// Whether a finding may stand.
    ///
    /// Suppressions only ever apply to the document they were written in: a
    /// comment in SKILL.md says nothing about findings on the files bundled
    /// beside it, and its line numbers mean nothing there either.
    pub fn allows(&self, document: &str, message: &Message) -> bool {
        if message.file != document {
            return true;
        }

        if self.file.contains(&message.rule) {
            return false;
        }

        !self
            .lines
            .iter()
            .any(|(line, rule)| *line == message.location.line && rule == &message.rule)
    }

    /// The directives no finding in `messages` ever matched.
    ///
    /// A directive is used when a finding in the document it was written in
    /// named its rule inside its scope — even if another directive silenced
    /// that finding first, the comment was not dead.
    pub fn unused(&self, document: &str, messages: &[Message]) -> Vec<UnusedSuppression> {
        self.directives
            .iter()
            .filter(|directive| {
                !messages.iter().any(|message| {
                    message.file == document
                        && message.rule == directive.rule
                        && directive.covers(message.location.line)
                })
            })
            .map(|directive| UnusedSuppression {
                rule: directive.rule.clone(),
                line: directive.comment_line,
                next_line: !directive.covers.is_empty(),
            })
            .collect()
    }
}

fn split_rules(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

/// The rule name slint reports an unused suppression comment under. Not part of
/// the catalogue: the engine writes it, and a config can still retune it.
const UNUSED_SUPPRESSION_RULE: &str = "suppression/unused";

/// The finding for a directive that silenced nothing.
fn unused_suppression_message(
    found: &UnusedSuppression,
    document: &str,
    severity: Severity,
) -> Message {
    let comment = if found.next_line {
        "slint-disable-next-line"
    } else {
        "slint-disable"
    };
    let scope = if found.next_line {
        " on the lines it covers"
    } else {
        ""
    };

    Message {
        rule: UNUSED_SUPPRESSION_RULE.to_string(),
        severity,
        message: format!(
            "This {comment} comment names {rule}, but nothing{scope} ever fired for it. Check the rule name for a typo, or remove the comment.",
            rule = found.rule,
        ),
        advice: "Name rules exactly as slint writes them (for example: body/posix-paths), and delete the comment once nothing needs suppressing.".into(),
        location: Location::at(found.line, 1),
        source: Source::Static,
        file: document.to_string(),
        fix: None,
        reference: Reference {
            title: "Report unused disable directives — ESLint".into(),
            url: "https://eslint.org/docs/latest/use/command-line-interface#--report-unused-disable-directives".into(),
        },
        confidence: 1.0,
    }
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

/// What the model pass does for one skill, and the seam its tests swap a fake in through. The
/// shared client is resolved by the caller, so a run pays for one client (and one runtime) no
/// matter how many skills it reviews.
type ModelReview<'a> =
    dyn Fn(&Skill, &Config) -> Result<(Vec<Message>, Vec<String>)> + Send + Sync + 'a;

/// Reads, lints and reports on every skill under the given paths.
pub fn run(
    paths: &[PathBuf],
    config: &Config,
    plugins: &[Plugin],
    passes: Passes,
) -> Result<Report> {
    // One client, one runtime, one limiter for the whole pass: a client per skill pays a fresh
    // TLS handshake per request and answers to no one about how many of them run at once.
    let shared = llm::GenAiChat::new(&config.llm);
    match &shared {
        Ok(client) => run_with_reviewer(paths, config, plugins, passes, &|skill, cfg| {
            llm::review_shared(client, skill, cfg)
        }),
        Err(failure) => {
            let message = format!("{failure:#}");
            run_with_reviewer(paths, config, plugins, passes, &move |_skill, _cfg| {
                Err(anyhow::anyhow!("{message}"))
            })
        }
    }
}

/// The same, with the model pass injectable — which is how its degradation behaviour is tested
/// without a network.
pub fn run_with_reviewer(
    paths: &[PathBuf],
    config: &Config,
    plugins: &[Plugin],
    passes: Passes,
    review: &ModelReview<'_>,
) -> Result<Report> {
    let ignore = skill::build_ignore(&config.ignore)?;
    let discovery = skill::discover(paths, &ignore)?;
    let directories = discovery.directories;

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

    // A finding is filed under the directory it names, matched exactly. A string prefix would let
    // `a` swallow every finding about a sibling `ab`, since `ab/SKILL.md` starts with `a`.
    for message in lint_project(&skills, config) {
        let directory = Path::new(&message.file).parent();

        if let Some(report) = per_skill
            .iter_mut()
            .find(|one| directory == Some(Path::new(&one.path)))
        {
            report.messages.push(message);
        }
    }

    // The model pass, last and optional. One request per skill, in parallel: the skills do not
    // share state, and waiting on them one after another is how a workspace review feels broken.
    // A provider that is unreachable leaves a note on that skill rather than an error on the run —
    // the static half already produced something worth reading. The same holds when a reply never
    // parses after retry: the failure becomes a note on that skill, and the report — with every
    // static finding and the JSON envelope a `--format json` caller is promised — survives (#65).
    if passes.model && config.llm.is_configured() {
        enum ModelOutcome {
            Ok(Vec<Message>, Vec<String>),
            Soft(String),
            Hard(anyhow::Error),
        }

        let outcomes: Vec<ModelOutcome> = skills
            .par_iter()
            .map(|one| match review(one, config) {
                Ok((messages, notes)) => ModelOutcome::Ok(messages, notes),
                Err(failure) if llm::is_unparseable_findings(&failure) => {
                    ModelOutcome::Hard(failure)
                }
                // The whole chain, and what to do about it. "asking openrouter::…" on its own
                // tells the reader that something failed and nothing about which part.
                Err(failure) => ModelOutcome::Soft(model_failure(&config.llm, &failure)),
            })
            .collect();

        for (report, outcome) in per_skill.iter_mut().zip(outcomes) {
            match outcome {
                ModelOutcome::Ok(messages, notes) => {
                    report.messages.extend(messages);
                    report.notes.extend(notes);
                }
                ModelOutcome::Soft(note) => report.notes.push(note),
                ModelOutcome::Hard(failure) => {
                    report.notes.push(format!(
                        "The model pass degraded for this skill — only the static rules ran. {failure:#}"
                    ));
                }
            }
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
        // Every report carries it, because a consumer that renders one skill at a time would
        // otherwise read an empty notes list as "the model pass ran and found nothing".
        let count = llm::rules::all().len();

        for report in &mut per_skill {
            report.notes.push(format!(
                "Skipped {count} model rules (not requested). Pass --llm to run them."
            ));
        }
    }

    // Anything the document itself asked not to hear about — scoped to that
    // document: a comment in SKILL.md does not reach into the files bundled
    // beside it. A directive that silenced nothing is reported the way eslint
    // reports unused disable directives, because the author believes
    // something is suppressed that is not.
    for (report, one) in per_skill.iter_mut().zip(skills.iter()) {
        let suppressions = Suppressions::read(&one.source);
        let unused = suppressions.unused(&one.document, &report.messages);

        report
            .messages
            .retain(|message| suppressions.allows(&one.document, message));

        if let Some(severity) = config.severity_for(UNUSED_SUPPRESSION_RULE, Severity::Warning) {
            for found in unused {
                report
                    .messages
                    .push(unused_suppression_message(&found, &one.document, severity));
            }
        }
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
        notes: discovery.skipped,
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
    use std::sync::Mutex;

    /// Serializes tests that mutate the process environment.
    ///
    /// `std::env::set_var` and `remove_var` change state for the whole process, and the default
    /// test harness runs tests in parallel (one thread per core). Any test that touches the
    /// environment must hold this lock for its entire body (issue #114).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Regression for https://github.com/MaximeGaudin/slint/issues/122 —
    /// dogfooding: the skills slint ships in its own repository must pass slint
    /// itself at error severity, because any error finding makes a run exit 1.
    #[test]
    fn the_bundled_skills_slint_ships_pass_slint_itself() {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cursor/skills")
            .canonicalize()
            .expect("the repository's bundled .cursor/skills directory exists");

        let report = run(&[bundled], &Config::default(), &[], Passes::default()).unwrap();

        let errors: Vec<String> = report
            .skills
            .iter()
            .flat_map(|one| one.messages.iter())
            .filter(|message| message.severity == Severity::Error)
            .map(|message| format!("{}: {} ({})", message.file, message.rule, message.message))
            .collect();

        assert!(
            errors.is_empty(),
            "slint's own bundled skills fail slint itself: {errors:#?}"
        );
    }

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
    fn the_model_pass_runs_one_bounded_request_per_skill() {
        use crate::llm::mock::MockServer;
        use std::sync::atomic::Ordering;
        use std::time::{SystemTime, UNIX_EPOCH};

        let server = MockServer::start(MockServer::ollama_reply(), false);
        let temporary = tempfile::tempdir().unwrap();
        // A salt keeps this run's prompts out of the reply cache earlier runs may have warmed:
        // every request here must really reach the mock, not an entry on disk.
        let salt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for index in 0..6 {
            let body = GOOD
                .replace("photo-culling", &format!("skill-{index}"))
                .replace(
                    "Import the RAW files.",
                    &format!("Import shoot {salt}-{index}."),
                );
            write_skill(temporary.path(), &format!("skill-{index}"), &body);
        }

        let mut config = Config::default();
        config.llm.provider = crate::config::Provider::Ollama;
        config.llm.model = "llama3.2".into();
        config.llm.base_url = Some(format!("http://{}/v1", server.address));
        config.llm.max_concurrent_requests = 2;

        let report = run(
            &[temporary.path().to_path_buf()],
            &config,
            &[],
            Passes {
                plugins: false,
                model: true,
            },
        )
        .expect("every skill reviews clean against the mock provider");

        assert_eq!(server.requests.load(Ordering::SeqCst), 6);
        let observed = server.max_in_flight.load(Ordering::SeqCst);
        assert!(
            observed <= 2,
            "the model pass must hold requests back, not let {observed} run at once"
        );
        assert!(
            report
                .skills
                .iter()
                .all(|one| !one.notes.iter().any(|note| note.contains("asking"))),
            "no skill may carry a provider failure note: {:?}",
            report
                .skills
                .iter()
                .flat_map(|one| &one.notes)
                .collect::<Vec<_>>()
        );
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
    fn a_suppression_only_applies_to_the_document_it_was_written_in() {
        let suppressions = Suppressions::read("<!-- slint-disable body/posix-paths -->\n");
        let mut here = message_with("body/posix-paths", 9);
        here.file = "skills/demo/SKILL.md".into();
        let mut there = message_with("body/posix-paths", 3);
        there.file = "skills/demo/references/big.md".into();

        // `allows` is always asked about the document the comment is written in.
        assert!(!suppressions.allows("skills/demo/SKILL.md", &here));
        assert!(suppressions.allows("skills/demo/SKILL.md", &there));
    }

    #[test]
    fn unused_names_every_directive_that_no_finding_ever_matched() {
        let suppressions = Suppressions::read(
            "one\n<!-- slint-disable-next-line body/posix-paths -->\ntwo\n<!-- slint-disable name/not-generic -->\n",
        );
        let mut fired = message_with("body/posix-paths", 3);
        fired.file = "skills/demo/SKILL.md".into();

        assert_eq!(
            suppressions.unused("skills/demo/SKILL.md", &[fired]),
            vec![UnusedSuppression {
                rule: "name/not-generic".into(),
                line: 4,
                next_line: false,
            }]
        );
    }

    fn message_with(rule: &str, line: usize) -> Message {
        Message {
            rule: rule.to_string(),
            severity: Severity::Warning,
            message: "something".into(),
            advice: "do something".into(),
            location: Location::at(line, 1),
            source: Source::Static,
            file: "skills/demo/SKILL.md".into(),
            fix: None,
            reference: Reference {
                title: "t".into(),
                url: "https://example.com".into(),
            },
            confidence: 1.0,
        }
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

    /// Regression for https://github.com/MaximeGaudin/slint/issues/66 — a skill whose directory
    /// name is a string prefix of a sibling's (`a` before `ab`) must not swallow the sibling's
    /// project-rule findings; each report hears only about its own document.
    #[test]
    fn a_project_finding_is_not_stolen_by_a_prefix_sibling() {
        let temporary = tempfile::tempdir().unwrap();
        let shared = "---\nname: {name}\ndescription: Culls a photo shoot in Lightroom by flagging keepers. Use when triaging RAW files after a {tail}.\n---\n\n## Culling\n\n1. Import.\n";

        write_skill(
            temporary.path(),
            "a",
            &shared.replace("{name}", "a").replace("{tail}", "session"),
        );
        write_skill(
            temporary.path(),
            "ab",
            &shared.replace("{name}", "ab").replace("{tail}", "shoot"),
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

        for skill in &report.skills {
            let about_self = skill
                .messages
                .iter()
                .filter(|one| {
                    one.rule == "project/distinct-descriptions"
                        && one.file == format!("{}/SKILL.md", skill.path)
                })
                .count();

            assert_eq!(
                about_self, 1,
                "{} should hear exactly once about its own description, got {:?}",
                skill.name, skill.messages
            );
            assert!(
                !skill
                    .messages
                    .iter()
                    .any(|one| one.rule == "project/distinct-descriptions"
                        && one.file != format!("{}/SKILL.md", skill.path)),
                "{} was handed a finding about another skill",
                skill.name
            );
        }
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/104 — the skip note is a fact
    /// about the run, so every skill's report carries it rather than one arbitrary skill.
    #[test]
    fn every_skill_says_when_the_model_pass_was_skipped() {
        let temporary = tempfile::tempdir().unwrap();
        let shared = GOOD.replace("photo-culling", "{name}");

        for name in ["alpha", "beta", "gamma"] {
            write_skill(temporary.path(), name, &shared.replace("{name}", name));
        }

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

        assert_eq!(report.skills.len(), 3);

        for skill in &report.skills {
            assert!(
                skill
                    .notes
                    .iter()
                    .any(|note| note.contains("Skipped") && note.contains("model rules")),
                "{} should carry the skip note, got {:?}",
                skill.name,
                skill.notes
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
        // Issue #114: the test harness runs tests in parallel, so any test that mutates the
        // process environment must hold ENV_LOCK for its entire body — without the lock another
        // test could observe the environment mid-mutation, and the libc calls would race.
        let _environment = ENV_LOCK.lock().unwrap();

        // SAFETY: ENV_LOCK serializes every environment mutation in this test binary, so no
        // other test thread is mutating the environment concurrently. The variable has a name
        // unique to this test and is read only by the model_failure call below.
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

    /// Regression for https://github.com/MaximeGaudin/slint/issues/65 — a model reply that never
    /// parses after retry degrades to a note on that skill; the static findings of every skill and
    /// the report itself (which the JSON format turns into the envelope) must survive.
    #[test]
    fn an_unparseable_model_reply_degrades_to_a_note_and_keeps_every_static_finding() {
        let temporary = tempfile::tempdir().unwrap();
        write_skill(
            temporary.path(),
            "hardfail",
            "---\nname: hardfail\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\nRead scripts\\notes.md.\n",
        );
        write_skill(temporary.path(), "clean", GOOD);

        let mut config = Config::default();
        config.llm = crate::config::LlmConfig {
            provider: crate::config::Provider::Openai,
            model: "gpt-mock".into(),
            api_key_env: Some("SLINT_TEST_MOCK_KEY".into()),
            ..crate::config::LlmConfig::default()
        };

        let report = run_with_reviewer(
            &[temporary.path().to_path_buf()],
            &config,
            &[],
            Passes {
                plugins: false,
                model: true,
            },
            &|skill, _config| {
                if skill.name == "hardfail" {
                    Err(crate::llm::UnparseableFindings {
                        detail: "The model reply was not valid findings JSON after one retry."
                            .into(),
                    }
                    .into())
                } else {
                    Ok((Vec::new(), Vec::new()))
                }
            },
        )
        .expect("a model-pass failure must not discard a fully computed report");

        assert_eq!(report.skills.len(), 2, "every skill stays in the report");

        let hardfail = report
            .skills
            .iter()
            .find(|one| one.name == "hardfail")
            .unwrap();

        assert!(
            hardfail
                .messages
                .iter()
                .any(|one| one.rule == "body/posix-paths"),
            "the static finding computed before the model failed must survive: {:?}",
            hardfail.messages
        );

        let note = hardfail.notes.join(" ");
        assert!(note.contains("degraded"), "{note}");
        assert!(
            note.contains("not valid findings JSON"),
            "the note must say why the model pass produced nothing: {note}"
        );

        let clean = report
            .skills
            .iter()
            .find(|one| one.path.ends_with("clean"))
            .unwrap();

        assert!(
            clean.notes.is_empty(),
            "one skill's model failure must not leak onto the others: {:?}",
            clean.notes
        );
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
            long.push_str(&format!(
                "\n## Section {index}\n\nSome prose about step {index} of the workflow.\n"
            ));
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
            report.skills[0]
                .messages
                .iter()
                .any(|one| one.rule == "bundle/contents-list"
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
        assert!(
            unused.message.contains("body/posix-path"),
            "{:?}",
            unused.message
        );
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
