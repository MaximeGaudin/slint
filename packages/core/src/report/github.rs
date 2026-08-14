//! Workflow commands, which GitHub turns into annotations on the diff.
//!
//! The point of this format is that a finding lands on the line of the pull request that caused it,
//! where it is read by the person who can fix it, rather than in a log nobody opens.

use crate::diagnostics::{Report, Severity};

pub fn render(report: &Report) -> String {
    let mut lines = Vec::new();

    for skill in &report.skills {
        for message in &skill.messages {
            let level = match message.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "notice",
            };

            // Newlines have to be escaped or the command ends at the first one, taking the rest of
            // the annotation with it.
            let body = format!(
                "{} ({})%0A%0A{}%0A{}",
                message.message, message.rule, message.advice, message.reference.url
            )
            .replace('\n', "%0A")
            .replace('\r', "");

            lines.push(format!(
                "::{level} file={},line={},col={},title=slint {}::{}",
                message.file, message.location.line, message.location.column, message.rule, body
            ));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;

    #[test]
    fn each_finding_becomes_one_annotation_at_its_position() {
        let text = render(&sample());
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("::warning file=skills/helper/SKILL.md,line=2,col=1"));
        assert!(lines[1].starts_with("::error file=skills/helper/SKILL.md,line=9,col=1"));
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
        report.skills[0].messages[0].message = "first line\nsecond line".into();

        let text = render(&report);

        assert_eq!(text.lines().count(), 2, "still one line per finding");
        assert!(text.contains("first line%0Asecond line"));
    }

    #[test]
    fn a_clean_report_prints_nothing_at_all() {
        let empty = Report {
            skills: vec![],
            fixed: 0,
        };
        assert!(render(&empty).is_empty());
    }
}
