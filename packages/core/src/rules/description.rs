//! Rules about the description, which is the whole of the routing surface.
//!
//! An agent chooses a skill from its description and nothing else. Almost every "the agent ignored
//! my skill" report is a description report, which is why this file is the densest one here.

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

use crate::diagnostics::{Fix, Location, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

/// The specification's cap.
const MAX_LENGTH: usize = 1024;

static FIRST_OR_SECOND_PERSON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(I can|I will|I'll|I am|I help|we can|we will|you can|you should|you may|your |use me\b|this skill (lets|helps) you)",
    )
    .expect("the person pattern compiles")
});

/// A situational clause. Deliberately generous: a false positive here nags a good skill, so the bar
/// is "does it mention a circumstance at all" rather than any particular phrasing.
static TRIGGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(use (this )?(when|for|if)|when the user|when asked|whenever|invoke when|triggers? on|apply when|reach for this when|for (drafting|writing|reviewing|planning|analysing|analyzing|processing|debugging|handling|triaging))",
    )
    .expect("the trigger pattern compiles")
});

/// HTML tags, plus Markdown emphasis: bold, italic. Emphasis matches carry their inner text as a
/// capture group so the fix can strip the delimiters and keep the word.
static MARKUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<[^>]+>|\*\*([^*\n]+)\*\*|\*([^*\s][^*\n]*)\*")
        .expect("the markup pattern compiles")
});

/// Markdown emphasis written with underscores. `_em_` is emphasis, but `my_file_name` is an
/// identifier, so — like CommonMark — intraword underscore matches are left alone.
static UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_([^_\s]+)_").expect("the underscore emphasis pattern compiles"));

fn is_intraword(description: &str, start: usize, end: usize) -> bool {
    let before = description[..start].chars().next_back();
    let after = description[end..].chars().next();

    before.is_some_and(|character| character.is_alphanumeric())
        || after.is_some_and(|character| character.is_alphanumeric())
}

fn has_underscore_emphasis(description: &str) -> bool {
    UNDERSCORE.captures_iter(description).any(|captures| {
        let whole = captures.get(0).expect("a match always has group 0");
        !is_intraword(description, whole.start(), whole.end())
    })
}

/// The description with every tag and emphasis delimiter gone and the wrapped words kept.
fn strip_markup(description: &str) -> String {
    let without_underscores =
        UNDERSCORE.replace_all(description, |captures: &regex::Captures<'_>| {
            let whole = captures.get(0).expect("a match always has group 0");
            if is_intraword(description, whole.start(), whole.end()) {
                whole.as_str().to_string()
            } else {
                captures[1].to_string()
            }
        });

    MARKUP
        .replace_all(&without_underscores, |captures: &regex::Captures<'_>| {
            captures
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map(|found| found.as_str().to_string())
                .unwrap_or_default()
        })
        .to_string()
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct LengthOptions {
    /// Below this, a description cannot carry both what it does and when to use it.
    min: usize,
}

impl Default for LengthOptions {
    fn default() -> Self {
        LengthOptions { min: 80 }
    }
}

