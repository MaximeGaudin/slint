//! The format a person reads.
//!
//! Grouped by skill, worst first, with the position on the left where the eye scans. Every finding
//! carries the sentence saying what to do and the document the rule comes from, because a linter
//! that only says "wrong" is a linter that gets argued with.

use owo_colors::OwoColorize;
use std::collections::BTreeSet;

use crate::diagnostics::{Message, Report, Severity, Source};

pub fn render(report: &Report, colour: bool) -> String {
    let mut out = String::new();
    // A rule's citation is printed the first time it appears and not again. Forty findings each
    // repeating the same URL is a wall of identical grey text that stops being read by the third.
    let mut cited: BTreeSet<&str> = BTreeSet::new();

    // The longest position and rule, so the columns line up rather than each row being its own
    // ragged shape.
    let width = |pick: fn(&Message) -> String| {
        report
            .skills
            .iter()
            .flat_map(|skill| skill.messages.iter())
            .map(|message| pick(message).chars().count())
            .max()
            .unwrap_or(0)
    };

    let position_width = width(position_of);
    let rule_width = width(|message| message.rule.clone());

    for skill in &report.skills {
        if skill.messages.is_empty() && skill.notes.is_empty() {
            continue;
        }

        out.push('\n');
        out.push_str(&paint(&skill.path, Paint::Bold, colour));

        let counts = counts_for(skill);
        if !counts.is_empty() {
            out.push_str(&paint(&format!("  {counts}"), Paint::Dim, colour));
        }
        out.push('\n');

        for message in &skill.messages {
            let position = position_of(message);

            out.push_str(&format!(
                "  {:>position_width$}  {}  {}  {}{}\n",
                paint(&position, Paint::Dim, colour),
                marker(message.severity, colour),
                paint(&pad(&message.rule, rule_width), Paint::Dim, colour),
                message.message,
                origin(message, colour),
            ));

            // The advice sits under the finding, indented past the columns so the eye follows one
            // line rather than scanning back to the left margin.
            out.push_str(&format!(
                "  {:>position_width$}  {}  {}\n",
                "",
                "       ",
                paint(
                    &format!("What to do: {}", message.advice),
                    Paint::Dim,
                    colour
                ),
            ));

            if cited.insert(message.rule.as_str()) {
                out.push_str(&format!(
                    "  {:>position_width$}  {}  {}\n",
                    "",
                    "       ",
                    paint(&message.reference.url, Paint::Dim, colour),
                ));
            }

            out.push('\n');
        }

        for note in &skill.notes {
            out.push_str(&format!(
                "  {}  {}\n",
                paint("note", Paint::Blue, colour),
                paint(note, Paint::Dim, colour)
            ));
        }
    }

    // About the run as a whole, not one skill: an argument that was not a skill at all.
    for note in &report.notes {
        out.push_str(&format!(
            "  {}  {}\n",
            paint("note", Paint::Blue, colour),
            paint(note, Paint::Dim, colour)
        ));
    }

    out.push('\n');
    out.push_str(&summary(report, colour));
    out.push('\n');

    out
}

/// `SKILL.md:10:1`, or the bundled file's own name when the finding is about one.
fn position_of(message: &Message) -> String {
    let file = message
        .file
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(&message.file);

    format!(
        "{file}:{}:{}",
        message.location.line, message.location.column
    )
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$}")
}

fn marker(severity: Severity, colour: bool) -> String {
    match severity {
        Severity::Error => paint("error  ", Paint::Red, colour),
        Severity::Warning => paint("warning", Paint::Yellow, colour),
        Severity::Info => paint("note   ", Paint::Blue, colour),
    }
}

/// Where a finding came from, when it is worth knowing. A static one says nothing: it is the
/// default, and marking it would put a badge on nine rows out of ten.
fn origin(message: &Message, colour: bool) -> String {
    match message.source {
        Source::Model => paint(
            &format!(" · model {:.0}%", message.confidence * 100.0),
            Paint::Dim,
            colour,
        ),
        Source::Plugin => paint(" · plugin", Paint::Dim, colour),
        Source::Static => String::new(),
    }
}

fn counts_for(skill: &crate::diagnostics::SkillReport) -> String {
    let errors = skill.count(Severity::Error);
    let warnings = skill.count(Severity::Warning);
    let infos = skill.count(Severity::Info);

    let parts: Vec<String> = [(errors, "error"), (warnings, "warning"), (infos, "note")]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}{}", if count == 1 { "" } else { "s" }))
        .collect();

    parts.join(", ")
}

