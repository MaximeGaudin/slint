//! Rules that can only be answered by looking at every skill at once.
//!
//! Two descriptions that compete for the same request are not visible from inside either of them.
//! This is the class of problem a per-file linter structurally cannot see, and the reason slint
//! reads the whole set before it reports anything.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::config::Config;
use crate::diagnostics::{Location, Message, Severity, Source};
use crate::rules::{ProjectRule, RuleMeta, sources};
use crate::skill::Skill;

static UNIQUE_NAME: RuleMeta = RuleMeta {
    name: "project/unique-name",
    summary: "No two skills in the project may share the same name.",
    rationale: "The name is an address. When two skills share one, the host resolves the conflict by source — Claude Code, for example, lets a personal skill override a project skill of the same name — so one skill silently stops loading.",
    advice: "Rename one skill so each name is unique and describes what makes that skill different.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::CLAUDE_CODE.0,
    reference_url: sources::CLAUDE_CODE.1,
};

static DISTINCT_DESCRIPTIONS: RuleMeta = RuleMeta {
    name: "project/distinct-descriptions",
    summary: "Skill descriptions in the same project should not sound like the same offer.",
    rationale: "Nearly identical descriptions make skill selection a coin toss; one skill will rarely be chosen.",
    advice: "Rewrite the overlapping descriptions so the first clause states what each skill covers that the other does not.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static CONSISTENT_NAMING: RuleMeta = RuleMeta {
    name: "project/consistent-naming-style",
    summary: "Skill names in the same project should follow one naming convention.",
    rationale: "The best-practices guide lists inconsistent patterns within a skill collection as an anti-pattern. Gerund names beside noun-phrase names read as an accident and make the collection harder to scan.",
    advice: "Pick one convention for the whole collection — the guide suggests the gerund form (verb + -ing, as in processing-pdfs) — and rename the outliers to match.",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

struct UniqueName;
struct DistinctDescriptions;
struct ConsistentNamingStyle;

/// The two naming styles the heuristic can tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// A verb form: "processing-pdfs", "culling-photos".
    Gerund,
    /// Everything else: noun phrases and imperative names, which this heuristic does not
    /// attempt to tell apart.
    Other,
}

/// Words that end in "ing" without being verb forms.
const NOT_GERUND: [&str; 19] = [
    "string",
    "thing",
    "ring",
    "king",
    "sing",
    "wing",
    "spring",
    "swing",
    "bring",
    "cling",
    "fling",
    "sting",
    "during",
    "morning",
    "evening",
    "nothing",
    "something",
    "anything",
    "everything",
];

/// A simple, documented heuristic: a name whose first hyphen-separated word is a plain verb in its
/// "-ing" form is gerund style; every other name — noun phrase or imperative — is not. The rule
/// never says which style is right, only that the collection should settle on one.
fn style_of(name: &str) -> Style {
    let first = name.split('-').next().unwrap_or(name).to_ascii_lowercase();

    let gerund =
        first.len() >= 5 && first.ends_with("ing") && !NOT_GERUND.contains(&first.as_str());

    if gerund { Style::Gerund } else { Style::Other }
}

impl ProjectRule for UniqueName {
    fn meta(&self) -> &'static RuleMeta {
        &UNIQUE_NAME
    }

    fn check(&self, skills: &[Skill], _config: &Config, severity: Severity) -> Vec<Message> {
        let mut by_name: BTreeMap<&str, Vec<&Skill>> = BTreeMap::new();

        for skill in skills {
            if !skill.name.is_empty() {
                by_name.entry(skill.name.as_str()).or_default().push(skill);
            }
        }

        let mut messages = Vec::new();

        for (name, sharing) in by_name {
            if sharing.len() < 2 {
                continue;
            }

            for skill in &sharing {
                let others: Vec<String> = sharing
                    .iter()
                    .filter(|other| other.directory != skill.directory)
                    .map(|other| other.directory.display().to_string())
                    .collect();

                messages.push(Message {
                    rule: UNIQUE_NAME.name.to_string(),
                    severity,
                    message: format!("\"{name}\" is also declared by {}", others.join(", ")),
                    advice: UNIQUE_NAME.advice.to_string(),
                    location: Location::at(skill.frontmatter_line("name"), 1),
                    source: Source::Static,
                    file: skill.document.clone(),
                    fix: None,
                    reference: UNIQUE_NAME.reference(),
                    confidence: 1.0,
                });
            }
        }

        messages
    }
}