static PRESENT: RuleMeta = RuleMeta {
    name: "description/present",
    summary: "Every skill must have a description in the frontmatter.",
    rationale: "The description is the only text the agent sees before deciding whether to load the skill. Without it, the skill is never selected.",
    advice: "Add a description field that says what the skill does and when to use it, using words a user request would contain.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static MAX_LENGTH_META: RuleMeta = RuleMeta {
    name: "description/max-length",
    summary: "Keep the description at or under 1024 characters.",
    rationale: "Descriptions are injected into the system prompt. Over 1024 characters they get truncated — often cutting off the \"when to use\" part at the end.",
    advice: "Shorten the description to under 1024 characters. Put what it does first and when to use it last.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static NO_MARKUP: RuleMeta = RuleMeta {
    name: "description/no-markup",
    summary: "The description must be plain text — no HTML or Markdown tags.",
    rationale: "The description is injected as plain text into a prompt. Tags are read as words, not markup, and the spec forbids them.",
    advice: "Remove any tags (for example <b> or **bold**). Write a normal sentence.",
    default_severity: Severity::Error,
    fixable: true,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static MIN_LENGTH: RuleMeta = RuleMeta {
    name: "description/min-length",
    summary: "The description must be long enough to explain what the skill does and when to use it.",
    rationale: "The agent chooses among all available skills using this text alone. A few words cannot carry both the purpose and the trigger.",
    advice: "Expand to at least 80 characters — what it does, then when to use it, in the words a request would use.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static THIRD_PERSON: RuleMeta = RuleMeta {
    name: "description/third-person",
    summary: "Write the description in the third person (\"Processes Excel files\"), not first person.",
    rationale: "Descriptions are injected into a system prompt. Mixing \"I can help…\" with third-person skills makes selection less reliable.",
    advice: "Rewrite as what the skill does (\"Converts PDFs to Markdown\"), not \"I can help you convert PDFs\".",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static SAYS_WHEN: RuleMeta = RuleMeta {
    name: "description/says-when",
    summary: "The description must say when to use the skill.",
    rationale: "Without a trigger clause, the agent has to guess relevance from a summary. That is the most common reason a skill never gets selected.",
    advice: "Add a clause like \"Use when …\" with the situation, using phrasing a real user request would use.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static NOT_JUST_NAME: RuleMeta = RuleMeta {
    name: "description/not-just-name",
    summary: "The description must add information beyond repeating the skill name.",
    rationale: "The name is already visible to the agent. A description that only restates it gives no extra signal for routing.",
    advice: "Add inputs, situation, or user wording the name does not already contain.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static CONCRETE_NOUN: RuleMeta = RuleMeta {
    name: "description/concrete-noun",
    summary: "The description should name concrete tools, formats, or systems users would mention.",
    rationale: "Matching is word-based. A YouTube-transcript skill whose description never says \"YouTube\" will not match \"transcribe this YouTube video\".",
    advice: "Include the specific nouns users type (YouTube, Excel, PDF) — not only vague categories like \"documents\".",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: false,
    reference_title: sources::ENGINEERING.0,
    reference_url: sources::ENGINEERING.1,
};

struct Present;
struct MaxLength;
struct NoMarkup;
struct MinLength;
struct ThirdPerson;
struct SaysWhen;
struct NotJustName;
struct ConcreteNoun;

/// Where the description is written, for every rule in this file.
fn at(context: &RuleContext<'_>) -> Location {
    Location::at(context.skill.frontmatter_line("description"), 1)
}

impl Rule for Present {
    fn meta(&self) -> &'static RuleMeta {
        &PRESENT
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        if context.skill.description.trim().is_empty() {
            context.report("The skill has no description", at(context));
        }
    }
}

impl Rule for MaxLength {
    fn meta(&self) -> &'static RuleMeta {
        &MAX_LENGTH_META
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let length = context.skill.description.chars().count();

        if length > MAX_LENGTH {
            context.report(
                format!("The description is {length} characters; the limit is {MAX_LENGTH}"),
                at(context),
            );
        }
    }
}

impl Rule for NoMarkup {
    fn meta(&self) -> &'static RuleMeta {
        &NO_MARKUP
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let description = context.skill.description.clone();
        if !MARKUP.is_match(&description) && !has_underscore_emphasis(&description) {
            return;
        }

        let cleaned = strip_markup(&description);
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

        let location = at(context);
        match span_of(context, &description) {
            Some((start, end)) => context.report_fixable(
                "The description contains markup",
                location,
                Fix {
                    start,
                    end,
                    replacement: cleaned,
                    description: "Removes the tags and closes the gap they leave.".into(),
                },
            ),
            None => context.report("The description contains markup", location),
        }
    }
}

impl Rule for MinLength {
    fn meta(&self) -> &'static RuleMeta {
        &MIN_LENGTH
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let options: LengthOptions = context.option();
        let trimmed = context.skill.description.trim().to_string();
        let length = trimmed.chars().count();

        // An absent description is the other rule's business; this one is about a thin one.
        if length > 0 && length < options.min {
            context.report(
                format!(
                    "The description is {length} characters, under the {} it needs",
                    options.min
                ),
                at(context),
            );
        }
    }
}

