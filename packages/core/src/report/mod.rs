//! Printing a report.
//!
//! Four formats, because four different readers exist: a person at a terminal, a CI annotation
//! service, an editor, and another agent. They are all the same data, and none of them is the
//! "real" one — the JSON is generated from the same report the terminal output is.

pub mod github;
pub mod json;
pub mod stylish;

use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::diagnostics::Report;

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
            "compact" => Ok(Format::Compact),
            other => Err(format!(
                "unknown format \"{other}\" — try stylish, json, github or compact"
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
        Format::Compact => compact(report),
    }
}

fn compact(report: &Report) -> String {
    let mut lines = Vec::new();

    for skill in &report.skills {
        for message in &skill.messages {
            lines.push(format!(
                "{}:{}:{}: {} [{}] {}",
                message.file,
                message.location.line,
                message.location.column,
                message.severity,
                message.rule,
                message.message
            ));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};

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
        }
    }

    #[test]
    fn every_format_has_a_name_that_can_be_typed() {
        assert_eq!("stylish".parse::<Format>().unwrap(), Format::Stylish);
        assert_eq!("json".parse::<Format>().unwrap(), Format::Json);
        assert_eq!("github".parse::<Format>().unwrap(), Format::Github);
        assert_eq!("compact".parse::<Format>().unwrap(), Format::Compact);

        let failure = "yaml".parse::<Format>().unwrap_err();
        assert!(
            failure.contains("stylish"),
            "the error lists what does work: {failure}"
        );
    }

    #[test]
    fn compact_prints_one_grep_friendly_line_per_finding() {
        let text = compact(&sample());
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("skills/helper/SKILL.md:2:1: warning [name/not-generic]"));
    }

    #[test]
    fn rendering_dispatches_to_the_chosen_format() {
        let report = sample();

        assert!(render(&report, Format::Json, false, -1).starts_with('{'));
        assert!(render(&report, Format::Github, false, -1).starts_with("::"));
        assert!(render(&report, Format::Compact, false, -1).contains("SKILL.md:2:1"));
        assert!(render(&report, Format::Stylish, false, -1).contains("helper"));
    }
}