impl ProjectRule for DistinctDescriptions {
    fn meta(&self) -> &'static RuleMeta {
        &DISTINCT_DESCRIPTIONS
    }

    fn check(&self, skills: &[Skill], config: &Config, severity: Severity) -> Vec<Message> {
        let threshold = config
            .options_for(DISTINCT_DESCRIPTIONS.name)
            .and_then(|options| options.get("similarity"))
            .and_then(|value| value.as_f64())
            .unwrap_or(0.8);

        let mut messages = Vec::new();

        for (index, skill) in skills.iter().enumerate() {
            if skill.description.trim().is_empty() {
                continue;
            }

            for other in skills.iter().skip(index + 1) {
                if other.description.trim().is_empty() {
                    continue;
                }

                if similarity(&skill.description, &other.description) < threshold {
                    continue;
                }

                for (one, two) in [(skill, other), (other, skill)] {
                    messages.push(Message {
                        rule: DISTINCT_DESCRIPTIONS.name.to_string(),
                        severity,
                        message: format!("The description reads almost the same as {}'s", two.name),
                        advice: DISTINCT_DESCRIPTIONS.advice.to_string(),
                        location: Location::at(one.frontmatter_line("description"), 1),
                        source: Source::Static,
                        file: one.document.clone(),
                        fix: None,
                        reference: DISTINCT_DESCRIPTIONS.reference(),
                        confidence: 1.0,
                    });
                }
            }
        }

        messages
    }
}

impl ProjectRule for ConsistentNamingStyle {
    fn meta(&self) -> &'static RuleMeta {
        &CONSISTENT_NAMING
    }

    fn check(&self, skills: &[Skill], _config: &Config, severity: Severity) -> Vec<Message> {
        let mut gerunds: Vec<&Skill> = Vec::new();
        let mut others: Vec<&Skill> = Vec::new();

        for skill in skills {
            if skill.name.is_empty() {
                continue;
            }

            if style_of(&skill.name) == Style::Gerund {
                gerunds.push(skill);
            } else {
                others.push(skill);
            }
        }

        // One style or fewer than two names is a collection that has nothing to be inconsistent with.
        if gerunds.is_empty() || others.is_empty() {
            return Vec::new();
        }

        let mut messages = Vec::new();

        // The minority is flagged, so one odd name does not drag the whole collection into the
        // report. On a tie no convention is in force yet, so every name hears about it.
        match gerunds.len().cmp(&others.len()) {
            Ordering::Greater => push_naming_messages(&mut messages, &others, &gerunds, severity),
            Ordering::Less => push_naming_messages(&mut messages, &gerunds, &others, severity),
            Ordering::Equal => {
                push_naming_messages(&mut messages, &gerunds, &others, severity);
                push_naming_messages(&mut messages, &others, &gerunds, severity);
            }
        }

        messages
    }
}

/// Reports every flagged skill against up to three names written in the other style.
fn push_naming_messages(
    messages: &mut Vec<Message>,
    flagged: &[&Skill],
    against: &[&Skill],
    severity: Severity,
) {
    let examples: Vec<&str> = against
        .iter()
        .take(3)
        .map(|skill| skill.name.as_str())
        .collect();

    let flagged_are_gerunds = style_of(&flagged[0].name) == Style::Gerund;
    let verb = if examples.len() == 1 { "is" } else { "are" };

    for skill in flagged {
        let message = if flagged_are_gerunds {
            format!(
                "The name \"{}\" is a gerund (verb + -ing), but {} {verb} not",
                skill.name,
                examples.join(", ")
            )
        } else {
            format!(
                "The name \"{}\" is not a gerund (verb + -ing), but {} {verb}",
                skill.name,
                examples.join(", ")
            )
        };

        messages.push(Message {
            rule: CONSISTENT_NAMING.name.to_string(),
            severity,
            message,
            advice: CONSISTENT_NAMING.advice.to_string(),
            location: Location::at(skill.frontmatter_line("name"), 1),
            source: Source::Static,
            file: skill.document.clone(),
            fix: None,
            reference: CONSISTENT_NAMING.reference(),
            confidence: 1.0,
        });
    }
}