impl Rule for ThirdPerson {
    fn meta(&self) -> &'static RuleMeta {
        &THIRD_PERSON
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        if FIRST_OR_SECOND_PERSON.is_match(&context.skill.description) {
            context.report("The description addresses the reader", at(context));
        }
    }
}

impl Rule for SaysWhen {
    fn meta(&self) -> &'static RuleMeta {
        &SAYS_WHEN
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let description = context.skill.description.trim().to_string();

        if !description.is_empty() && !TRIGGER.is_match(&description) {
            context.report("The description never says when to use this", at(context));
        }
    }
}

impl Rule for NotJustName {
    fn meta(&self) -> &'static RuleMeta {
        &NOT_JUST_NAME
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let name = context.skill.name.replace('-', " ").to_ascii_lowercase();
        let description = context.skill.description.trim().to_ascii_lowercase();

        if name.is_empty() || description.is_empty() {
            return;
        }

        let stripped = description
            .replace(&name, "")
            .chars()
            .filter(|character| character.is_alphanumeric())
            .count();

        // What is left once the name is removed is what the description actually contributed.
        if stripped < 24 {
            context.report("The description repeats the name and stops", at(context));
        }
    }
}

impl Rule for ConcreteNoun {
    fn meta(&self) -> &'static RuleMeta {
        &CONCRETE_NOUN
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let description = context.skill.description.trim().to_string();
        if description.is_empty() {
            return;
        }

        // A proper noun, an extension, a path or a CamelCase product name. One is enough: the rule
        // is looking for any anchor a request could share, not for a well-stocked vocabulary.
        let has_anchor = description
            .split_whitespace()
            .enumerate()
            .any(|(index, word)| {
                let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.');

                looks_like_a_file(cleaned)
                    // An acronym: RAW, PDF, API.
                    || cleaned.chars().filter(|c| c.is_uppercase()).count() > 1
                    // A proper noun: Lightroom, Excel, YouTube. Not in the first position:
                    // there the capital is usually the leading verb ("Processes"), which this
                    // heuristic cannot tell from a product name.
                    || (index > 0
                        && cleaned.chars().next().is_some_and(|c| c.is_uppercase())
                        && cleaned.chars().skip(1).any(|c| c.is_lowercase()))
            });

        if !has_anchor {
            context.report(
                "The description names nothing specific a request would contain",
                at(context),
            );
        }
    }
}

