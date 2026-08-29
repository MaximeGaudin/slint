//! The format a machine reads.
//!
//! An envelope with an `ok` flag, so a caller can branch before parsing anything, and data on
//! stdout with everything else on stderr — the shape that makes this safe to pipe. The editor
//! integration and any agent calling slint read exactly this, and `ok` says what the exit code
//! says, so the two verdicts never disagree.

use serde::Serialize;

use crate::diagnostics::Report;

/// The version of the envelope's shape, so a consumer can detect a breaking
/// change before it breaks them. Bump on any change to the fields below that
/// removes or reinterprets one; adding a field does not.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Envelope<'a> {
    /// The version of this shape. Pinned in `SCHEMA_VERSION`.
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// False when the run failed: anything at error severity was found, or `--max-warnings` was
    /// exceeded. Warnings alone do not flip it — they are the linter's opinion, and a caller that
    /// wants them fatal says so with --max-warnings, which this flag honours.
    pub ok: bool,
    pub summary: Summary,
    pub data: &'a Report,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub skills: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub fixable: usize,
    pub fixed: usize,
}

pub fn envelope(report: &Report, max_warnings: i64) -> Envelope<'_> {
    Envelope {
        schema_version: SCHEMA_VERSION,
        ok: report.passes(max_warnings),
        summary: Summary {
            skills: report.skills.len(),
            errors: report.errors(),
            warnings: report.warnings(),
            infos: report.infos(),
            fixable: report.fixable(),
            fixed: report.fixed,
        },
        data: report,
    }
}

pub fn render(report: &Report, max_warnings: i64) -> String {
    serde_json::to_string_pretty(&envelope(report, max_warnings))
        .unwrap_or_else(|failure| format!("{{\"ok\":false,\"error\":\"{failure}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;
    use std::collections::BTreeMap;

    /// Regression for https://github.com/MaximeGaudin/slint/issues/125 —
    /// a consumer can only parse a moving format safely if the envelope says
    /// which version of it they are looking at.
    #[test]
    fn the_envelope_names_its_schema_version_so_consumers_can_detect_change() {
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample(), -1)).unwrap();

        assert_eq!(parsed["schemaVersion"], 1);
    }

    #[test]
    fn the_envelope_says_whether_anything_is_broken_before_the_data_is_read() {
        let text = render(&sample(), -1);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(parsed["ok"], false, "the sample has an error in it");
        assert_eq!(parsed["summary"]["errors"], 1);
        assert_eq!(parsed["summary"]["warnings"], 1);
        assert_eq!(parsed["summary"]["skills"], 1);
    }

    #[test]
    fn warnings_alone_leave_the_run_ok() {
        let mut report = sample();
        report.skills[0]
            .messages
            .retain(|one| one.severity != crate::Severity::Error);

        let parsed: serde_json::Value = serde_json::from_str(&render(&report, -1)).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn a_warning_budget_flips_ok_like_the_exit_code_does() {
        let mut report = sample();
        report.skills[0]
            .messages
            .retain(|one| one.severity != crate::Severity::Error);

        let parsed: serde_json::Value = serde_json::from_str(&render(&report, 0)).unwrap();
        assert_eq!(
            parsed["ok"], false,
            "the budget is exceeded, so the run failed"
        );

        let parsed: serde_json::Value = serde_json::from_str(&render(&report, 1)).unwrap();
        assert_eq!(parsed["ok"], true, "the budget still has room");
    }

    #[test]
    fn every_finding_carries_its_citation_into_the_json() {
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample(), -1)).unwrap();
        let first = &parsed["data"]["skills"][0]["messages"][0];

        assert_eq!(first["rule"], "name/not-generic");
        assert_eq!(first["severity"], "warning");
        assert_eq!(
            first["reference"]["url"],
            "https://example.com/best-practices"
        );
        assert!(first["advice"].as_str().unwrap().len() > 10);
    }

    #[test]
    fn notes_survive_into_the_json_so_a_client_can_show_what_did_not_run() {
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample(), -1)).unwrap();
        assert!(
            parsed["data"]["skills"][0]["notes"][0]
                .as_str()
                .unwrap()
                .contains("need a model")
        );
    }

    #[test]
    fn the_output_is_valid_json_even_with_nothing_in_it() {
        let empty = crate::Report {
            skills: vec![],
            fixed: 0,
            notes: Vec::new(),
            fingerprints: BTreeMap::new(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&render(&empty, -1)).unwrap();

        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["summary"]["skills"], 0);
    }
}