/// How much two descriptions overlap, as a fraction of the shorter one's words.
///
/// A word-set overlap rather than an edit distance: what matters is whether they compete for the
/// same request, and two sentences can be worded quite differently while offering exactly the same
/// thing. Very short words are dropped, since "the" agreeing is not evidence of anything.
pub fn similarity(left: &str, right: &str) -> f64 {
    let words = |text: &str| -> std::collections::BTreeSet<String> {
        text.to_ascii_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.len() > 3)
            .map(|word| word.to_string())
            .collect()
    };

    let left_words = words(left);
    let right_words = words(right);

    if left_words.is_empty() || right_words.is_empty() {
        return 0.0;
    }

    let shared = left_words.intersection(&right_words).count() as f64;
    let smallest = left_words.len().min(right_words.len()) as f64;

    shared / smallest
}

static UNIQUE_NAME_RULE: UniqueName = UniqueName;
static DISTINCT_RULE: DistinctDescriptions = DistinctDescriptions;
static NAMING_RULE: ConsistentNamingStyle = ConsistentNamingStyle;

pub fn rules() -> Vec<&'static dyn ProjectRule> {
    vec![&UNIQUE_NAME_RULE, &DISTINCT_RULE, &NAMING_RULE]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::good_skill;

    fn skill(name: &str, description: &str) -> Skill {
        let mut parsed = good_skill();
        parsed.name = name.into();
        parsed.description = description.into();
        parsed.directory = std::path::PathBuf::from(format!("skills/{name}"));
        parsed.document = format!("skills/{name}/SKILL.md");
        parsed
    }

    #[test]
    fn distinct_skills_pass() {
        let skills = vec![
            skill(
                "photo-culling",
                "Culls a photo shoot in Lightroom. Use when triaging RAW files.",
            ),
            skill(
                "invoice-drafting",
                "Drafts an invoice from a timesheet in Numbers. Use when billing a client.",
            ),
        ];

        let config = Config::default();
        assert!(
            UNIQUE_NAME_RULE
                .check(&skills, &config, Severity::Error)
                .is_empty()
        );
        assert!(
            DISTINCT_RULE
                .check(&skills, &config, Severity::Warning)
                .is_empty()
        );
    }

    #[test]
    fn two_skills_with_one_name_are_both_reported() {
        let mut first = skill("photo-culling", "One description about culling RAW shoots.");
        first.directory = std::path::PathBuf::from("skills/a");
        let mut second = skill(
            "photo-culling",
            "A different description about invoices entirely.",
        );
        second.directory = std::path::PathBuf::from("skills/b");

        let messages =
            UNIQUE_NAME_RULE.check(&[first, second], &Config::default(), Severity::Error);

        assert_eq!(messages.len(), 2, "both files hear about it");
        assert!(messages[0].message.contains("skills/b"));
        assert!(messages[1].message.contains("skills/a"));
        // https://github.com/MaximeGaudin/slint/issues/112 — the specification
        // covers a single skill; name collisions are a host concern.
        assert_eq!(
            messages[0].reference.url, "https://code.claude.com/docs/en/skills",
            "the citation must name the doc that discusses same-name skills: {:?}",
            messages[0].reference
        );
    }

    #[test]
    fn near_identical_descriptions_are_reported_on_both_sides() {
        let skills = vec![
            skill(
                "photo-culling",
                "Culls a photo shoot in Lightroom by flagging keepers. Use when triaging RAW files after a session.",
            ),
            skill(
                "shoot-triage",
                "Culls a photo shoot in Lightroom by flagging keepers. Use when triaging RAW files after a shoot.",
            ),
        ];

        let messages = DISTINCT_RULE.check(&skills, &Config::default(), Severity::Warning);

        assert_eq!(messages.len(), 2);
        assert!(messages[0].message.contains("shoot-triage"));
        assert!(messages[1].message.contains("photo-culling"));
    }

    #[test]
    fn the_similarity_threshold_can_be_configured() {
        let skills = vec![
            skill(
                "a",
                "Culls a photo shoot in Lightroom by flagging keepers of a session.",
            ),
            skill(
                "b",
                "Culls a photo shoot in Capture One by rating frames of a session.",
            ),
        ];

        let mut config = Config::default();
        config.rules.insert(
            "project/distinct-descriptions".into(),
            crate::config::RuleSetting::Tuned(
                Severity::Warning,
                serde_json::json!({ "similarity": 0.4 }),
            ),
        );

        assert!(
            DISTINCT_RULE
                .check(&skills, &Config::default(), Severity::Warning)
                .is_empty()
        );
        assert_eq!(
            DISTINCT_RULE
                .check(&skills, &config, Severity::Warning)
                .len(),
            2
        );
    }

    #[test]
    fn similarity_ignores_short_words_and_case() {
        assert!(similarity("Culls a photo shoot", "culls a PHOTO shoot") > 0.99);
        assert_eq!(similarity("", "anything at all"), 0.0);
        assert!(similarity("photo culling shoots", "invoice drafting clients") < 0.1);
    }

    #[test]
    fn a_skill_with_no_description_is_not_compared() {
        let skills = vec![skill("a", ""), skill("b", "")];
        assert!(
            DISTINCT_RULE
                .check(&skills, &Config::default(), Severity::Warning)
                .is_empty()
        );
    }

    /// Regression for #93: nothing compared naming style across a project's skills.
    fn consistent_naming_messages(skills: &[Skill]) -> Vec<Message> {
        crate::engine::lint_project(skills, &Config::default())
            .into_iter()
            .filter(|message| message.rule == "project/consistent-naming-style")
            .collect()
    }

    #[test]
    fn a_collection_mixing_naming_styles_is_reported() {
        let skills = vec![
            skill(
                "processing-pdfs",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "extract-pdf-text",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "cull-photos",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
        ];

        let messages = consistent_naming_messages(&skills);

        // The lone gerund name is the outlier; the two imperative names are the majority.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Info);
        assert_eq!(
            messages[0].message,
            "The name \"processing-pdfs\" is a gerund (verb + -ing), but extract-pdf-text, cull-photos are not"
        );
        assert_eq!(messages[0].file, "skills/processing-pdfs/SKILL.md");
    }

    #[test]
    fn a_consistently_gerund_collection_passes() {
        let skills = vec![
            skill(
                "processing-pdfs",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "culling-photos",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
        ];

        assert!(consistent_naming_messages(&skills).is_empty());
    }

    #[test]
    fn a_consistently_non_gerund_collection_passes() {
        let skills = vec![
            skill(
                "extract-pdf-text",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "pdf-export",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "invoice-builder",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
        ];

        assert!(consistent_naming_messages(&skills).is_empty());
    }

    #[test]
    fn a_single_skill_is_never_reported() {
        let skills = vec![skill(
            "processing-pdfs",
            "Culls a photo shoot. Use when triaging RAW files.",
        )];

        assert!(consistent_naming_messages(&skills).is_empty());
    }

    #[test]
    fn words_that_merely_end_in_ing_are_not_gerunds() {
        let skills = vec![
            skill(
                "string-utils",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
            skill(
                "thing-counter",
                "Culls a photo shoot. Use when triaging RAW files.",
            ),
        ];

        assert!(consistent_naming_messages(&skills).is_empty());
    }
}
