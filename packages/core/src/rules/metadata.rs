//! Rules about the frontmatter beyond the name and the description.

use crate::diagnostics::{Location, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

/// Keys the format defines. Anything else is the author's own, which is allowed and worth naming
/// carefully rather than forbidding.
const KNOWN: [&str; 6] = [
    "name",
    "description",
    "license",
    "version",
    "allowed-tools",
    "metadata",
];

static FRONTMATTER: RuleMeta = RuleMeta {
    name: "frontmatter/present",
    summary: "SKILL.md must start with a YAML frontmatter block (--- ... ---).",
    rationale: "Without frontmatter there is no declared name or description, so nothing for the agent to select on.",
    advice: "Start the file with a --- block that at least sets name and description.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static KEY_FORMAT: RuleMeta = RuleMeta {
    name: "frontmatter/key-format",
    summary: "Frontmatter keys should be lowercase with hyphens (no spaces).",
    rationale: "Keys with spaces or odd capitalization often fail to resolve when the skill is loaded.",
    advice: "Rename the key to lowercase-hyphen-form (example: allowed-tools).",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

struct Frontmatter;
struct KeyFormat;

impl Rule for Frontmatter {
    fn meta(&self) -> &'static RuleMeta {
        &FRONTMATTER
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        if !context.skill.has_frontmatter {
            context.report(
                "The document has no frontmatter, so it declares no name and no description",
                Location::at(1, 1),
            );
        }
    }
}

impl Rule for KeyFormat {
    fn meta(&self) -> &'static RuleMeta {
        &KEY_FORMAT
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let keys: Vec<String> = context.skill.metadata.keys().cloned().collect();

        for key in keys {
            let line = context.skill.frontmatter_line(&key);

            if key.contains(' ') {
                context.report(
                    format!("\"{key}\" has a space in it"),
                    Location::at(line, 1),
                );
                continue;
            }

            let lower = key.to_ascii_lowercase();
            if key != lower && KNOWN.contains(&lower.as_str()) {
                context.report(
                    format!("\"{key}\" shadows the defined field \"{lower}\""),
                    Location::at(line, 1),
                );
            }
        }
    }
}

static FRONTMATTER_RULE: Frontmatter = Frontmatter;
static KEY_RULE: KeyFormat = KeyFormat;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![&FRONTMATTER_RULE, &KEY_RULE]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{check, good_skill};
    use crate::skill;

    #[test]
    fn a_document_with_frontmatter_passes() {
        let skill = good_skill();

        assert!(check(&FRONTMATTER_RULE, &skill).is_empty());
        assert!(check(&KEY_RULE, &skill).is_empty());
    }

    #[test]
    fn a_document_with_no_frontmatter_is_an_error() {
        let parsed = skill::parse("# Culling\n\nJust instructions.\n");
        let messages = check(&FRONTMATTER_RULE, &parsed);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
        assert_eq!(messages[0].location.line, 1);
    }

    #[test]
    fn a_key_with_a_space_is_reported() {
        let parsed = skill::parse("---\nname: a\ndescription: b\nmy key: value\n---\n\nBody.\n");
        let messages = check(&KEY_RULE, &parsed);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("my key"));
    }

    #[test]
    fn a_key_that_shadows_a_defined_field_is_reported() {
        let parsed = skill::parse("---\nname: a\ndescription: b\nLicense: MIT\n---\n\nBody.\n");
        let messages = check(&KEY_RULE, &parsed);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("license"));
    }

    #[test]
    fn an_ordinary_custom_key_is_left_alone() {
        let parsed =
            skill::parse("---\nname: a\ndescription: b\nteam: photography\n---\n\nBody.\n");
        assert!(check(&KEY_RULE, &parsed).is_empty());
    }

    /// Regression for #2: product-specific top-level keys are not agentskills.io fields.
    fn unknown_field_messages(source: &str) -> Vec<crate::diagnostics::Message> {
        let parsed = skill::parse(source);
        crate::engine::lint_skill(&parsed, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "frontmatter/unknown-field")
            .collect()
    }

    #[test]
    fn disable_model_invocation_is_an_unknown_field() {
        let messages = unknown_field_messages(
            "---\nname: a\ndescription: b\ndisable-model-invocation: true\n---\n\nBody.\n",
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Warning);
        assert!(messages[0].message.contains("disable-model-invocation"));
    }

    #[test]
    fn compatibility_is_a_recognized_field() {
        let messages = unknown_field_messages(
            "---\nname: a\ndescription: b\ncompatibility: Requires git 2.0+\n---\n\nBody.\n",
        );

        assert!(messages.is_empty());
    }

    #[test]
    fn top_level_version_is_an_unknown_field() {
        let messages =
            unknown_field_messages("---\nname: a\ndescription: b\nversion: \"1.0\"\n---\n\nBody.\n");

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("version"));
    }

    #[test]
    fn product_options_under_metadata_are_not_unknown_fields() {
        let messages = unknown_field_messages(
            "---\nname: a\ndescription: b\nmetadata:\n  disable-model-invocation: \"true\"\n---\n\nBody.\n",
        );

        assert!(messages.is_empty());
    }

    #[test]
    fn an_ordinary_custom_key_is_an_unknown_field() {
        let messages =
            unknown_field_messages("---\nname: a\ndescription: b\nteam: photography\n---\n\nBody.\n");

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("team"));
    }
}
