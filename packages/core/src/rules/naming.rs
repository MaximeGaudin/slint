//! Rules about the name, which is half of what an agent matches a request against.

use crate::diagnostics::{Location, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

const GENERIC: [&str; 15] = [
    "helper",
    "helpers",
    "utils",
    "util",
    "tools",
    "tool",
    "misc",
    "other",
    "stuff",
    "data",
    "files",
    "documents",
    "general",
    "common",
    "shared",
];

/// Words the specification reserves.
const RESERVED: [&str; 2] = ["anthropic", "claude"];

static FORMAT: RuleMeta = RuleMeta {
    name: "name/format",
    summary: "Skill names must be lowercase, use hyphens, and be at most 64 characters.",
    rationale: "The name is used as an id in listings, fetches, and folder names. The Agent Skills spec only allows [a-z0-9-] so all of those stay in sync.",
    advice: "Change the name to lowercase words separated by single hyphens (example: cull-photos), max 64 characters.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static RESERVED_WORD: RuleMeta = RuleMeta {
    name: "name/no-reserved-word",
    summary: "Do not put reserved vendor words (like anthropic or claude) in the skill name.",
    rationale: "Claude's skill guidelines reserve those words; a skill that uses them can be rejected on upload.",
    advice: "Remove the reserved word from the name. Name the skill after what it does instead.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static GENERIC_NAME: RuleMeta = RuleMeta {
    name: "name/not-generic",
    summary: "The skill name should describe what the skill does.",
    rationale: "Names like helper or utils give the agent nothing to match a user request against, and they collide with other generic skills.",
    advice: "Rename it to the domain plus the action (example: cull-photos, not helper).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

static MATCHES_DIRECTORY: RuleMeta = RuleMeta {
    name: "name/matches-directory",
    summary: "The name in the frontmatter must match the skill folder name.",
    rationale: "Tooling and unpacking use the folder name as the skill address. If it disagrees with the declared name, something breaks wherever the skill is installed next.",
    advice: "Rename either the folder or the name: field in the frontmatter so they are identical.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::OVERVIEW.0,
    reference_url: sources::OVERVIEW.1,
};

struct Format;
struct ReservedWord;
struct Generic;
struct MatchesDirectory;

impl Rule for Format {
    fn meta(&self) -> &'static RuleMeta {
        &FORMAT
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let name = context.skill.name.clone();
        let line = context.skill.frontmatter_line("name");

        // Never declared beats badly spelled: a name backfilled from the directory is display
        // text, and the specification's required field being absent is the finding to report.
        if !context.skill.name_declared || name.trim().is_empty() {
            context.report("The skill has no name", Location::at(line, 1));
            return;
        }

        if name.chars().count() > 64 {
            context.report(
                format!(
                    "The name is {} characters; the limit is 64",
                    name.chars().count()
                ),
                Location::at(line, 1),
            );
        }

        if !name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }) {
            context.report(
                format!("\"{name}\" has characters outside a-z, 0-9 and -"),
                Location::at(line, 1),
            );
        }

        if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
            context.report(
                format!("\"{name}\" has a hyphen where a word should be"),
                Location::at(line, 1),
            );
        }
    }
}

impl Rule for ReservedWord {
    fn meta(&self) -> &'static RuleMeta {
        &RESERVED_WORD
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        // A name the frontmatter never declared is the directory-name fallback, shown so messages
        // say which skill they are about. Policing it would invent a name the author never wrote.
        if !context.skill.name_declared {
            return;
        }

        let name = context.skill.name.to_ascii_lowercase();
        let line = context.skill.frontmatter_line("name");

        for word in RESERVED {
            if name.contains(word) {
                context.report(
                    format!(
                        "\"{}\" contains the reserved word {word}",
                        context.skill.name
                    ),
                    Location::at(line, 1),
                );
            }
        }
    }
}

impl Rule for Generic {
    fn meta(&self) -> &'static RuleMeta {
        &GENERIC_NAME
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        // Same as the reserved words: the fallback name is display text, not a declaration.
        if !context.skill.name_declared {
            return;
        }

        let name = context.skill.name.to_ascii_lowercase();

        if GENERIC.contains(&name.as_str()) {
            let line = context.skill.frontmatter_line("name");
            context.report(
                format!(
                    "\"{}\" says nothing about what this does",
                    context.skill.name
                ),
                Location::at(line, 1),
            );
        }
    }
}

impl Rule for MatchesDirectory {
    fn meta(&self) -> &'static RuleMeta {
        &MATCHES_DIRECTORY
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let Some(directory) = context
            .skill
            .directory
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return;
        };

        // Nothing declared is the missing-name rule's business, not this one's.
        if !context.skill.name_declared || context.skill.name.is_empty() || directory.is_empty() {
            return;
        }

        if context.skill.name != directory {
            let line = context.skill.frontmatter_line("name");
            context.report(
                format!(
                    "The frontmatter says \"{}\" and the directory says \"{directory}\"",
                    context.skill.name
                ),
                Location::at(line, 1),
            );
        }
    }
}

