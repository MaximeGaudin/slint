//! The format a person reads.
//!
//! Grouped by skill, worst first, with the position on the left where the eye scans. Every finding
//! carries the sentence saying what to do and the document the rule comes from, because a linter
//! that only says "wrong" is a linter that gets argued with.

use owo_colors::OwoColorize;
use std::collections::BTreeSet;
use unicode_width::UnicodeWidthStr;

use crate::diagnostics::{Message, Report, Severity, Source};

pub fn render(report: &Report, colour: bool) -> String {
    render_with_width(report, colour, terminal_width())
}

/// Like render, with the wrap width given outright. A test process has no terminal to measure,
/// so this is the seam the tests pin: `Some(width)` wraps, `None` runs the lines at their
/// natural length, exactly as when no width can be known.
pub fn render_with_width(report: &Report, colour: bool, width: Option<usize>) -> String {
    let mut out = String::new();
    // A rule's citation is printed the first time it appears and not again. Forty findings each
    // repeating the same URL is a wall of identical grey text that stops being read by the third.
    let mut cited: BTreeSet<&str> = BTreeSet::new();

    // The longest position and rule, so the columns line up rather than each row being its own
    // ragged shape. Widths are display columns — a CJK rule name is two columns per character,
    // and counting scalar values would leave its neighbours' messages hanging in the air.
    let width_of = |pick: fn(&Message) -> String| {
        report
            .skills
            .iter()
            .flat_map(|skill| skill.messages.iter())
            .map(|message| pick(message).width())
            .max()
            .unwrap_or(0)
    };

    let position_width = width_of(position_of);
    let rule_width = width_of(|message| message.rule.clone());

    // What hangs under the message column of the row above, so a wrapped finding still reads as
    // one finding. Both gutters are plain spaces of the same display width as the row's prefix.
    let message_gutter = format!(
        "  {}  {}  {}  ",
        " ".repeat(position_width),
        "       ",
        " ".repeat(rule_width)
    );
    let advice_gutter = format!("  {}  {}  ", " ".repeat(position_width), "       ");

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
            let marker_text = marker(message.severity, colour);
            let rule_text = paint(&pad(&message.rule, rule_width), Paint::Dim, colour);

            // The message and its advice wrap when a terminal width is known; the citation URL
            // never does — a URL broken across lines is a URL that no longer works.
            let message_lines = fitted(
                &message.message,
                width.map(|width| width.saturating_sub(position_width + rule_width + 15)),
            );
            let source = origin(message, colour);

            for (index, line) in message_lines.iter().enumerate() {
                if index == 0 {
                    out.push_str(&format!(
                        "  {:>position_width$}  {}  {}  ",
                        paint(&position, Paint::Dim, colour),
                        marker_text,
                        rule_text,
                    ));
                } else {
                    out.push_str(&message_gutter);
                }

                out.push_str(line);

                if index + 1 == message_lines.len() {
                    out.push_str(&source);
                }

                out.push('\n');
            }

            // The advice sits under the finding, indented past the columns so the eye follows one
            // line rather than scanning back to the left margin.
            let advice_lines = fitted(
                &format!("What to do: {}", message.advice),
                width.map(|width| width.saturating_sub(position_width + 13)),
            );

            for line in &advice_lines {
                out.push_str(&advice_gutter);
                out.push_str(&paint(line, Paint::Dim, colour));
                out.push('\n');
            }

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

/// Below this a "wrapped" row is one word per line — no report is readable like that, so narrow
/// panes get the natural-length lines instead.
const MIN_WRAP_WIDTH: usize = 12;

/// `text` as the lines that fit the available columns, or whole when there is no usable width or
/// the text already fits. Words are kept whole; one longer than the line gets the line to itself.
fn fitted(text: &str, available: Option<usize>) -> Vec<String> {
    match available.filter(|available| *available >= MIN_WRAP_WIDTH) {
        Some(width) if text.width() > width => wrap(text, width),
        _ => vec![text.to_string()],
    }
}

/// Greedy word wrap on spaces, measured in display columns.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in text.split(' ') {
        if !current.is_empty() && current.width() + 1 + word.width() > width {
            lines.push(std::mem::take(&mut current));
        }

        if !current.is_empty() {
            current.push(' ');
        }

        current.push_str(word);
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

/// The width to wrap at, when one can be known: COLUMNS wins, because a caller that pinned it
/// knows the pane better than the ioctl does; then the terminal's own size; then nothing, and
/// nothing means the lines run their natural length.
fn terminal_width() -> Option<usize> {
    width_from_environment().or_else(width_from_terminal)
}

fn width_from_environment() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|columns| columns.trim().parse::<usize>().ok())
        .filter(|columns| *columns > 0)
}