/// Whether a word is a filename or an extension rather than a sentence ending in a full stop.
fn looks_like_a_file(word: &str) -> bool {
    let Some((stem, extension)) = word.rsplit_once('.') else {
        return false;
    };

    !stem.is_empty()
        && (1..=5).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The byte range of the description's value inside the document, for a fix.
fn span_of(context: &RuleContext<'_>, description: &str) -> Option<(usize, usize)> {
    let start = context.skill.source.find(description)?;
    Some((start, start + description.len()))
}

static PRESENT_RULE: Present = Present;
static MAX_LENGTH_RULE: MaxLength = MaxLength;
static NO_MARKUP_RULE: NoMarkup = NoMarkup;
static MIN_LENGTH_RULE: MinLength = MinLength;
static THIRD_PERSON_RULE: ThirdPerson = ThirdPerson;
static SAYS_WHEN_RULE: SaysWhen = SaysWhen;
static NOT_JUST_NAME_RULE: NotJustName = NotJustName;
static CONCRETE_NOUN_RULE: ConcreteNoun = ConcreteNoun;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![
        &PRESENT_RULE,
        &MAX_LENGTH_RULE,
        &NO_MARKUP_RULE,
        &MIN_LENGTH_RULE,
        &THIRD_PERSON_RULE,
        &SAYS_WHEN_RULE,
        &NOT_JUST_NAME_RULE,
        &CONCRETE_NOUN_RULE,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, RuleSetting};
    use crate::rules::testing::{check, check_with, good_skill, skill_described};

    #[test]
    fn a_good_description_passes_every_rule_here() {
        let skill = good_skill();

        for rule in rules() {
            assert!(
                check(rule, &skill).is_empty(),
                "{} fired on a good description",
                rule.meta().name
            );
        }
    }

    #[test]
    fn a_missing_description_is_an_error() {
        let messages = check(&PRESENT_RULE, &skill_described(""));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
    }

    #[test]
    fn a_missing_description_does_not_also_fire_the_softer_rules() {
        let skill = skill_described("");

        assert!(check(&MIN_LENGTH_RULE, &skill).is_empty());
        assert!(check(&SAYS_WHEN_RULE, &skill).is_empty());
        assert!(check(&NOT_JUST_NAME_RULE, &skill).is_empty());
        assert!(check(&CONCRETE_NOUN_RULE, &skill).is_empty());
    }

    #[test]
    fn a_description_over_the_specification_limit_is_reported() {
        let long = "word ".repeat(300);
        let messages = check(&MAX_LENGTH_RULE, &skill_described(&long));

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("1024"));
    }

    #[test]
    fn markup_is_reported_and_fixed_by_removing_it() {
        let skill = skill_described("Culls a <b>photo</b> shoot. Use when triaging RAW files.");
        let messages = check(&NO_MARKUP_RULE, &skill);

        assert_eq!(messages.len(), 1);
        let fix = messages[0].fix.as_ref().expect("markup is fixable");
        assert_eq!(
            fix.replacement,
            "Culls a photo shoot. Use when triaging RAW files."
        );
    }

    #[test]
    fn a_fix_replaces_only_the_description_inside_the_document() {
        let skill = skill_described("Culls a <b>photo</b> shoot. Use when triaging RAW files.");
        let messages = check(&NO_MARKUP_RULE, &skill);
        let fix = messages[0].fix.as_ref().unwrap();

        // The fix is a byte range into SKILL.md, and applying it must leave the rest untouched.
        let mut patched = skill.source.clone();
        patched.replace_range(fix.start..fix.end, &fix.replacement);

        assert!(patched.starts_with("---\nname: photo-culling"));
        assert!(!patched.contains("<b>"));
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/80 —
    /// the rule's own advice cites **bold** as markup, so Markdown emphasis must be detected
    /// the same way an HTML tag is, and removed while keeping the word it wrapped.
    #[test]
    fn markdown_bold_is_reported_and_fixed_like_html_tags() {
        let skill = skill_described(
            "Culls a **photo** shoot in Lightroom by flagging keepers. Use when triaging RAW files after a session.",
        );
        let messages = check(&NO_MARKUP_RULE, &skill);

        assert_eq!(messages.len(), 1);
        let fix = messages[0].fix.as_ref().expect("emphasis is fixable");
        assert_eq!(
            fix.replacement,
            "Culls a photo shoot in Lightroom by flagging keepers. Use when triaging RAW files after a session."
        );
    }

    #[test]
    fn italic_and_underscore_emphasis_are_reported() {
        for description in [
            "Culls a *photo* shoot in Lightroom by flagging keepers. Use when triaging RAW files.",
            "Culls a _photo_ shoot in Lightroom by flagging keepers. Use when triaging RAW files.",
        ] {
            assert_eq!(
                check(&NO_MARKUP_RULE, &skill_described(description)).len(),
                1,
                "for {description}"
            );
        }
    }

    #[test]
    fn snake_case_identifiers_are_not_markup() {
        let skill = skill_described(
            "Reads each my_file_name dump in the folder. Use when parsing raw editor exports.",
        );

        assert!(check(&NO_MARKUP_RULE, &skill).is_empty());
    }

    #[test]
    fn a_thin_description_is_reported_against_the_configured_minimum() {
        let skill = skill_described("Culls photos. Use when triaging.");
        assert_eq!(check(&MIN_LENGTH_RULE, &skill).len(), 1);

        let mut config = Config::default();
        config.rules.insert(
            "description/min-length".into(),
            RuleSetting::Tuned(Severity::Warning, serde_json::json!({ "min": 10 })),
        );

        assert!(check_with(&MIN_LENGTH_RULE, &skill, &config).is_empty());
    }

    /// https://github.com/MaximeGaudin/slint/issues/79 — the advice used to say
    /// "roughly 150–300 characters" while the rule only fires below 80, and no
    /// source states 150–300. The advice must match the shipped default.
    #[test]
    fn the_min_length_advice_matches_the_configured_default() {
        assert!(
            MIN_LENGTH.advice.contains("80"),
            "advice must name the shipped default of 80: {}",
            MIN_LENGTH.advice
        );
        assert!(
            !MIN_LENGTH.advice.contains("150"),
            "advice must not advertise a range no source states: {}",
            MIN_LENGTH.advice
        );
    }

    #[test]
    fn first_and_second_person_are_reported() {
        for description in [
            "I can help you cull a photo shoot. Use when triaging RAW files.",
            "You should use this when triaging RAW files after a shoot in Lightroom.",
            "This skill helps you cull a shoot. Use when triaging RAW files.",
        ] {
            let messages = check(&THIRD_PERSON_RULE, &skill_described(description));
            assert_eq!(messages.len(), 1, "for {description}");
        }
    }

    #[test]
    fn a_description_with_no_trigger_clause_is_reported() {
        let skill = skill_described(
            "Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest of the frames.",
        );

        assert_eq!(check(&SAYS_WHEN_RULE, &skill).len(), 1);
    }

    #[test]
    fn several_phrasings_all_count_as_a_trigger_clause() {
        for description in [
            "Culls a shoot in Lightroom. Use when triaging RAW files after a session.",
            "Culls a shoot in Lightroom, whenever a session needs narrowing to selects.",
            "Culls a shoot in Lightroom. Reach for this when a shoot needs narrowing down.",
        ] {
            assert!(
                check(&SAYS_WHEN_RULE, &skill_described(description)).is_empty(),
                "for {description}"
            );
        }
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/242 — "Culls" and "culling"
    /// are forms of "cull", "photos" is "photo": the description said nothing beyond the name,
    /// but the exact substring removal matched nothing and the leftovers cleared the floor.
    #[test]
    fn a_description_restating_the_name_in_other_word_forms_is_reported() {
        let mut skill = skill_described("Culls photos. Use when culling.");
        skill.name = "cull-photos".into();

        assert_eq!(check(&NOT_JUST_NAME_RULE, &skill).len(), 1);
    }

    #[test]
    fn a_description_that_only_restates_the_name_is_reported() {
        let mut skill = skill_described("Photo culling. Use when culling photos.");
        skill.name = "photo-culling".into();

        assert_eq!(check(&NOT_JUST_NAME_RULE, &skill).len(), 1);
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/241 — the first word was
    /// skipped unconditionally, so a description whose only concrete anchor opens it was
    /// reported as naming nothing specific.
    #[test]
    fn a_leading_acronym_counts_as_a_concrete_noun() {
        let skill = skill_described(
            "PDF conversion for scanned documents into contact sheets for client review; use when a client requests proofs from a shoot.",
        );

        assert!(check(&CONCRETE_NOUN_RULE, &skill).is_empty());
    }

    #[test]
    fn a_description_with_no_concrete_noun_is_reported() {
        let skill = skill_described(
            "handles the relevant items in the usual way. use when the situation calls for it and things need doing.",
        );

        assert_eq!(check(&CONCRETE_NOUN_RULE, &skill).len(), 1);
    }

    /// The skip the fix above replaced existed for a reason: the first word of a third-person
    /// description is usually the verb, and the proper-noun heuristic would happily read
    /// "Processes" as a product name. Position 0 still only counts anchors a verb cannot be.
    #[test]
    fn a_leading_verb_is_still_not_an_anchor() {
        let skill = skill_described(
            "Processes the relevant items in the usual way, and finishes whenever the situation calls for it and things need doing.",
        );

        assert_eq!(check(&CONCRETE_NOUN_RULE, &skill).len(), 1);
    }

    #[test]
    fn a_file_extension_counts_as_something_concrete() {
        let skill = skill_described(
            "Converts a shoot into contact sheets as sheet.pdf files for review. Use when a client asks for proofs.",
        );

        assert!(check(&CONCRETE_NOUN_RULE, &skill).is_empty());
    }
}