static FORMAT_RULE: Format = Format;
static RESERVED_RULE: ReservedWord = ReservedWord;
static GENERIC_RULE: Generic = Generic;
static DIRECTORY_RULE: MatchesDirectory = MatchesDirectory;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![&FORMAT_RULE, &RESERVED_RULE, &GENERIC_RULE, &DIRECTORY_RULE]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{check, skill_named};

    #[test]
    fn a_well_formed_name_passes() {
        let skill = skill_named("photo-culling");
        assert!(check(&FORMAT_RULE, &skill).is_empty());
        assert!(check(&GENERIC_RULE, &skill).is_empty());
        assert!(check(&RESERVED_RULE, &skill).is_empty());
    }

    #[test]
    fn an_uppercase_name_is_reported() {
        let messages = check(&FORMAT_RULE, &skill_named("PhotoCulling"));
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("outside a-z"));
    }

    #[test]
    fn a_name_with_a_double_hyphen_is_reported() {
        let messages = check(&FORMAT_RULE, &skill_named("photo--culling"));
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("hyphen"));
    }

    #[test]
    fn an_over_long_name_is_reported() {
        let long = "a".repeat(65);
        let messages = check(&FORMAT_RULE, &skill_named(&long));
        assert!(
            messages
                .iter()
                .any(|one| one.message.contains("65 characters"))
        );
    }

    #[test]
    fn an_empty_name_is_reported_once_rather_than_four_times() {
        let mut skill = skill_named("x");
        skill.name = String::new();

        let messages = check(&FORMAT_RULE, &skill);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("no name"));
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/236 — a frontmatter with no
    /// `name:` field must produce the missing-name finding, not pass by borrowing the directory
    /// name for every rule.
    #[test]
    fn a_name_the_frontmatter_never_declared_is_reported_as_missing() {
        let parsed = crate::skill::parse(
            "---\ndescription: Culls a photo shoot in Lightroom by flagging the keepers. Use when triaging RAW files.\n---\n\nBody.\n",
        );
        assert!(!parsed.name_declared);

        let messages = check(&FORMAT_RULE, &parsed);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("no name"));
    }

    #[test]
    fn the_directory_name_fallback_is_display_text_the_other_rules_leave_alone() {
        let mut parsed = crate::skill::parse(
            "---\ndescription: Culls a photo shoot in Lightroom by flagging the keepers. Use when triaging RAW files.\n---\n\nBody.\n",
        );
        parsed.name = "claude-helper".into();

        assert!(check(&RESERVED_RULE, &parsed).is_empty());
        assert!(check(&GENERIC_RULE, &parsed).is_empty());
        assert!(check(&DIRECTORY_RULE, &parsed).is_empty());
    }

    #[test]
    fn a_reserved_word_is_reported() {
        let messages = check(&RESERVED_RULE, &skill_named("claude-helper"));
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("claude"));
        // https://github.com/MaximeGaudin/slint/issues/84 — the reserved-word
        // restriction comes from the Claude docs, not the vendor-neutral spec.
        assert_eq!(
            messages[0].reference.url,
            "https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices",
            "the citation must name the doc that states the restriction: {:?}",
            messages[0].reference
        );
    }

    #[test]
    fn a_generic_name_is_reported() {
        for name in ["helper", "utils", "misc"] {
            let messages = check(&GENERIC_RULE, &skill_named(name));
            assert_eq!(messages.len(), 1, "for {name}");
        }
    }

    #[test]
    fn a_name_that_disagrees_with_its_directory_is_reported() {
        let mut skill = skill_named("photo-culling");
        skill.directory = std::path::PathBuf::from("skills/raw-export");

        let messages = check(&DIRECTORY_RULE, &skill);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("raw-export"));
    }

    #[test]
    fn a_name_that_matches_its_directory_passes() {
        let mut skill = skill_named("photo-culling");
        skill.directory = std::path::PathBuf::from("skills/photo-culling");

        assert!(check(&DIRECTORY_RULE, &skill).is_empty());
    }

    #[test]
    fn every_message_carries_the_rule_and_its_source() {
        let messages = check(&GENERIC_RULE, &skill_named("helper"));

        assert_eq!(messages[0].rule, "name/not-generic");
        assert_eq!(messages[0].severity, Severity::Warning);
        assert!(messages[0].reference.url.starts_with("https://"));
        assert!(!messages[0].advice.is_empty());
    }
}
