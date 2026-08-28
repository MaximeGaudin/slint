//! Rules about the instructions themselves: their size, their paths, and what they must not carry.

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

use crate::diagnostics::{Fix, Location, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

static WINDOWS_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\w.-]+\\[\w.-]+").expect("the windows path pattern compiles"));

static ABSOLUTE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s`(\x22'])((?:/(?:Users|home|var|opt|tmp)/|~/)[\w./-]+)")
        .expect("the absolute path pattern compiles")
});

static TIME_SENSITIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(as of \d{4}|before (january|february|march|april|may|june|july|august|september|october|november|december|\d{4})|after (january|february|march|april|may|june|july|august|september|october|november|december|\d{4})|at the time of writing|for now,|until further notice)",
    )
    .expect("the time pattern compiles")
});

/// Credential shapes, each anchored on something that is a credential and nothing else.
pub static SECRETS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        (
            "a private key block",
            Regex::new(r"-----BEGIN (RSA |EC |OPENSSH |PGP )?PRIVATE KEY").unwrap(),
        ),
        (
            "an OpenAI-style key",
            Regex::new(r"\bsk-[A-Za-z0-9]{20,}").unwrap(),
        ),
        (
            "a GitHub token",
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}").unwrap(),
        ),
        (
            "an AWS access key id",
            Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
        ),
        (
            "a Slack token",
            Regex::new(r"\bxox[abpsr]-[A-Za-z0-9-]{10,}").unwrap(),
        ),
        (
            "a Google API key",
            Regex::new(r"\bAIza[0-9A-Za-z_-]{35}\b").unwrap(),
        ),
        (
            "an assigned password",
            Regex::new(r#"(?i)\b(password|passwd|secret)\s*[:=]\s*["'][^"']{6,}"#).unwrap(),
        ),
    ]
});

#[derive(Debug, Deserialize)]
#[serde(default)]
struct LineOptions {
    /// Past this, an agent starts skimming rather than reading.
    max: usize,
}

impl Default for LineOptions {
    fn default() -> Self {
        LineOptions { max: 500 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct TokenOptions {
    /// The published guidance for a skill body.
    max: usize,
}

impl Default for TokenOptions {
    fn default() -> Self {
        TokenOptions { max: 5_000 }
    }
}

static NOT_EMPTY: RuleMeta = RuleMeta {
    name: "body/not-empty",
    summary: "SKILL.md must contain real instructions, not only headings.",
    rationale: "After the skill is selected, the body is what the agent follows. An empty section gives it nothing to do.",
    advice: "Write the steps. If detail lives in other files, say which files to read and when.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::OVERVIEW.0,
    reference_url: sources::OVERVIEW.1,
};

static MAX_LINES: RuleMeta = RuleMeta {
    name: "body/max-lines",
    summary: "Keep SKILL.md short enough to read end-to-end (about 500 lines max).",
    rationale: "Past roughly 500 lines agents start skimming. Put detail in linked files; keep SKILL.md as the overview.",
    advice: "Move detailed sections into files next to SKILL.md and link them by path. Leave an overview in the body.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static TOKEN_BUDGET: RuleMeta = RuleMeta {
    name: "body/token-budget",
    summary: "Keep the body within a reasonable token budget (~5000 tokens).",
    rationale: "The whole body is loaded when the skill is selected, on top of the conversation and tools. Oversized bodies crowd out useful context.",
    advice: "Move detail into referenced files that load only when needed, instead of on every activation.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static POSIX_PATHS: RuleMeta = RuleMeta {
    name: "body/posix-paths",
    summary: "Use forward slashes in paths (scripts/foo.py), not backslashes.",
    rationale: "Backslash paths break outside Windows — including in most agent sandboxes that unpack the skill.",
    advice: "Replace \\ with / in every bundled path.",
    default_severity: Severity::Warning,
    fixable: true,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static RELATIVE_PATHS: RuleMeta = RuleMeta {
    name: "body/relative-paths",
    summary: "Paths must be relative to the skill folder, not absolute machine paths.",
    rationale: "Paths like /Users/... or ~/... only exist on the author's machine. Agents unpack the skill elsewhere.",
    advice: "Rewrite as a path relative to the skill directory (example: scripts/run.py).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static NO_SECRET: RuleMeta = RuleMeta {
    name: "body/no-secret",
    summary: "Do not put API keys, passwords, or other secrets in the skill.",
    rationale: "Skills are loaded into agent context and often shared. A secret in the file is exposed to every session that fetches it.",
    advice: "Remove the secret from the file and rotate it (it may already have been exposed). Tell the agent to read credentials from the environment instead.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::ENGINEERING.0,
    reference_url: sources::ENGINEERING.1,
};

static NO_TIME_BOMB: RuleMeta = RuleMeta {
    name: "body/no-time-bomb",
    summary: "Do not make instructions depend on calendar dates.",
    rationale: "Date-based branches are wrong on one side of the date, and the agent cannot reliably know which side it is on.",
    advice: "Remove the date condition. If old behavior still matters, document it under an \"Older versions\" section instead of an if-date branch.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static UNDECLARED_TOOL: RuleMeta = RuleMeta {
    name: "body/undeclared-tool",
    summary: "Host-specific tools in the body must be declared in allowed-tools or given a portable fallback.",
    rationale: "Hard-requiring a product-private tool (for example Cursor's AskQuestion) without listing it or providing a chat fallback makes the skill stall or invent fake tool calls on other hosts.",
    advice: "Add the tool to frontmatter allowed-tools, or rewrite the step in host-agnostic terms with a fallback when the tool is missing (for example: ask the same questions conversationally).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

/// Known host-private tools that are not portable across Agent Skills runtimes.
const HOST_SPECIFIC_TOOLS: [&str; 1] = ["AskQuestion"];

static HARDCODED_REPO_PATH: RuleMeta = RuleMeta {
    name: "body/hardcoded-repo-path",
    summary: "Do not hard-require consumer-repo paths without a missing-path fallback.",
    rationale: "Skills move across repos. A fixed docs/ or src/ layout works in one tree and fails elsewhere unless the skill discovers, asks, parameterizes, or stops.",
    advice: "Keep the default layout as the happy path, then add a Prerequisites gate: if the path is missing, ask the user, accept an override, mark it optional, or stop and explain the expected layout.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static IMPERATIVE_INSTRUCTIONS: RuleMeta = RuleMeta {
    name: "body/imperative-instructions",
    summary: "Write instructional steps as direct orders, not conversational or passive advice.",
    rationale: "Soft phrasing (\"you might\", \"consider\", \"feel free\") lets weaker models skip or paraphrase steps. Imperative procedures keep runs consistent across hosts and model tiers.",
    advice: "Rewrite as an imperative procedure (\"Check X\", \"List Y\", \"Write Z\"). Avoid \"you might\", \"consider\", \"feel free\", \"it would be helpful\", and stacked hedges like \"generally usually ideally\".",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

struct NotEmpty;
struct MaxLines;
struct TokenBudget;
struct PosixPaths;
struct RelativePaths;
struct NoSecret;
struct NoTimeBomb;
struct UndeclaredTool;
struct HardcodedRepoPath;
struct ImperativeInstructions;

impl Rule for NotEmpty {
    fn meta(&self) -> &'static RuleMeta {
        &NOT_EMPTY
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let instructions: String = context
            .skill
            .body
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_string();

        if instructions.is_empty() {
            let line = context.skill.frontmatter_lines + 1;
            context.report(
                "There is nothing here to follow",
                Location::at(line.max(1), 1),
            );
        }
    }
}

impl Rule for MaxLines {
    fn meta(&self) -> &'static RuleMeta {
        &MAX_LINES
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let options: LineOptions = context.option();
        let lines = context.skill.body.lines().count();

        if lines > options.max {
            context.report(
                format!(
                    "The body is {lines} lines, over the {} it should stay under",
                    options.max
                ),
                Location::at(context.skill.document_line(1), 1),
            );
        }
    }
}

/// Roughly what a tokeniser would say, without shipping one.
///
/// Four bytes to a token is the ratio every published estimate uses for English prose and it is
/// within a few percent on markdown. A real tokeniser would be exact for one vendor's models and
/// wrong for the next, at the cost of a dependency and a megabyte of vocabulary — and this rule
/// only ever needs to know whether a body is near a threshold, not what it costs to the token.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

impl Rule for TokenBudget {
    fn meta(&self) -> &'static RuleMeta {
        &TOKEN_BUDGET
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let options: TokenOptions = context.option();
        let estimate = estimate_tokens(&context.skill.body);

        if estimate > options.max {
            context.report(
                format!(
                    "The body is roughly {estimate} tokens, over the {} it should stay under",
                    options.max
                ),
                Location::at(context.skill.document_line(1), 1),
            );
        }
    }
}

impl Rule for PosixPaths {
    fn meta(&self) -> &'static RuleMeta {
        &POSIX_PATHS
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let source = context.skill.source.clone();

        for (index, line) in context.skill.body.lines().enumerate() {
            if line.contains("http") {
                continue;
            }

            let Some(found) = WINDOWS_PATH.find(line) else {
                continue;
            };

            let document_line = context.skill.document_line(index + 1);
            let location = Location::span(document_line, found.start() + 1, found.len());

            // The whole document, with every separator normalised: two backslashes on one line are
            // one problem, and a fix per occurrence would fight itself on the next pass.
            let replacement = WINDOWS_PATH
                .replace_all(&source, |captures: &regex::Captures<'_>| {
                    captures[0].replace('\\', "/")
                })
                .to_string();

            context.report_fixable(
                format!("\"{}\" is a Windows path", found.as_str()),
                location,
                Fix {
                    start: 0,
                    end: source.len(),
                    replacement,
                    description: "Replaces backslash separators with forward slashes throughout."
                        .into(),
                },
            );

            // One finding per line: ten separators on one line is one thing to fix.
        }
    }
}

impl Rule for RelativePaths {
    fn meta(&self) -> &'static RuleMeta {
        &RELATIVE_PATHS
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        for (index, line) in context.skill.body.lines().enumerate() {
            let Some(found) = ABSOLUTE_PATH.captures(line).and_then(|caps| caps.get(1)) else {
                continue;
            };

            let document_line = context.skill.document_line(index + 1);
            context.report(
                format!("\"{}\" only exists on one machine", found.as_str()),
                Location::span(document_line, found.start() + 1, found.len()),
            );
        }
    }
}

/// Values that mark an assignment as author-written documentation rather than a leak: setup
/// instructions routinely show `password="changeme123"` without anyone's real password in it.
static PLACEHOLDER_CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(change[-_ ]?me|your[-_ ]|example[-_ ]?|placeholder|dummy|sample[-_ ]|insert[-_ ]|replace[-_ ]|<[^>]+>|\$\{[^}]*\}|\*{5,}|x{5,})"#,
    )
    .expect("the placeholder credential pattern compiles")
});

/// The format-specific shapes (sk-…, ghp_…, AKIA…) say "real key" on their own, but an assigned
/// password cannot: it is also how documentation writes example values. A match whose value is an
/// obvious placeholder is therefore not reported.
fn is_placeholder_credential(label: &str, matched: &str) -> bool {
    label == "an assigned password" && PLACEHOLDER_CREDENTIAL.is_match(matched)
}

impl Rule for NoSecret {
    fn meta(&self) -> &'static RuleMeta {
        &NO_SECRET
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        for (index, line) in context.skill.body.lines().enumerate() {
            for (label, pattern) in SECRETS.iter() {
                let Some(whole) = pattern.find(line) else {
                    continue;
                };
                if is_placeholder_credential(label, whole.as_str()) {
                    continue;
                }

                let document_line = context.skill.document_line(index + 1);
                // Deliberately no quote of the match: a report that repeats the secret is a
                // second place it leaks from.
                context.report(
                    format!("Line {document_line} looks like {label}"),
                    Location::at(document_line, 1),
                );
            }
        }

        for file in &context.skill.files {
            let Some(text) = &file.text else { continue };

            for (label, pattern) in SECRETS.iter() {
                let Some(whole) = pattern.find(text) else {
                    continue;
                };
                if is_placeholder_credential(label, whole.as_str()) {
                    continue;
                }

                let path = file.path.clone();
                context.report_in_file(
                    &path,
                    format!("{path} contains {label}"),
                    Location::whole_file(),
                );
            }
        }
    }
}

impl Rule for NoTimeBomb {
    fn meta(&self) -> &'static RuleMeta {
        &NO_TIME_BOMB
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        for (index, line) in context.skill.body.lines().enumerate() {
            let Some(found) = TIME_SENSITIVE.find(line) else {
                continue;
            };

            let document_line = context.skill.document_line(index + 1);
            context.report(
                format!("\"{}\" ties this to a date", found.as_str()),
                Location::span(document_line, found.start() + 1, found.len()),
            );
        }
    }
}

impl Rule for UndeclaredTool {
    fn meta(&self) -> &'static RuleMeta {
        &UNDECLARED_TOOL
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let allowed = context
            .skill
            .metadata
            .get("allowed-tools")
            .map(|value| value.as_str())
            .unwrap_or("");
        let body = context.skill.body.as_str();
        let body_lower = body.to_ascii_lowercase();
        let has_fallback = body_lower.contains("not available")
            || body_lower.contains("when available")
            || body_lower.contains("if unavailable")
            || body_lower.contains("ask conversationally")
            || body_lower.contains("ask these questions conversationally")
            || body_lower.contains("otherwise ask");

        if has_fallback {
            return;
        }

        for tool in HOST_SPECIFIC_TOOLS {
            if !body.contains(tool) {
                continue;
            }

            if allowed
                .split_whitespace()
                .any(|token| token.eq_ignore_ascii_case(tool))
            {
                continue;
            }

            for (index, line) in body.lines().enumerate() {
                if !line.contains(tool) {
                    continue;
                }

                let lower = line.to_ascii_lowercase();
                if lower.contains("do not use")
                    || lower.contains("don't use")
                    || lower.contains("never use")
                    || lower.contains("avoid using")
                {
                    continue;
                }

                let document_line = context.skill.document_line(index + 1);
                let column = line.find(tool).map(|offset| offset + 1).unwrap_or(1);
                context.report(
                    format!(
                        "Instructions require tool \"{tool}\" but it is not listed in allowed-tools"
                    ),
                    Location::at(document_line, column),
                );
                break;
            }
        }
    }
}

impl Rule for HardcodedRepoPath {
    fn meta(&self) -> &'static RuleMeta {
        &HARDCODED_REPO_PATH
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        static BACKTICK_PATH: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)`((?:docs|src|app|packages)/[^`]+)`")
                .expect("the backtick consumer path pattern compiles")
        });
        static BARE_PATH: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)\b((?:docs|src|app|packages)/[\w./-]+)")
                .expect("the bare consumer path pattern compiles")
        });

        let body = context.skill.body.as_str();
        let body_lower = body.to_ascii_lowercase();
        let has_fallback = body_lower.contains("does not exist")
            || body_lower.contains("if missing")
            || body_lower.contains("if the path is missing")
            || body_lower.contains("ask the user")
            || body_lower.contains("optional")
            || body_lower.contains("override")
            || body_lower.contains("stop and explain")
            || body_lower.contains("when absent");

        if has_fallback {
            return;
        }

        let mut reported: Vec<String> = Vec::new();

        for (index, line) in body.lines().enumerate() {
            let mut matches: Vec<(usize, String)> = BACKTICK_PATH
                .captures_iter(line)
                .filter_map(|caps| {
                    let whole = caps.get(0)?;
                    let path = caps.get(1)?.as_str().to_string();
                    Some((whole.start(), path))
                })
                .collect();

            if matches.is_empty() {
                matches = BARE_PATH
                    .captures_iter(line)
                    .filter_map(|caps| {
                        let whole = caps.get(0)?;
                        let path = caps.get(1)?.as_str().to_string();
                        Some((whole.start(), path))
                    })
                    .collect();
            }

            for (start, path) in matches {
                let lower = path.to_ascii_lowercase();
                if lower.starts_with("scripts/")
                    || lower.starts_with("references/")
                    || lower.starts_with("assets/")
                    || lower.starts_with("templates/")
                {
                    continue;
                }

                if reported.iter().any(|seen| seen.eq_ignore_ascii_case(&path)) {
                    continue;
                }
                reported.push(path.clone());

                let document_line = context.skill.document_line(index + 1);
                context.report(
                    format!(
                        "Instructions require repository path \"{path}\" with no fallback if it is missing"
                    ),
                    Location::span(document_line, start + 1, path.len()),
                );
            }
        }
    }
}