#[cfg(unix)]
fn width_from_terminal() -> Option<usize> {
    let mut window = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // SAFETY: TIOCGWINSZ with a valid, mutable winsize — the kernel only fills the struct in.
    let measured = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut window) };

    (measured == 0 && window.ws_col > 0).then_some(window.ws_col as usize)
}

#[cfg(not(unix))]
fn width_from_terminal() -> Option<usize> {
    None
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

/// Spaces out to `width` display columns, so a row's next column starts where its neighbours'
/// do — `format!("{text:<width$}")` counts scalar values, not what a terminal actually shows.
fn pad(text: &str, width: usize) -> String {
    let used = text.width();
    let mut padded = text.to_string();

    if width > used {
        padded.push_str(&" ".repeat(width - used));
    }

    padded
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
        .map(|(count, label)| plural(count, label))
        .collect();

    parts.join(", ")
}

/// A count and its word, said properly: "1 error" and "2 errors" — never the "(s)" placeholder.
/// Words that pluralise with -es ("fix" → "fixes") get it, so the applied count reads right too.
fn plural(count: usize, word: &str) -> String {
    let suffix = if count == 1 {
        ""
    } else if word.ends_with(['s', 'x', 'z']) || word.ends_with("ch") || word.ends_with("sh") {
        "es"
    } else {
        "s"
    };

    format!("{count} {word}{suffix}")
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
            nothing.push_str(&format!("\n{} applied.", plural(report.fixed, "fix")));
        }

        return nothing;
    }

    let errors = report.errors();
    let warnings = report.warnings();
    let infos = report.infos();

    if errors + warnings + infos == 0 {
        let mut clean = paint(
            &format!(
                "Nothing to report across {}.",
                plural(report.skills.len(), "skill")
            ),
            Paint::Green,
            colour,
        );

        // A run that fixed everything it found still has to say what it did to the files, or
        // --fix looks like it did nothing at all.
        if report.fixed > 0 {
            clean.push_str(&format!("\n{} applied.", plural(report.fixed, "fix")));
        }

        return clean;
    }

    let counted = format!(
        "{}: {}, {}, {} across {}.",
        plural(errors + warnings + infos, "problem"),
        plural(errors, "error"),
        plural(warnings, "warning"),
        plural(infos, "note"),
        plural(report.skills.len(), "skill")
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
        line.push_str(&format!("\n{} applied.", plural(report.fixed, "fix")));
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
    use std::collections::BTreeMap;

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
        assert!(text.contains("2 problems: 1 error, 1 warning, 0 notes"));
    }

    #[test]
    fn singular_counts_are_not_written_with_a_parenthesised_s() {
        // https://github.com/MaximeGaudin/slint/issues/169: the summary hard-coded "(s)", so
        // one finding read as "1 problem(s): 1 error(s)" while the per-skill line right above
        // it already said "1 warning". Same numbers, same words.
        let mut single = sample();
        single.skills[0].messages.truncate(1);
        single.skills[0].notes.clear();
        single.fixed = 1;

        let text = render(&single, false);

        assert!(
            text.contains("1 problem: 0 errors, 1 warning, 0 notes across 1 skill."),
            "{text}"
        );
        assert!(text.contains("1 fix applied."), "{text}");
        assert!(!text.contains("(s)"), "{text}");
    }

    #[test]
    fn a_clean_run_says_so_rather_than_printing_nothing() {
        let mut clean = sample();
        for skill in &mut clean.skills {
            skill.messages.clear();
            skill.notes.clear();
        }

        assert!(render(&clean, false).contains("Nothing to report across 1 skill."));
    }

    #[test]
    fn a_run_that_lints_nothing_is_not_reported_as_clean() {
        // https://github.com/MaximeGaudin/slint/issues/118: "found nothing to check" is a
        // different verdict from "checked and all clean", and must not share its wording.
        let empty = Report {
            skills: vec![],
            fixed: 0,
            notes: vec![],
            fingerprints: BTreeMap::new(),
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
        assert!(text.contains("3 fixes applied."), "{text}");
    }

    #[test]
    fn a_wide_rule_name_pads_to_its_display_width_not_its_char_count() {
        // https://github.com/MaximeGaudin/slint/issues/163: a CJK rule name is 12 scalar values
        // but 18 terminal columns; padding by scalar values pushes the message column of every
        // row sharing the width out of line with the wide row.
        fn display_width(text: &str) -> usize {
            // Enough of an oracle for this test: ASCII is one column, the CJK Unified
            // Ideographs used here are East Asian Wide, two columns.
            text.chars()
                .map(|c| {
                    if ('\u{4E00}'..='\u{9FFF}').contains(&c) {
                        2
                    } else {
                        1
                    }
                })
                .sum()
        }

        let mut report = sample();
        report.skills[0].messages[0].rule = "house/规则名称测试".into();
        report.skills[0].messages[1].rule = "house/x".into();

        let text = render(&report, false);
        let finding_row = |needle: &str| {
            text.lines()
                .find(|line| line.contains(needle))
                .expect("each finding renders on its own row")
        };

        // The display width of everything left of the message must be the same on both rows.
        let message_column = |line: &str, needle: &str| {
            display_width(&line[..line.find(needle).expect("the message is on its row")])
        };

        assert_eq!(
            message_column(finding_row("\"helper\" says"), "\"helper\" says"),
            message_column(
                finding_row("The instructions name scripts"),
                "The instructions name"
            ),
            "both message columns must begin on the same display column"
        );
    }

    #[test]
    fn a_narrow_width_wraps_the_message_and_its_advice_under_the_same_column() {
        use unicode_width::UnicodeWidthStr as _;

        // Wide enough that no single word exceeds the room left after the columns, so every
        // wrapped line can genuinely fit. Pre-fix, the longest finding ran past 100 columns.
        let text = render_with_width(&sample(), false, Some(80));

        for line in text.lines() {
            assert!(line.width() <= 80, "every line fits the width: {line}");
        }

        // Nothing is lost to the wrapping: the message and the advice, words in order. Wrapped
        // continuation lines carry their gutter, so whitespace is normalised before matching.
        let folded = text
            .lines()
            .flat_map(|line| line.split_whitespace())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            folded.contains("The instructions name scripts/cull.py, which is not in the bundle"),
            "{folded}"
        );
        assert!(
            folded.contains("What to do: Rename it after what it does."),
            "{folded}"
        );
    }

    #[test]
    fn without_a_usable_width_the_lines_run_at_their_natural_length() {
        let text = render_with_width(&sample(), false, None);

        assert!(
            text.lines().any(|line| line
                .contains("The instructions name scripts/cull.py, which is not in the bundle")),
            "no width to wrap to, so the finding stays on its one line: {text}"
        );
    }

    #[test]
    fn wrapping_keeps_whole_words_and_gives_a_long_word_its_own_line() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap("aaaaa bb", 5), vec!["aaaaa", "bb"]);
        assert_eq!(wrap("already fits", 40), vec!["already fits"]);
        assert_eq!(wrap("", 40), Vec::<String>::new());
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
