//! The report as JUnit XML, for the CI systems — Jenkins, GitLab, Buildkite — that render test
//! results from it rather than from annotations.
//!
//! One testsuite per skill, one testcase per finding, a failure or an error on the testcase so
//! the run turns red where it should. Notes ride along in `system-out`: a skipped pass is not a
//! finding, but it is the difference between a partial run and a complete one.

use crate::diagnostics::{Report, Severity};

/// The whole report, as one JUnit XML log.
pub fn render(report: &Report) -> String {
    let mut errors = 0;
    let mut failures = 0;
    let mut tests = 0;
    let mut suites = String::new();

    for skill in &report.skills {
        let mut suite_errors = 0;
        let mut suite_failures = 0;
        let mut cases = String::new();

        for message in &skill.messages {
            let (element, kind, counted) = match message.severity {
                Severity::Error => ("error", "error", &mut suite_errors),
                Severity::Warning | Severity::Info => {
                    ("failure", message.severity.as_str(), &mut suite_failures)
                }
            };

            *counted += 1;

            cases.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"{}\" file=\"{}\" line=\"{}\" column=\"{}\">\n",
                escape_attribute(&skill.path),
                escape_attribute(&message.rule),
                escape_attribute(&message.file),
                message.location.line,
                message.location.column,
            ));

            cases.push_str(&format!(
                "      <{element} message=\"{}\" type=\"{kind}\">What to do: {}&#10;{}</{element}>\n",
                escape_attribute(&message.message),
                escape_attribute(&message.advice),
                escape_attribute(&message.reference.url),
            ));

            cases.push_str("    </testcase>\n");
        }

        // A suite with no testcase reads as skipped rather than passed, so a clean skill gets
        // one to carry its green.
        let suite_tests = if skill.messages.is_empty() { 1 } else { skill.messages.len() };

        if skill.messages.is_empty() {
            cases.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"no findings\"/>\n",
                escape_attribute(&skill.path)
            ));
        }

        suites.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\">\n",
            escape_attribute(&skill.path),
            suite_tests,
            suite_failures,
            suite_errors,
        ));

        if !skill.notes.is_empty() {
            let notes = skill.notes.join("\n");
            suites.push_str(&format!(
                "    <system-out>{}</system-out>\n",
                escape_text(&notes)
            ));
        }

        suites.push_str(&cases);
        suites.push_str("  </testsuite>\n");

        errors += suite_errors;
        failures += suite_failures;
        tests += suite_tests;
    }

    // A run that looked at nothing still has to be a valid, passing log — some CI parsers treat
    // an empty testsuites element as a run that never happened.
    if report.skills.is_empty() {
        tests += 1;
        suites.push_str("  <testsuite name=\"slint\" tests=\"1\" failures=\"0\" errors=\"0\">\n");
        suites
            .push_str("    <testcase classname=\"slint\" name=\"no skills were linted\"/>\n");
        suites.push_str("  </testsuite>\n");
    }

    // Notes about the run as a whole — the model pass that did not run, an argument that was
    // skipped — are not findings, but they are the difference between a partial run and a
    // complete one, so they get a suite whose system-out carries them.
    if !report.notes.is_empty() {
        tests += 1;
        suites.push_str("  <testsuite name=\"slint\" tests=\"1\" failures=\"0\" errors=\"0\">\n");
        suites.push_str(&format!(
            "    <system-out>{}</system-out>\n",
            escape_text(&report.notes.join("\n"))
        ));
        suites.push_str("    <testcase classname=\"slint\" name=\"run notes\"/>\n");
        suites.push_str("  </testsuite>\n");
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"slint\" tests=\"{}\" failures=\"{}\" errors=\"{}\">\n{} </testsuites>\n",
        tests, failures, errors, suites
    )
}

/// Escapes text that lands inside an attribute value, where the delimiters themselves end the
/// attribute. A newline is written as its reference so one finding stays one line of XML.
fn escape_attribute(text: &str) -> String {
    escape_text(text)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\n', "&#10;")
}

