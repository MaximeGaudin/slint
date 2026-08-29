//! Workflow commands, which GitHub turns into annotations on the diff.
//!
//! The point of this format is that a finding lands on the line of the pull request that caused it,
//! where it is read by the person who can fix it, rather than in a log nobody opens.

use crate::diagnostics::{Report, Severity};

/// Escapes the data part of a workflow command, in the order the GitHub Actions spec requires:
/// `%` first (so a literal `%0A` in user text cannot be read back as a newline), then `\r` and
/// `\n` (or the command ends at the first real newline, taking the rest of the annotation).
fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escapes a property value like `file=` or `title=`, which additionally cannot contain the
/// delimiters `,` and `:` GitHub's naive property parser splits on.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

pub fn render(report: &Report) -> String {
    let mut lines = Vec::new();

    for skill in &report.skills {
        for message in &skill.messages {
            let level = match message.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "notice",
            };

            let body = escape_data(&format!(
                "{} ({})\n\n{}\n{}",
                message.message, message.rule, message.advice, message.reference.url
            ));

            // The span rides along when the rule knows it, in the order the workflow-command
            // spec lists the properties, so GitHub underlines the whole match and not just the
            // first character of it.
            let mut command = format!(
                "::{level} file={},line={},col={}",
                escape_property(&message.file),
                message.location.line,
                message.location.column,
            );

            if let Some(end_line) = message.location.end_line {
                command.push_str(&format!(",endLine={end_line}"));
            }

            if let Some(end_column) = message.location.end_column {
                command.push_str(&format!(",endColumn={end_column}"));
            }

            command.push_str(&format!(
                ",title=slint {}::{}",
                escape_property(&message.rule),
                body
            ));

            lines.push(command);
        }

        // The caveat about a pass that did not run has to survive the format, or a partial run
        // looks like a complete one in exactly the place people look: the PR annotations.
        for note in &skill.notes {
            lines.push(format!("::notice title=slint note::{}", escape_data(note)));
        }
    }

    for note in &report.notes {
        lines.push(format!("::notice title=slint note::{}", escape_data(note)));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Location;
    use crate::report::tests::sample;

    #[test]
    fn each_finding_becomes_one_annotation_at_its_position() {
        let mut report = sample();
        report.skills[0].notes.clear();
        let text = render(&report);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("::warning file=skills/helper/SKILL.md,line=2,col=1"));
        assert!(lines[1].starts_with("::error file=skills/helper/SKILL.md,line=9,col=1"));
    }

    // https://github.com/MaximeGaudin/slint/issues/144: a rule that knows the span (say, a
    // Windows path it matched) has to say where it ends, or GitHub underlines one character
    // where the offending text is seventeen.
    #[test]
    fn a_span_becomes_endline_and_endcolumn_so_the_whole_match_is_underlined() {
        let mut report = sample();
        report.skills[0].messages[0].location = Location::span(8, 9, 17);

        let text = render(&report);
        let line = text.lines().next().unwrap();

        assert!(
            line.contains("line=8,col=9,endLine=8,endColumn=26,"),
            "the span travels into the annotation: {line}"
        );
    }

    #[test]
    fn a_finding_with_no_span_stays_a_point_annotation() {
        let text = render(&sample());
        let line = text.lines().next().unwrap();

        assert!(!line.contains("endLine"), "{line}");
        assert!(!line.contains("endColumn"), "{line}");
    }

    #[test]
    fn a_note_is_a_notice_rather_than_a_warning() {
        let mut report = sample();
        report.skills[0].messages[0].severity = Severity::Info;

        assert!(render(&report).starts_with("::notice"));
    }

    #[test]
    fn the_advice_and_the_citation_ride_along_in_the_annotation_body() {
        let text = render(&sample());

        assert!(text.contains("Rename it after what it does."));
        assert!(text.contains("https://example.com/best-practices"));
    }

    #[test]
    fn newlines_are_escaped_so_an_annotation_is_never_cut_in_half() {
        let mut report = sample();
        report.skills[0].notes.clear();
        report.skills[0].messages[0].message = "first line\nsecond line".into();

        let text = render(&report);

        assert_eq!(text.lines().count(), 2, "still one line per finding");
        assert!(text.contains("first line%0Asecond line"));
    }

    #[test]
    fn a_comma_in_a_file_path_cannot_split_the_property_list() {
        let mut report = sample();
        report.skills[0].messages[0].file = "/tmp/f/skill, oddly named/SKILL.md".into();

        let text = render(&report);

        assert!(text.contains("file=/tmp/f/skill%2C oddly named/SKILL.md"));
        assert!(!text.contains("file=/tmp/f/skill, oddly named"));
    }

    #[test]
    fn a_colon_in_a_file_path_is_encoded_for_the_property_parser() {
        let mut report = sample();
        report.skills[0].messages[0].file = "/tmp/weird:name/SKILL.md".into();

        let text = render(&report);

        assert!(text.contains("file=/tmp/weird%3Aname/SKILL.md"));
    }

    #[test]
    fn a_literal_percent_sequence_in_user_text_is_never_reinterpreted() {
        let mut report = sample();
        report.skills[0].messages[0].message =
            "\"helper%0Adiscount\" has characters outside a-z".into();

        let text = render(&report);

        assert!(text.contains("helper%250Adiscount"));
        assert!(!text.contains("helper%0Adiscount"));
    }

    #[test]
    fn a_carriage_return_is_encoded_rather_than_deleted() {
        let mut report = sample();
        report.skills[0].notes.clear();
        report.skills[0].messages[0].message = "first\rsecond".into();

        let text = render(&report);

        assert!(text.contains("first%0Dsecond"));
        assert_eq!(text.lines().count(), 2, "still one line per finding");
    }

    #[test]
    fn a_clean_report_prints_nothing_at_all() {
        let empty = Report {
            skills: vec![],
            fixed: 0,
            notes: Vec::new(),
        };
        assert!(render(&empty).is_empty());
    }

    // https://github.com/MaximeGaudin/slint/issues/134: the CI path is exactly where a team is
    // most likely to be unaware that the model pass never ran, so the caveat has to ride along
    // as a notice rather than vanishing with the format.
    #[test]
    fn a_skipped_pass_becomes_a_notice_rather_than_disappearing() {
        let mut report = sample();
        report.notes = vec!["no provider is configured: set api_key_env".into()];

        let text = render(&report);

        assert!(
            text.contains("::notice title=slint note::no provider is configured"),
            "the run's notes print as notices: {text}"
        );
        assert!(
            text.contains("::notice title=slint note::8 rules need a model and none is configured."),
            "the skill's notes print as notices too: {text}"
        );
        assert!(
            text.lines().all(|line| !line.contains("api_key_env\n")),
            "a note stays on one line: {text}"
        );
    }
}
