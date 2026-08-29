//! Printing a report.
//!
//! Four formats, because four different readers exist: a person at a terminal, a CI annotation
//! service, an editor, and another agent. They are all the same data, and none of them is the
//! "real" one — the JSON is generated from the same report the terminal output is.

pub mod github;
pub mod json;
pub mod junit;
pub mod sarif;
pub mod stylish;

use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::diagnostics::{Report, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// For a person: grouped, coloured, with the citation under each finding.
    #[default]
    Stylish,
    /// For a machine: an envelope with an `ok` flag, so a caller can branch before parsing.
    Json,
    /// For CI: workflow commands GitHub turns into annotations on the diff.
    Github,
    /// For scanners and quality dashboards: SARIF 2.1.0, one result per finding.
    Sarif,
    /// For CI test reports: JUnit XML, one testcase per finding.
    Junit,
    /// One line per finding, for grep and for editors that expect a compiler.
    Compact,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "stylish" => Ok(Format::Stylish),
            "json" => Ok(Format::Json),
            "github" => Ok(Format::Github),
            "sarif" => Ok(Format::Sarif),
            "junit" => Ok(Format::Junit),
            "compact" => Ok(Format::Compact),
            other => Err(format!(
                "unknown format \"{other}\" — try stylish, json, github, sarif, junit or compact"
            )),
        }
    }
}

/// Renders a report in the chosen format. The warning budget only matters to the JSON envelope:
/// it is what the exit code is judged against, and `ok` says the same thing.
pub fn render(report: &Report, format: Format, colour: bool, max_warnings: i64) -> String {
    match format {
        Format::Stylish => stylish::render(report, colour),
        Format::Json => json::render(report, max_warnings),
        Format::Github => github::render(report),
        Format::Sarif => sarif::render(report),
        Format::Junit => junit::render(report),
        Format::Compact => compact(report),
    }
}

fn compact(report: &Report) -> String {
    let mut lines = Vec::new();

    for skill in &report.skills {
        for message in &skill.messages {
            // The ESLint compact convention, which the name promises: the position in prose,
            // a capitalised severity, and the rule in brackets at the end of the line.
            lines.push(format!(
                "{}: line {}, col {}, {} - {} ({})",
                message.file,
                message.location.line,
                message.location.column,
                severity_word(message.severity),
                message.message,
                message.rule
            ));
        }

        // A note is not a finding, but it is the only record that a pass did not run, so it
        // prints under the skill's path rather than being read out of the JSON alone.
        for note in &skill.notes {
            lines.push(format!("{}: note: {}", skill.path, note));
        }
    }

    for note in &report.notes {
        lines.push(format!("note: {note}"));
    }

    lines.join("\n")
}

/// ESLint's compact format capitalises the severity; an `info` is a `note` here, the word every
/// other format uses for a judgement call.
fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
        Severity::Info => "Note",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};
    use std::collections::BTreeMap;

    pub fn sample() -> Report {
        Report {
            skills: vec![SkillReport {
                path: "skills/helper".into(),
                name: "helper".into(),
                messages: vec![
                    Message {
                        rule: "name/not-generic".into(),
                        severity: Severity::Warning,
                        message: "\"helper\" says nothing about what this does".into(),
                        advice: "Rename it after what it does.".into(),
                        location: Location::at(2, 1),
                        source: Source::Static,
                        file: "skills/helper/SKILL.md".into(),
                        fix: None,
                        reference: Reference {
                            title: "Skill authoring best practices".into(),
                            url: "https://example.com/best-practices".into(),
                        },
                        confidence: 1.0,
                    },
                    Message {
                        rule: "bundle/no-dangling-path".into(),
                        severity: Severity::Error,
                        message:
                            "The instructions name scripts/cull.py, which is not in the bundle"
                                .into(),
                        advice: "Add the file, or take the reference out.".into(),
                        location: Location::at(9, 1),
                        source: Source::Static,
                        file: "skills/helper/SKILL.md".into(),
                        fix: None,
                        reference: Reference {
                            title: "The AgentSkill specification".into(),
                            url: "https://example.com/spec".into(),
                        },
                        confidence: 1.0,
                    },
                ],
                notes: vec!["8 rules need a model and none is configured.".into()],
            }],
            fixed: 0,
            notes: Vec::new(),
            fingerprints: BTreeMap::new(),
        }
    }

    #[test]
    fn every_format_has_a_name_that_can_be_typed() {
        assert_eq!("stylish".parse::<Format>().unwrap(), Format::Stylish);
        assert_eq!("json".parse::<Format>().unwrap(), Format::Json);
        assert_eq!("github".parse::<Format>().unwrap(), Format::Github);
        assert_eq!("sarif".parse::<Format>().unwrap(), Format::Sarif);
        assert_eq!("junit".parse::<Format>().unwrap(), Format::Junit);
        assert_eq!("compact".parse::<Format>().unwrap(), Format::Compact);

        let failure = "yaml".parse::<Format>().unwrap_err();
        assert!(
            failure.contains("stylish"),
            "the error lists what does work: {failure}"
        );
    }

    // https://github.com/MaximeGaudin/slint/issues/177: the name "compact" is ESLint's, and the
    // shape has to be ESLint's too: `path: line X, col Y, Severity - message (rule)`.
    #[test]
    fn compact_prints_one_grep_friendly_line_per_finding() {
        let mut report = sample();
        report.skills[0].notes.clear();
        let text = compact(&report);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "skills/helper/SKILL.md: line 2, col 1, Warning - \"helper\" says nothing about what this does (name/not-generic)"
        );
        assert_eq!(
            lines[1],
            "skills/helper/SKILL.md: line 9, col 1, Error - The instructions name scripts/cull.py, which is not in the bundle (bundle/no-dangling-path)"
        );
    }

    #[test]
    fn an_info_finding_reads_as_a_note_in_compact() {
        let mut report = sample();
        report.skills[0].notes.clear();
        report.skills[0].messages[0].severity = Severity::Info;

        let text = compact(&report);

        assert!(text.contains(", Note - "), "{text}");
    }

    // https://github.com/MaximeGaudin/slint/issues/134: a note says a pass did not run, and a
    // format that drops it makes a partial run read as a complete one.
    #[test]
    fn notes_survive_into_compact_so_a_skipped_pass_stays_visible() {
        let mut report = sample();
        report.notes = vec!["1 argument was not a skill and was skipped.".into()];

        let text = compact(&report);

        assert!(
            text.contains("skills/helper: note: 8 rules need a model and none is configured."),
            "the skill's own notes print under its path: {text}"
        );
        assert!(
            text.contains("note: 1 argument was not a skill and was skipped."),
            "the run's notes print without a path: {text}"
        );
    }

    #[test]
    fn rendering_dispatches_to_the_chosen_format() {
        let report = sample();

        assert!(render(&report, Format::Json, false, -1).starts_with('{'));
        assert!(render(&report, Format::Github, false, -1).starts_with("::"));
        assert!(render(&report, Format::Sarif, false, -1).starts_with('{'));
        assert!(render(&report, Format::Junit, false, -1).starts_with("<?xml"));
        assert!(render(&report, Format::Compact, false, -1).contains("SKILL.md: line 2, col 1"));
        assert!(render(&report, Format::Stylish, false, -1).contains("helper"));
    }
}
