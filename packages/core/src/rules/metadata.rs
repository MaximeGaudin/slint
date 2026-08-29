//! Rules about the frontmatter beyond the name and the description.

use crate::diagnostics::{Location, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, sources};

/// The specification's cap on the optional compatibility field.
const COMPATIBILITY_MAX_LENGTH: usize = 500;

/// Top-level frontmatter fields recognized by the Agent Skills specification.
/// Product-specific options belong under `metadata`, not as new top-level keys.
const SPEC_FIELDS: [&str; 6] = [
    "name",
    "description",
    "license",
    "compatibility",
    "metadata",
    "allowed-tools",
];

/// Top-level frontmatter fields Claude Code documents as first-class extensions
/// (https://code.claude.com/docs/en/skills). They are recognized rather than flagged, even
/// though other hosts may ignore them.
const CLAUDE_FIELDS: [&str; 14] = [
    "disable-model-invocation",
    "user-invocable",
    "disallowed-tools",
    "model",
    "effort",
    "context",
    "agent",
    "background",
    "hooks",
    "paths",
    "shell",
    "argument-hint",
    "arguments",
    "when_to_use",
];

fn is_known_field(key: &str) -> bool {
    SPEC_FIELDS.contains(&key) || CLAUDE_FIELDS.contains(&key)
}

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

static UNKNOWN_FIELD: RuleMeta = RuleMeta {
    name: "frontmatter/unknown-field",
    summary: "Top-level frontmatter keys must be agentskills.io fields, Claude Code extensions, or live under metadata.",
    rationale: "Product-specific keys at the top level are invisible or rejected on other hosts. The spec keeps its own top-level set closed; Claude Code's documented extensions are recognized, and anything else belongs under metadata.",
    advice: "Remove the key, or move product-specific options under metadata (example: metadata.version). Recognized top-level fields: name, description, license, compatibility, metadata, allowed-tools, plus Claude Code's extensions (disable-model-invocation, user-invocable, disallowed-tools, model, effort, context, agent, background, hooks, paths, shell, argument-hint, arguments, when_to_use).",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

static COMPATIBILITY_MAX_LENGTH_META: RuleMeta = RuleMeta {
    name: "frontmatter/compatibility-max-length",
    summary: "Keep the compatibility field at or under 500 characters.",
    rationale: "The specification caps compatibility at 1–500 characters. A longer field is rejected by strict hosts, and an environment blurb that long is doing the body's job.",
    advice: "Shorten compatibility to the environment requirements only (example: \"Requires Python 3.11+ and docker\"), and move the rest into the body.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

struct Frontmatter;
struct KeyFormat;
struct UnknownField;
struct CompatibilityMaxLength;

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
            if key != lower && SPEC_FIELDS.contains(&lower.as_str()) {
                context.report(
                    format!("\"{key}\" shadows the defined field \"{lower}\""),
                    Location::at(line, 1),
                );
            }
        }
    }
}

impl Rule for UnknownField {
    fn meta(&self) -> &'static RuleMeta {
        &UNKNOWN_FIELD
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let keys: Vec<String> = context.skill.metadata.keys().cloned().collect();

        for key in keys {
            // Pairs flattened out of the metadata map report against "metadata" itself.
            if key.starts_with("metadata.") {
                continue;
            }

            let lower = key.to_ascii_lowercase();
            if is_known_field(&lower) {
                continue;
            }

            let line = context.skill.frontmatter_line(&key);
            context.report(
                format!("\"{key}\" is not a recognized frontmatter field"),
                Location::at(line, 1),
            );
        }
    }
}

static TYPE_ERROR: RuleMeta = RuleMeta {
    name: "frontmatter/type-error",
    summary: "name and description must be YAML strings.",
    rationale: "An agent reads the name and description as text. A value YAML parses as a number, boolean, null, sequence, or mapping is not that text — or is nothing at all — and every check on it would run against the wrong thing.",
    advice: "Write the value as plain text, quoting it if it starts with a character YAML treats specially.",
    default_severity: Severity::Error,
    fixable: false,
    needs_model: false,
    reference_title: sources::SPECIFICATION.0,
    reference_url: sources::SPECIFICATION.1,
};

struct TypeError;

impl Rule for TypeError {
    fn meta(&self) -> &'static RuleMeta {
        &TYPE_ERROR
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        for field in ["name", "description"] {
            let Some(actual) = context.skill.scalar_types.get(field) else {
                continue;
            };
            if actual == "string" {
                continue;
            }

            let line = context.skill.frontmatter_line(field);
            context.report(
                format!("\"{field}\" is a YAML {actual}, not a string"),
                Location::at(line, 1),
            );
        }
    }
}

impl Rule for CompatibilityMaxLength {
    fn meta(&self) -> &'static RuleMeta {
        &COMPATIBILITY_MAX_LENGTH_META
    }

    fn check(&self, context: &mut RuleContext<'_>) {
        let Some(value) = context.skill.metadata.get("compatibility") else {
            return;
        };

        let length = value.chars().count();

        if length > COMPATIBILITY_MAX_LENGTH {
            context.report(
                format!(
                    "The compatibility field is {length} characters; the limit is {COMPATIBILITY_MAX_LENGTH}"
                ),
                Location::at(context.skill.frontmatter_line("compatibility"), 1),
            );
        }
    }
}