impl Rule for ImperativeInstructions {
    fn meta(&self) -> &'static RuleMeta {
        &IMPERATIVE_INSTRUCTIONS
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        static SOFT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"(?i)\b(you might(?: want)?|you may want|feel free|consider |try to|it would be|perhaps|when you feel|if you think|as you see fit)\b",
            )
            .expect("the soft-instruction pattern compiles")
        });
        static PASSIVE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r"(?i)\b([A-Za-z][A-Za-z0-9_-]*(?:\s+[A-Za-z][A-Za-z0-9_-]*){0,3}\s+should be\s+(?:[a-z]+ed|done|made|given|taken|written|shown|known|seen|held|kept|left|sent|built|told|met|paid|put|set|cut|run|read|found|lost|meant|said|led|brought|bought|caught|taught|thought|sold|grown|thrown|drawn|spoken|broken|hidden|chosen|driven|forgotten|stolen|understood|won|eaten|frozen|worn|torn|blown|flown|begun|ridden|risen|fallen|shaken|mistaken|bitten|lain|laid|dealt|felt|gotten))\b",
            )
            .expect("the passive-instruction pattern compiles")
        });
        static HEDGE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)\b(generally|usually|ideally)\b").expect("the hedge pattern compiles")
        });

        let mut in_fence = false;

        for (index, line) in context.skill.body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence || trimmed.starts_with('#') {
                continue;
            }

            let document_line = context.skill.document_line(index + 1);

            if let Some(found) = SOFT.find(line) {
                context.report(
                    format!(
                        "\"{}\" is conversational instead of a direct order",
                        found.as_str()
                    ),
                    Location::span(document_line, found.start() + 1, found.len()),
                );
                continue;
            }

            if let Some(found) = PASSIVE.find(line) {
                context.report(
                    format!(
                        "\"{}\" is passive instead of a direct order",
                        found.as_str()
                    ),
                    Location::span(document_line, found.start() + 1, found.len()),
                );
                continue;
            }

            let hedges: Vec<_> = HEDGE.find_iter(line).collect();
            if hedges.len() >= 3 {
                let first = hedges[0];
                context.report(
                    format!(
                        "\"{}\" stacks hedges instead of giving a direct order",
                        first.as_str()
                    ),
                    Location::span(document_line, first.start() + 1, first.len()),
                );
            }
        }
    }
}

