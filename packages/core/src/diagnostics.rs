//! What a rule produces, and what everything downstream reads.
//!
//! One shape for every finding, whether it came from a regex, from a plugin, or from a model. The
//! reporters, the fixer and the editor integration only ever handle this, which is what keeps a new
//! rule from needing a new code path anywhere else.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Strips terminal control characters from text written outside this tool.
///
/// A finding's text is arbitrary: a model wrote it, a plugin wrote it, or it was captured from the
/// very document being linted — and a reporter prints it to a terminal that will obey its escapes.
/// So the C0/C1 control range (ESC starts an ANSI sequence, CR can reset the cursor) has to come
/// out before the text is stored. Tab and newline are kept: they are the author's own paragraphing
/// and are inert on the terminal.
pub(crate) fn strip_control(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || *character == '\t' || *character == '\n')
        .collect::<String>()
}

/// How much a finding matters. Ordered worst-first, so a sort is a sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The skill is broken, or the specification would refuse it.
    Error,
    /// It will work and behave worse than it should.
    Warning,
    /// A judgement call worth seeing once.
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }

    /// The name used in a config file, where `off` is also a value a rule can take.
    pub fn parse(text: &str) -> Option<Severity> {
        match text {
            "error" | "err" | "2" => Some(Severity::Error),
            "warn" | "warning" | "1" => Some(Severity::Warning),
            "info" | "note" => Some(Severity::Info),
            _ => None,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Where a finding is, in the file a reader would open.
///
/// Lines and columns are 1-based, as every editor and every compiler reports them. `end` is
/// exclusive and optional: a rule that knows only the line says only the line rather than
/// inventing a span that would underline the wrong words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
}

impl Location {
    pub fn at(line: usize, column: usize) -> Self {
        Location {
            line,
            column,
            end_line: None,
            end_column: None,
        }
    }

    pub fn span(line: usize, column: usize, length: usize) -> Self {
        Location {
            line,
            column,
            end_line: Some(line),
            end_column: Some(column + length),
        }
    }

    /// The first line of a document, for a finding about the document as a whole.
    pub fn whole_file() -> Self {
        Location::at(1, 1)
    }
}

/// A replacement for a byte range of the file the finding is in.
///
/// Byte offsets rather than lines, because a fix is applied to text and re-linted afterwards; a
/// line-based patch has to be re-resolved after the fix above it lands, and that is where an
/// autofixer starts corrupting files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// Byte offset into the file's text, inclusive.
    pub start: usize,
    /// Byte offset into the file's text, exclusive.
    pub end: usize,
    pub replacement: String,
    /// One sentence, in the author's terms, on what changed.
    pub description: String,
}

/// Where a rule's claim comes from.
///
/// Not optional, and that is the point: every rule here asserts something about someone else's
/// writing, and an assertion with no source has to be taken on trust. A rule without a citation
/// does not get into the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub title: String,
    pub url: String,
}

/// What produced a finding, which is also how much it can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Read from the text. Exact, free, and always available.
    Static,
    /// Written by a language model. A reading, not a measurement.
    Model,
    /// Produced by a plugin, which may be either.
    Plugin,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// The rule that fired, as it is written in a config file.
    pub rule: String,
    pub severity: Severity,
    /// One line, in the imperative or the indicative — never a scolding.
    pub message: String,
    /// What to do about it. Static text per rule, so it costs nothing to produce.
    pub advice: String,
    pub location: Location,
    pub source: Source,
    /// The document this is about, relative to where slint was run.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    pub reference: Reference,
    /// The model's own estimate, for a finding a model wrote. Always 1.0 for anything computed.
    pub confidence: f32,
}

impl Message {
    pub fn is_fixable(&self) -> bool {
        self.fix.is_some()
    }
}

/// What a file's bytes were when its fixes were computed.
///
/// A fix is a byte range, so it is only as valid as the text it was measured against. The length
/// plus a cheap hash is enough to catch the file that changed between the lint pass and the fix
/// pass — including the same-length rewrite the bounds checks alone would wave straight through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    /// The length of the text, in bytes.
    pub length: usize,
    /// A hash of the text, from the standard library's own hasher.
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(text: &str) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);

        Fingerprint {
            length: text.len(),
            hash: hasher.finish(),
        }
    }

    /// Whether `text` is still the bytes this was taken from.
    pub fn matches(&self, text: &str) -> bool {
        self.length == text.len() && Fingerprint::of(text).hash == self.hash
    }
}

/// Everything found in one skill, plus what could not be looked at.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillReport {
    /// The skill's directory, relative to the run.
    pub path: String,
    pub name: String,
    pub messages: Vec<Message>,
    /// Said out loud rather than inferred from an empty result — a pass that did not run, a file
    /// that could not be read.
    pub notes: Vec<String>,
}

impl SkillReport {
    pub fn count(&self, severity: Severity) -> usize {
        self.messages
            .iter()
            .filter(|one| one.severity == severity)
            .count()
    }
}