static FRONTMATTER_RULE: Frontmatter = Frontmatter;
static KEY_RULE: KeyFormat = KeyFormat;
static UNKNOWN_FIELD_RULE: UnknownField = UnknownField;
static COMPATIBILITY_MAX_LENGTH_RULE: CompatibilityMaxLength = CompatibilityMaxLength;
static TYPE_ERROR_RULE: TypeError = TypeError;

pub fn rules() -> Vec<&'static dyn Rule> {
    vec![
        &FRONTMATTER_RULE,
        &KEY_RULE,
        &UNKNOWN_FIELD_RULE,
        &COMPATIBILITY_MAX_LENGTH_RULE,
        &TYPE_ERROR_RULE,
    ]
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

    /// Regression for #81: the frontmatter extensions Claude Code documents are first-class
    /// top-level fields, not unknown keys to remove.
    #[test]
    fn claude_code_extension_fields_are_recognized() {
        for field in [
            "disable-model-invocation",
            "user-invocable",
            "disallowed-tools",
            "model",
            "effort",
            "context",
            "agent",
            "background",
            "hooks",
            "paths",
            "shell",
            "argument-hint",
            "arguments",
            "when_to_use",
        ] {
            let messages = unknown_field_messages(&format!(
                "---\nname: a\ndescription: b\n{field}: true\n---\n\nBody.\n"
            ));

            assert!(messages.is_empty(), "\"{field}\" should be recognized");
        }
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
        let messages = unknown_field_messages(
            "---\nname: a\ndescription: b\nversion: \"1.0\"\n---\n\nBody.\n",
        );

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
        let messages = unknown_field_messages(
            "---\nname: a\ndescription: b\nteam: photography\n---\n\nBody.\n",
        );

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("team"));
    }

    /// Regression for #92: the spec caps compatibility at 500 characters, and nothing checked it.
    fn compatibility_max_length_messages(source: &str) -> Vec<crate::diagnostics::Message> {
        let parsed = skill::parse(source);
        crate::engine::lint_skill(&parsed, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "frontmatter/compatibility-max-length")
            .collect()
    }

    #[test]
    fn an_oversized_compatibility_field_is_reported() {
        let filler = "requires ".repeat(70);
        let source = format!(
            "---\nname: photo-culling\ndescription: Culls a photo shoot. Use when triaging RAW files.\ncompatibility: \"{filler}\"\n---\n\n## Culling\n\n1. Import the RAW files.\n"
        );
        let messages = compatibility_max_length_messages(&source);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
        assert!(
            messages[0].message.contains("500"),
            "{}",
            messages[0].message
        );
        assert_eq!(messages[0].location.line, 4);
    }

    #[test]
    fn a_compatibility_field_within_the_limit_passes() {
        let messages = compatibility_max_length_messages(
            "---\nname: a\ndescription: b\ncompatibility: Requires Python 3.11+ and docker\n---\n\nBody.\n",
        );

        assert!(messages.is_empty());
    }

    #[test]
    fn a_missing_compatibility_field_is_not_reported() {
        let messages =
            compatibility_max_length_messages("---\nname: a\ndescription: b\n---\n\nBody.\n");

        assert!(messages.is_empty());
    }

    fn type_error_messages(source: &str) -> Vec<crate::diagnostics::Message> {
        let parsed = skill::parse(source);
        crate::engine::lint_skill(&parsed, &crate::config::Config::default())
            .into_iter()
            .filter(|message| message.rule == "frontmatter/type-error")
            .collect()
    }

    /// Regression for #94: a frontmatter value that YAML parses as a non-string scalar must be
    /// reported as a type error, not quietly stringified.
    #[test]
    fn a_boolean_description_is_a_type_error() {
        let messages = type_error_messages("---\nname: a\ndescription: true\n---\n\nBody.\n");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].severity, Severity::Error);
        assert!(messages[0].message.contains("boolean"));
    }

    #[test]
    fn a_numeric_name_is_a_type_error() {
        let messages = type_error_messages("---\nname: 12345\ndescription: b\n---\n\nBody.\n");

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("number"));
    }

    #[test]
    fn a_null_description_is_a_type_error() {
        let messages = type_error_messages("---\nname: a\ndescription:\n---\n\nBody.\n");

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("null"));
    }

    #[test]
    fn a_sequence_description_is_a_type_error() {
        let messages = type_error_messages(
            "---\nname: a\ndescription:\n  - first\n  - second\n---\n\nBody.\n",
        );

        assert_eq!(messages.len(), 1);
        assert!(messages[0].message.contains("sequence"));
    }

    #[test]
    fn string_and_absent_fields_are_not_type_errors() {
        assert!(type_error_messages("---\nname: a\ndescription: b\n---\n\nBody.\n").is_empty());
        assert!(type_error_messages("---\ndescription: b\n---\n\nBody.\n").is_empty());
    }
}