static NOT_EMPTY_RULE: NotEmpty = NotEmpty;
static MAX_LINES_RULE: MaxLines = MaxLines;
static TOKEN_RULE: TokenBudget = TokenBudget;
static POSIX_RULE: PosixPaths = PosixPaths;
static RELATIVE_RULE: RelativePaths = RelativePaths;
static SECRET_RULE: NoSecret = NoSecret;
static TIME_RULE: NoTimeBomb = NoTimeBomb;
static UNDECLARED_TOOL_RULE: UndeclaredTool = UndeclaredTool;
static HARDCODED_REPO_PATH_RULE: HardcodedRepoPath = HardcodedRepoPath;
static IMPERATIVE_RULE: ImperativeInstructions = ImperativeInstructions;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![
        &NOT_EMPTY_RULE,
        &MAX_LINES_RULE,
        &TOKEN_RULE,
        &POSIX_RULE,
        &RELATIVE_RULE,
        &SECRET_RULE,
        &TIME_RULE,
        &UNDECLARED_TOOL_RULE,
        &HARDCODED_REPO_PATH_RULE,
        &IMPERATIVE_RULE,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RuleSetting};
    use crate::rules::testing::{check, check_with, good_skill, skill_with_body};

    #[test]
    fn a_good_body_passes_every_rule_here() {
        let skill = good_skill();

        for rule in rules() {
            assert!(
                check(rule, &skill).is_empty(),
                "{} fired on a good body",
                rule.meta().name
            );
        }
    }

    #[test]
    fn a_body_of_headings_and_nothing_else_is_an_error() {
        let messages = check(
            &NOT_EMPTY_RULE,
            &skill_with_body("\n## Culling\n\n### Later\n"),
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
    }

    #[test]
    fn a_long_body_is_reported_against_the_configured_maximum() {
        let body = format!("\n## Culling\n\n{}", "Step.\n".repeat(600));
        let skill = skill_with_body(&body);

        assert_eq!(check(&MAX_LINES_RULE, &skill).len(), 1);

        let mut config = Config::default();
        config.rules.insert(
            "body/max-lines".into(),
            RuleSetting::Tuned(Severity::Warning, serde_json::json!({ "max": 1000 })),
        );

        assert!(check_with(&MAX_LINES_RULE, &skill, &config).is_empty());
    }

    #[test]
    fn a_body_over_the_token_budget_is_reported_against_the_configured_maximum() {
        let body = format!(
            "\n## Culling\n\n{}",
            "A sentence about culling photographs. ".repeat(600)
        );
        let skill = skill_with_body(&body);

        assert_eq!(check(&TOKEN_RULE, &skill).len(), 1);

        let mut config = Config::default();
        config.rules.insert(
            "body/token-budget".into(),
            RuleSetting::Tuned(Severity::Warning, serde_json::json!({ "max": 50_000 })),
        );

        assert!(check_with(&TOKEN_RULE, &skill, &config).is_empty());
    }

    #[test]
    fn the_token_estimate_is_a_quarter_of_the_bytes() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn a_windows_path_is_reported_where_it_is_written() {
        let skill = skill_with_body("\n## Culling\n\n1. Read scripts\\notes.md first.\n");
        let messages = check(&POSIX_RULE, &skill);

        assert_eq!(messages.len(), 1);
        // Frontmatter is four lines here, so the fourth body line is the eighth of the document.
        assert_eq!(messages[0].location.line, skill.document_line(4));
        assert!(messages[0].message.contains("scripts\\notes.md"));
    }

    #[test]
    fn the_windows_path_fix_normalises_every_separator_at_once() {
        let skill = skill_with_body(
            "\n## Culling\n\n1. Read scripts\\notes.md.\n2. Then references\\formats.md.\n",
        );
        let messages = check(&POSIX_RULE, &skill);

        assert_eq!(messages.len(), 2, "one finding per line");

        let fix = messages[0].fix.as_ref().unwrap();
        let mut patched = skill.source.clone();
        patched.replace_range(fix.start..fix.end, &fix.replacement);

        assert!(patched.contains("scripts/notes.md"));
        assert!(patched.contains("references/formats.md"));
        assert!(!patched.contains('\\'));
    }

    #[test]
    fn a_url_is_not_mistaken_for_a_windows_path() {
        let skill =
            skill_with_body("\n## Culling\n\nSee https://example.com/a\\b for the format.\n");
        assert!(check(&POSIX_RULE, &skill).is_empty());
    }

    #[test]
    fn an_absolute_path_is_reported() {
        for line in ["/Users/mgaudin/shoots/raw", "~/shoots/raw"] {
            let skill = skill_with_body(&format!("\n## Culling\n\nImport from {line} first.\n"));
            let messages = check(&RELATIVE_RULE, &skill);

            assert_eq!(messages.len(), 1, "for {line}");
            assert!(messages[0].message.contains(line));
        }
    }

    #[test]
    fn a_relative_path_is_left_alone() {
        let skill = skill_with_body("\n## Culling\n\nRead scripts/notes.md first.\n");
        assert!(check(&RELATIVE_RULE, &skill).is_empty());
    }

    #[test]
    fn a_credential_in_the_body_is_an_error_that_does_not_repeat_the_secret() {
        let skill = skill_with_body(
            "\n## Culling\n\nExport SLACK_TOKEN=xoxb-1234567890abcdef before running.\n",
        );
        let messages = check(&SECRET_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
        assert!(!messages[0].message.contains("xoxb-"));
    }

    #[test]
    fn a_credential_in_a_bundled_file_is_reported_against_that_file() {
        let mut skill = good_skill();
        skill.files.push(crate::skill::BundledFile {
            path: "scripts/cull.py".into(),
            bytes: 40,
            executable: true,
            text: Some("TOKEN = \"ghp_abcdefghijklmnopqrstuvwxyz0123\"\n".into()),
        });

        let messages = check(&SECRET_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].file.ends_with("scripts/cull.py"));
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/95 —
    /// placeholder values in author-written setup instructions are not leaks.
    #[test]
    fn a_placeholder_password_in_the_documentation_is_not_a_leak() {
        for line in [
            "1. In the config file, set `password=\"changeme123\"` before first run.",
            "1. Then set password=\"your-password-here\" in the same file.",
        ] {
            let skill = skill_with_body(&format!("\n## Setup\n\n{line}\n"));
            assert!(check(&SECRET_RULE, &skill).is_empty(), "for {line}");
        }
    }

    #[test]
    fn a_placeholder_password_in_a_bundled_file_is_not_a_leak() {
        let mut skill = good_skill();
        skill.files.push(crate::skill::BundledFile {
            path: "scripts/setup.py".into(),
            bytes: 40,
            executable: true,
            text: Some("password = \"changeme123\"  # default for the demo\n".into()),
        });

        assert!(check(&SECRET_RULE, &skill).is_empty());
    }

    #[test]
    fn a_real_looking_password_assignment_is_still_reported() {
        let skill = skill_with_body("\n## Setup\n\n1. Log in with password=\"Kj9#mPx2vQz\".\n");
        let messages = check(&SECRET_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
    }

    #[test]
    fn a_dated_instruction_is_reported() {
        for line in [
            "As of 2024 the export dialog is under File.",
            "Before March, use the legacy exporter.",
            "At the time of writing this needs the beta build.",
        ] {
            let skill = skill_with_body(&format!("\n## Culling\n\n{line}\n"));
            assert_eq!(check(&TIME_RULE, &skill).len(), 1, "for {line}");
        }
    }

    #[test]
    fn ordinary_prose_about_time_is_not_a_time_bomb() {
        let skill = skill_with_body("\n## Culling\n\nFlag the keepers before exporting them.\n");
        assert!(check(&TIME_RULE, &skill).is_empty());
    }

    #[test]
    fn hardcoded_consumer_repo_paths_without_a_fallback_are_a_warning() {
        let skill = skill_with_body(
            r#"
## Scope

| Path | Role |
|------|------|
| `docs/01 - Briefs/*.md` | Source of product truth. Read only. |
| `docs/Stack.md` | Source of stack truth. Read only. |
| `docs/02 - Specs/` | The only place this skill writes. |

## Steps

1. List `docs/01 - Briefs/` and read every brief.
2. Create `docs/02 - Specs/Index.md`.
"#,
        );
        let messages = check(&HARDCODED_REPO_PATH_RULE, &skill);

        assert!(
            !messages.is_empty(),
            "expected a warning for hardcoded consumer-repo paths"
        );
        assert_eq!(messages[0].severity, Severity::Warning);
        assert_eq!(messages[0].rule, "body/hardcoded-repo-path");
        assert!(
            messages
                .iter()
                .any(|message| message.message.contains("docs/01 - Briefs")),
            "expected the finding to name a required repo path, got {messages:?}"
        );
    }

    #[test]
    fn a_missing_path_fallback_suppresses_the_hardcoded_repo_path_warning() {
        let skill = skill_with_body(
            r#"
## Prerequisites

Default layout: `docs/01 - Briefs/` and `docs/02 - Specs/`.
If `docs/01 - Briefs/` does not exist, ask the user where briefs live, or stop and explain the expected layout.

## Steps

1. List `docs/01 - Briefs/` only after the path is confirmed.
"#,
        );

        assert!(check(&HARDCODED_REPO_PATH_RULE, &skill).is_empty());
    }

    #[test]
    fn skill_bundle_paths_are_not_consumer_repo_paths() {
        let skill = skill_with_body(
            "\n## Culling\n\n1. Read `scripts/cull.py`.\n2. Follow `references/formats.md`.\n3. Copy `assets/template.md`.\n",
        );

        assert!(check(&HARDCODED_REPO_PATH_RULE, &skill).is_empty());
    }

    #[test]
    fn conversational_workflow_steps_are_reported() {
        let skill = skill_with_body(
            "\n## Workflow\n\n\
You might want to start by looking through the briefs folder if you can.\n\
Consider auditing each brief for gaps. It would be helpful to ask clarifying questions.\n\
Feel free to rewrite the brief once you have enough answers.\n",
        );
        let messages = check(&IMPERATIVE_RULE, &skill);

        assert!(
            messages.len() >= 3,
            "expected soft markers to fire, got {messages:?}"
        );
        assert_eq!(messages[0].rule, "body/imperative-instructions");
        assert_eq!(messages[0].severity, Severity::Warning);
        assert!(
            messages
                .iter()
                .any(|m| m.message.to_ascii_lowercase().contains("you might")),
            "{messages:?}"
        );
    }

    #[test]
    fn imperative_workflow_steps_pass() {
        let skill = skill_with_body(
            "\n## Workflow\n\n\
1. List `docs/01 - Briefs/` and read every file in numeric order.\n\
2. For each brief, report a one-line verdict and a blocking-gap count.\n\
3. Ask a batch of 3–6 multiple-choice questions for the current brief.\n\
4. Write the answers into the brief immediately.\n",
        );
        assert!(check(&IMPERATIVE_RULE, &skill).is_empty());
    }

    #[test]
    fn a_passive_procedure_is_reported() {
        let skill = skill_with_body(
            "\n## Workflow\n\n1. Authentication should be checked on every endpoint.\n",
        );
        let messages = check(&IMPERATIVE_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("should be checked"));
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/73 —
    /// "should be <adjective>" is a description, not a passive construction.
    #[test]
    fn a_should_be_with_an_adjective_is_not_reported_as_passive() {
        for line in [
            "The output should be correct before you continue.",
            "The final report should be short.",
        ] {
            let skill = skill_with_body(&format!("\n## Workflow\n\n1. {line}\n"));
            assert!(
                check(&IMPERATIVE_RULE, &skill).is_empty(),
                "\"{line}\" is not passive"
            );
        }
    }

    #[test]
    fn a_passive_procedure_with_an_irregular_participle_is_reported() {
        let skill = skill_with_body(
            "\n## Workflow\n\n1. The result should be written to disk before exiting.\n",
        );
        let messages = check(&IMPERATIVE_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("should be written"));
    }

    #[test]
    fn stacked_hedges_in_one_step_are_reported() {
        let skill = skill_with_body(
            "\n## Workflow\n\n1. Generally usually ideally run the export after culling.\n",
        );
        let messages = check(&IMPERATIVE_RULE, &skill);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("hedge") || messages[0].message.contains("Generally"));
    }

    #[test]
    fn soft_language_inside_a_fenced_example_is_ignored() {
        let skill = skill_with_body(
            "\n## Workflow\n\n\
1. Write the brief.\n\n\
```markdown\n\
You might want to start by looking through the briefs folder.\n\
```\n",
        );
        assert!(check(&IMPERATIVE_RULE, &skill).is_empty());
    }

    #[test]
    fn a_flow_sequence_allowed_tool_is_not_undeclared() {
        // Regression for https://github.com/MaximeGaudin/slint/issues/288: allowed-tools written as
        // a YAML flow sequence `[AskQuestion, Read]` must still be read as declaring AskQuestion.
        let skill = crate::skill::parse(
            "---\nname: grill-brief\ndescription: Interrogates briefs with batched questions until they are spec-ready. Use when grilling briefs.\nallowed-tools: [AskQuestion, Read]\n---\n\n## Grill Brief\n\n1. Ask a batch of questions with the AskQuestion tool.\n",
        );

        assert!(
            check(&UNDECLARED_TOOL_RULE, &skill).is_empty(),
            "expected flow-sequence allowed-tools to declare AskQuestion"
        );
    }
}