/// Escapes text that lands inside an element or an attribute: the markup characters first, so a
/// finding quoting XML cannot be read back as markup.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};
    use crate::report::tests::sample;

    #[test]
    fn the_log_declares_itself_and_counts_every_finding() {
        let text = render(&sample());

        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{text}");
        assert!(
            text.contains("<testsuites name=\"slint\" tests=\"2\" failures=\"1\" errors=\"1\">"),
            "one warning and one error across the run: {text}"
        );
        assert!(
            text.contains("<testsuite name=\"skills/helper\" tests=\"2\" failures=\"1\" errors=\"1\">"),
            "{text}"
        );
    }

    #[test]
    fn each_finding_is_a_testcase_at_its_position_and_its_rule() {
        let text = render(&sample());

        assert!(
            text.contains("<testcase classname=\"skills/helper\" name=\"name/not-generic\" file=\"skills/helper/SKILL.md\" line=\"2\" column=\"1\">"),
            "{text}"
        );
    }

    #[test]
    fn severity_decides_whether_a_finding_fails_or_errors_the_testcase() {
        let text = render(&sample());

        assert!(
            text.contains("<failure message=\"&quot;helper&quot; says nothing about what this does\" type=\"warning\">"),
            "a warning is a failure: {text}"
        );
        assert!(
            text.contains("<error message=\"The instructions name scripts/cull.py, which is not in the bundle\" type=\"error\">"),
            "an error is an error: {text}"
        );
    }

    #[test]
    fn the_advice_and_the_citation_ride_along_in_the_failure_body() {
        let text = render(&sample());

        assert!(text.contains("What to do: Rename it after what it does."));
        assert!(text.contains("https://example.com/best-practices"));
    }

    #[test]
    fn user_text_is_escaped_so_a_finding_cannot_break_the_xml() {
        let mut report = sample();
        report.skills[0].messages[0].message = "\"helper\" <script> & 'quotes'".into();

        let text = render(&report);

        assert!(text.contains("&quot;helper&quot; &lt;script&gt; &amp; &apos;quotes&apos;"), "{text}");
        assert!(!text.contains("<script>"), "{text}");
    }

    #[test]
    fn an_info_finding_is_a_failure_rather_than_a_pass() {
        let mut report = sample();
        report.skills[0].messages[0].severity = Severity::Info;

        let text = render(&report);

        assert!(text.contains("type=\"info\""), "{text}");
        assert!(text.contains("<testsuites name=\"slint\" tests=\"2\" failures=\"1\" errors=\"1\">"), "{text}");
    }

    #[test]
    fn a_clean_run_still_carries_one_passing_testcase_so_the_run_is_not_read_as_skipped() {
        let mut report = sample();
        report.skills[0].messages.clear();

        let text = render(&report);

        assert!(text.contains("tests=\"1\" failures=\"0\" errors=\"0\""), "{text}");
        assert!(text.contains("<testcase classname=\"skills/helper\" name=\"no findings\"/>"), "{text}");
    }

    #[test]
    fn a_report_with_no_skills_at_all_is_one_suite_with_one_passing_testcase() {
        let empty = Report {
            skills: vec![],
            fixed: 0,
            notes: Vec::new(),
        };

        let text = render(&empty);

        assert!(text.contains("<testsuite name=\"slint\" tests=\"1\" failures=\"0\" errors=\"0\">"), "{text}");
        assert!(text.contains("<testcase classname=\"slint\" name=\"no skills were linted\"/>"), "{text}");
    }

    #[test]
    fn notes_ride_along_in_system_out_so_a_skipped_pass_stays_visible() {
        let mut report = sample();
        report.notes = vec!["no provider is configured: set api_key_env".into()];

        let text = render(&report);

        assert!(
            text.contains("<system-out>no provider is configured: set api_key_env</system-out>"),
            "{text}"
        );
    }
}