/// The last line: what was found, and what can be done about it without thinking.
fn summary(report: &Report, colour: bool) -> String {
    if report.skills.is_empty() {
        // A run that looked at nothing is not a clean pass: a typo'd CI path must never read as
        // success, so this gets its own verdict rather than the all-clear.
        let mut nothing = paint(
            "No SKILL.md files found — nothing was linted.",
            Paint::Yellow,
            colour,
        );

        if report.fixed > 0 {
            nothing.push_str(&format!("\n{} fix(es) applied.", report.fixed));
        }

        return nothing;
    }

    let errors = report.errors();
    let warnings = report.warnings();
    let infos = report.infos();

    if errors + warnings + infos == 0 {
        let mut clean = paint(
            &format!("Nothing to report across {} skill(s).", report.skills.len()),
            Paint::Green,
            colour,
        );

        // A run that fixed everything it found still has to say what it did to the files, or
        // --fix looks like it did nothing at all.
        if report.fixed > 0 {
            clean.push_str(&format!("\n{} fix(es) applied.", report.fixed));
        }

        return clean;
    }

    let counted = format!(
        "{} problem(s): {errors} error(s), {warnings} warning(s), {infos} note(s) across {} skill(s).",
        errors + warnings + infos,
        report.skills.len()
    );

    let mut line = if errors > 0 {
        paint(&counted, Paint::Red, colour)
    } else {
        paint(&counted, Paint::Yellow, colour)
    };

    let fixable = report.fixable();
    if fixable > 0 {
        line.push_str(&format!(
            "\n{}",
            paint(
                &format!("{fixable} of them are computed fixes: run again with --fix."),
                Paint::Dim,
                colour
            )
        ));
    }

    if report.fixed > 0 {
        line.push_str(&format!("\n{} fix(es) applied.", report.fixed));
    }

    line
}

enum Paint {
    Bold,
    Dim,
    Red,
    Yellow,
    Blue,
    Green,
}

fn paint(text: &str, style: Paint, colour: bool) -> String {
    if !colour {
        return text.to_string();
    }

    match style {
        Paint::Bold => text.bold().to_string(),
        Paint::Dim => text.dimmed().to_string(),
        Paint::Red => text.red().to_string(),
        Paint::Yellow => text.yellow().to_string(),
        Paint::Blue => text.blue().to_string(),
        Paint::Green => text.green().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::tests::sample;

    #[test]
    fn a_finding_prints_its_position_message_rule_advice_and_source() {
        let text = render(&sample(), false);

        assert!(text.contains("skills/helper"));
        assert!(text.contains("2:1"));
        assert!(text.contains("warning"));
        assert!(text.contains("\"helper\" says nothing about what this does"));
        assert!(text.contains("name/not-generic"));
        assert!(text.contains("What to do: Rename it after what it does."));
        assert!(text.contains("https://example.com/best-practices"));
    }

    #[test]
    fn notes_are_printed_so_a_pass_that_did_not_run_is_visible() {
        let text = render(&sample(), false);
        assert!(text.contains("8 rules need a model"));
    }

    #[test]
    fn the_summary_counts_every_severity() {
        let text = render(&sample(), false);
        assert!(text.contains("2 problem(s): 1 error(s), 1 warning(s), 0 note(s)"));
    }

    #[test]
    fn a_clean_run_says_so_rather_than_printing_nothing() {
        let mut clean = sample();
        for skill in &mut clean.skills {
            skill.messages.clear();
            skill.notes.clear();
        }

        assert!(render(&clean, false).contains("Nothing to report across 1 skill(s)."));
    }

    #[test]
    fn a_run_that_lints_nothing_is_not_reported_as_clean() {
        // https://github.com/MaximeGaudin/slint/issues/118: "found nothing to check" is a
        // different verdict from "checked and all clean", and must not share its wording.
        let empty = Report {
            skills: vec![],
            fixed: 0,
            notes: vec![],
        };
        let text = render(&empty, false);

        assert!(text.contains("No SKILL.md files found"), "{text}");
        assert!(!text.contains("Nothing to report"), "{text}");
    }

    #[test]
    fn a_run_that_fixed_everything_still_says_what_it_did() {
        let mut fixed = sample();
        for skill in &mut fixed.skills {
            skill.messages.clear();
            skill.notes.clear();
        }
        fixed.fixed = 3;
        let text = render(&fixed, false);

        assert!(text.contains("Nothing to report"), "{text}");
        assert!(text.contains("3 fix(es) applied."), "{text}");
    }

    #[test]
    fn colour_is_optional_and_off_leaves_no_escape_codes() {
        let text = render(&sample(), false);
        assert!(!text.contains('\u{1b}'));

        let coloured = render(&sample(), true);
        assert!(coloured.contains('\u{1b}'));
    }

    #[test]
    fn fixable_findings_are_advertised_once_and_only_when_there_are_any() {
        let mut report = sample();
        assert!(!render(&report, false).contains("--fix"));

        report.skills[0].messages[0].fix = Some(crate::diagnostics::Fix {
            start: 0,
            end: 1,
            replacement: "x".into(),
            description: "d".into(),
        });

        assert!(render(&report, false).contains("1 of them are computed fixes"));
    }
}
