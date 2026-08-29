//! The format a machine reads.
//!
//! An envelope with an `ok` flag, so a caller can branch before parsing anything, and data on
//! stdout with everything else on stderr — the shape that makes this safe to pipe. The editor
//! integration and any agent calling slint read exactly this.

use serde::Serialize;

use crate::diagnostics::Report;

#[derive(Debug, Serialize)]
pub struct Envelope<'a> {
    /// False when anything at error severity was found. Warnings do not flip it: they are the
    /// linter's opinion, and a caller that wants them fatal says so with --max-warnings.
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

pub fn envelope(report: &Report) -> Envelope<'_> {
    Envelope {
        ok: report.errors() == 0,
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

pub fn render(report: &Report) -> String {
    serde_json::to_string_pretty(&envelope(report))
        .unwrap_or_else(|failure| format!("{{\"ok\":false,\"error\":\"{failure}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;

    /// Regression for https://github.com/MaximeGaudin/slint/issues/125 —
    /// a consumer can only parse a moving format safely if the envelope says
    /// which version of it they are looking at.
    #[test]
    fn the_envelope_names_its_schema_version_so_consumers_can_detect_change() {
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample())).unwrap();

        assert_eq!(parsed["schemaVersion"], 1);
    }

    #[test]
    fn the_envelope_says_whether_anything_is_broken_before_the_data_is_read() {
        let text = render(&sample());
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

        let parsed: serde_json::Value = serde_json::from_str(&render(&report)).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn every_finding_carries_its_citation_into_the_json() {
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample())).unwrap();
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
        let parsed: serde_json::Value = serde_json::from_str(&render(&sample())).unwrap();
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
        };
        let parsed: serde_json::Value = serde_json::from_str(&render(&empty)).unwrap();

        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["summary"]["skills"], 0);
    }
}
