//! The report as SARIF, for the scanners, dashboards and code-quality gates that speak it.
//!
//! SARIF 2.1.0, one result per finding, the same data every other format prints. Levels map the
//! way the spec's consumers expect: SARIF has no `info`, so a judgement call becomes `note`.

use serde_json::{Value, json};

use crate::diagnostics::{Location, Report, Severity};

/// The whole report, as one SARIF log.
pub fn render(report: &Report) -> String {
    let results: Vec<Value> = report
        .skills
        .iter()
        .flat_map(|skill| {
            skill.messages.iter().map(|message| {
                json!({
                    "ruleId": message.rule,
                    "level": level(message.severity),
                    "message": { "text": message.message },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": message.file },
                            "region": region(message.location),
                        },
                    }],
                    // The citation is what makes a SARIF viewer as useful as the terminal output.
                    "properties": {
                        "advice": message.advice,
                        "reference": message.reference.url,
                    },
                })
            })
        })
        .collect();

    let log = json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "slint",
                    "informationUri": "https://slint.dev",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            },
            "results": results,
        }],
    });

    serde_json::to_string_pretty(&log).expect("the SARIF log is representable")
}

fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

fn region(location: Location) -> Value {
    let mut region = json!({
        "startLine": location.line,
        "startColumn": location.column,
    });

    if let Some(end_line) = location.end_line {
        region["endLine"] = json!(end_line);
    }
    if let Some(end_column) = location.end_column {
        region["endColumn"] = json!(end_column);
    }

    region
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Message, Reference, SkillReport, Source};

    fn report() -> Report {
        Report {
            skills: vec![SkillReport {
                path: "skills/helper".into(),
                name: "helper".into(),
                messages: vec![Message {
                    rule: "name/not-generic".into(),
                    severity: Severity::Warning,
                    message: "\"helper\" says nothing about what this does".into(),
                    advice: "Rename it after what it does.".into(),
                    location: Location {
                        line: 2,
                        column: 1,
                        end_line: Some(2),
                        end_column: Some(9),
                    },
                    source: Source::Static,
                    file: "skills/helper/SKILL.md".into(),
                    fix: None,
                    reference: Reference {
                        title: "t".into(),
                        url: "https://example.com".into(),
                    },
                    confidence: 1.0,
                }],
                notes: vec![],
            }],
            fixed: 0,
        }
    }

    #[test]
    fn the_log_declares_sarif_2_1_and_names_the_tool() {
        let parsed: Value = serde_json::from_str(&render(&report())).unwrap();

        assert_eq!(parsed["version"], "2.1.0");
        assert_eq!(parsed["runs"][0]["tool"]["driver"]["name"], "slint");
        assert_eq!(
            parsed["runs"][0]["tool"]["driver"]["informationUri"],
            "https://slint.dev"
        );
    }

    #[test]
    fn a_finding_becomes_a_result_with_its_location_and_citation() {
        let parsed: Value = serde_json::from_str(&render(&report())).unwrap();
        let result = &parsed["runs"][0]["results"][0];

        assert_eq!(result["ruleId"], "name/not-generic");
        assert_eq!(result["level"], "warning", "a warning stays a warning");
        assert_eq!(
            result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "skills/helper/SKILL.md"
        );
        assert_eq!(
            result["locations"][0]["physicalLocation"]["region"]["startLine"],
            2
        );
        assert_eq!(
            result["locations"][0]["physicalLocation"]["region"]["endLine"],
            2
        );
        assert_eq!(result["properties"]["reference"], "https://example.com");
    }

    #[test]
    fn an_info_finding_maps_to_sarif_note() {
        let mut report = report();
        report.skills[0].messages[0].severity = Severity::Info;

        let parsed: Value = serde_json::from_str(&render(&report)).unwrap();

        assert_eq!(parsed["runs"][0]["results"][0]["level"], "note");
    }
}