/// Every skill looked at in one run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub skills: Vec<SkillReport>,
    /// Fixes written to disk during this run, when `--fix` was given.
    pub fixed: usize,
    /// About the run as a whole rather than one skill: arguments that were skipped, a discovery
    /// that came up empty. Said out loud rather than inferred from an empty result.
    #[serde(default)]
    pub notes: Vec<String>,
    /// What the bytes of each fixable file were when its fixes were computed, keyed by the path
    /// the fixes name. Only meaningful inside the process that computed them — `--fix` reads the
    /// file again before applying — so it never reaches the JSON envelope.
    #[serde(skip)]
    pub fingerprints: BTreeMap<String, Fingerprint>,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.skills
            .iter()
            .map(|one| one.count(Severity::Error))
            .sum()
    }

    pub fn warnings(&self) -> usize {
        self.skills
            .iter()
            .map(|one| one.count(Severity::Warning))
            .sum()
    }

    pub fn infos(&self) -> usize {
        self.skills
            .iter()
            .map(|one| one.count(Severity::Info))
            .sum()
    }

    pub fn total(&self) -> usize {
        self.skills.iter().map(|one| one.messages.len()).sum()
    }

    pub fn fixable(&self) -> usize {
        self.skills
            .iter()
            .flat_map(|one| one.messages.iter())
            .filter(|one| one.is_fixable())
            .count()
    }

    /// Whether the run survives the caller's warning budget: anything at error severity fails
    /// it, and so does going over `--max-warnings`. The exit code and the JSON envelope's `ok`
    /// flag both read this, so the two verdicts a caller can see cannot disagree.
    pub fn passes(&self, max_warnings: i64) -> bool {
        if self.errors() > 0 {
            return false;
        }

        !(max_warnings >= 0 && self.warnings() as i64 > max_warnings)
    }

    /// Worst first, then by file, then by position: the order a list of problems is read in.
    pub fn sorted(mut self) -> Self {
        for skill in &mut self.skills {
            skill.messages.sort_by(|a, b| {
                a.severity
                    .cmp(&b.severity)
                    .then(a.file.cmp(&b.file))
                    .then(a.location.line.cmp(&b.location.line))
                    .then(a.location.column.cmp(&b.location.column))
                    .then(a.rule.cmp(&b.rule))
            });
        }

        self.skills.sort_by(|a, b| a.path.cmp(&b.path));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(rule: &str, severity: Severity, line: usize) -> Message {
        Message {
            rule: rule.to_string(),
            severity,
            message: "something".into(),
            advice: "do something".into(),
            location: Location::at(line, 1),
            source: Source::Static,
            file: "SKILL.md".into(),
            fix: None,
            reference: Reference {
                title: "t".into(),
                url: "https://example.com".into(),
            },
            confidence: 1.0,
        }
    }

    #[test]
    fn severity_orders_worst_first() {
        let mut all = vec![Severity::Info, Severity::Error, Severity::Warning];
        all.sort();
        assert_eq!(
            all,
            vec![Severity::Error, Severity::Warning, Severity::Info]
        );
    }

    #[test]
    fn severity_parses_the_names_a_config_may_use() {
        assert_eq!(Severity::parse("error"), Some(Severity::Error));
        assert_eq!(Severity::parse("warn"), Some(Severity::Warning));
        assert_eq!(Severity::parse("warning"), Some(Severity::Warning));
        assert_eq!(Severity::parse("info"), Some(Severity::Info));
        assert_eq!(Severity::parse("2"), Some(Severity::Error));
        assert_eq!(Severity::parse("off"), None);
        assert_eq!(Severity::parse("nonsense"), None);
    }

    #[test]
    fn a_span_ends_where_the_match_does() {
        let span = Location::span(4, 7, 12);
        assert_eq!(span.end_line, Some(4));
        assert_eq!(span.end_column, Some(19));
    }

    #[test]
    fn counts_are_per_severity() {
        let report = Report {
            skills: vec![SkillReport {
                path: "skills/a".into(),
                name: "a".into(),
                messages: vec![
                    message("one", Severity::Error, 1),
                    message("two", Severity::Warning, 2),
                    message("three", Severity::Warning, 3),
                    message("four", Severity::Info, 4),
                ],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
            fingerprints: BTreeMap::new(),
        };

        assert_eq!(report.errors(), 1);
        assert_eq!(report.warnings(), 2);
        assert_eq!(report.infos(), 1);
        assert_eq!(report.total(), 4);
    }

    #[test]
    fn sorting_puts_the_worst_first_then_the_earliest() {
        let report = Report {
            skills: vec![SkillReport {
                path: "skills/a".into(),
                name: "a".into(),
                messages: vec![
                    message("info-late", Severity::Info, 9),
                    message("error-late", Severity::Error, 8),
                    message("warn-early", Severity::Warning, 1),
                ],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
            fingerprints: BTreeMap::new(),
        }
        .sorted();

        let order: Vec<&str> = report.skills[0]
            .messages
            .iter()
            .map(|one| one.rule.as_str())
            .collect();
        assert_eq!(order, vec!["error-late", "warn-early", "info-late"]);
    }

    #[test]
    fn fixable_counts_only_what_carries_a_fix() {
        let mut fixable = message("fixable", Severity::Warning, 1);
        fixable.fix = Some(Fix {
            start: 0,
            end: 1,
            replacement: "x".into(),
            description: "d".into(),
        });

        let report = Report {
            skills: vec![SkillReport {
                path: "skills/a".into(),
                name: "a".into(),
                messages: vec![fixable, message("plain", Severity::Warning, 2)],
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
            fingerprints: BTreeMap::new(),
        };

        assert_eq!(report.fixable(), 1);
    }
}
