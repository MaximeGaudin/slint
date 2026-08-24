//! The rule catalogue.
//!
//! Every rule is a value with a name, a severity it ships with, the sentence it prints, the advice
//! that follows it, and the document it is derived from. A rule with no citation does not get into
//! the registry — that is enforced by a test, because "the linter says so" is the failure mode this
//! tool exists to avoid.
//!
//! Rules are cheap by construction: they read text that has already been parsed, and the ones that
//! genuinely cannot be answered that way are marked `needs_model` and live in `llm`.

pub mod body;
pub mod bundle;
pub mod description;
pub mod metadata;
pub mod naming;
pub mod project;

use serde::Serialize;

use crate::config::Config;
use crate::diagnostics::{Fix, Location, Message, Reference, Severity, Source};
use crate::skill::Skill;

/// The documents the catalogue is derived from.
///
/// Named constants rather than a URL pasted into forty rules: a link that moves is one edit, and a
/// typo is visible here rather than buried in the middle of a rule.
pub mod sources {
    pub const BEST_PRACTICES: (&str, &str) = (
        "Skill authoring best practices — Claude Docs",
        "https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices",
    );
    pub const OVERVIEW: (&str, &str) = (
        "Agent Skills overview — Claude Docs",
        "https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview",
    );
    pub const CLAUDE_CODE: (&str, &str) = (
        "Extend Claude with skills — Claude Code docs",
        "https://code.claude.com/docs/en/skills",
    );
    pub const ENGINEERING: (&str, &str) = (
        "Equipping agents for the real world with Agent Skills — Anthropic Engineering",
        "https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills",
    );
    pub const SPECIFICATION: (&str, &str) = (
        "The AgentSkill specification",
        "https://agentskills.io/specification",
    );
    pub const OPTIONAL_DIRECTORIES: (&str, &str) = (
        "The AgentSkill specification — Optional directories",
        "https://agentskills.io/specification#optional-directories",
    );
    pub const PAPER: (&str, &str) = (
        "Authoring Agent Skills: a software-engineering approach",
        "https://arxiv.org/html/2607.25032v1",
    );
}

/// What a rule is, apart from what it does.
#[derive(Debug, Clone, Serialize)]
pub struct RuleMeta {
    /// As it is written in a config file. `area/thing`, so a config reads as a sentence.
    pub name: &'static str,
    /// One line: what it checks.
    pub summary: &'static str,
    /// Why it matters, in the terms of what it costs an agent.
    pub rationale: &'static str,
    /// What to do about it. Static text, so advice costs nothing and is there before any model is.
    pub advice: &'static str,
    pub default_severity: Severity,
    /// Whether it can produce a fix that is computed rather than written.
    pub fixable: bool,
    /// Whether answering it needs a language model.
    pub needs_model: bool,
    pub reference_title: &'static str,
    pub reference_url: &'static str,
}

impl RuleMeta {
    pub fn reference(&self) -> Reference {
        Reference {
            title: self.reference_title.to_string(),
            url: self.reference_url.to_string(),
        }
    }
}

/// What a rule is given, and what it reports into.
pub struct RuleContext<'a> {
    pub skill: &'a Skill,
    pub config: &'a Config,
    severity: Severity,
    meta: &'static RuleMeta,
    messages: Vec<Message>,
}

impl<'a> RuleContext<'a> {
    pub fn new(
        skill: &'a Skill,
        config: &'a Config,
        meta: &'static RuleMeta,
        severity: Severity,
    ) -> Self {
        RuleContext {
            skill,
            config,
            severity,
            meta,
            messages: Vec::new(),
        }
    }

    /// The rule's own options, or the default when the config said nothing.
    pub fn option<T: serde::de::DeserializeOwned + Default>(&self) -> T {
        self.config
            .options_for(self.meta.name)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    /// Reports against `SKILL.md`.
    pub fn report(&mut self, message: impl Into<String>, location: Location) {
        let file = self.skill.document.clone();
        self.push(message, location, file, None, None, None);
    }

    /// Reports against `SKILL.md`, with a fix that is computed rather than written.
    pub fn report_fixable(&mut self, message: impl Into<String>, location: Location, fix: Fix) {
        let file = self.skill.document.clone();
        self.push(message, location, file, Some(fix), None, None);
    }

    /// Reports against a bundled file.
    pub fn report_in_file(&mut self, path: &str, message: impl Into<String>, location: Location) {
        let file = format!("{}/{path}", self.skill.directory.display());
        self.push(message, location, file, None, None, None);
    }

    /// Reports against a bundled file with finding-specific advice and citation.
    pub fn report_in_file_with(
        &mut self,
        path: &str,
        message: impl Into<String>,
        location: Location,
        advice: impl Into<String>,
        reference: Reference,
    ) {
        let file = format!("{}/{path}", self.skill.directory.display());
        self.push(
            message,
            location,
            file,
            None,
            Some(advice.into()),
            Some(reference),
        );
    }

    pub fn report_fixable_in_file(
        &mut self,
        path: &str,
        message: impl Into<String>,
        location: Location,
        fix: Fix,
    ) {
        let file = format!("{}/{path}", self.skill.directory.display());
        self.push(message, location, file, Some(fix), None, None);
    }

    fn push(
        &mut self,
        message: impl Into<String>,
        location: Location,
        file: String,
        fix: Option<Fix>,
        advice: Option<String>,
        reference: Option<Reference>,
    ) {
        self.messages.push(Message {
            rule: self.meta.name.to_string(),
            severity: self.severity,
            message: message.into(),
            advice: advice.unwrap_or_else(|| self.meta.advice.to_string()),
            location,
            source: Source::Static,
            file,
            fix,
            reference: reference.unwrap_or_else(|| self.meta.reference()),
            confidence: 1.0,
        });
    }

    pub fn finish(self) -> Vec<Message> {
        self.messages
    }
}

/// A rule that reads one skill.
pub trait Rule: Sync + Send {
    fn meta(&self) -> &'static RuleMeta;
    fn check(&self, context: &mut RuleContext<'_>);

    /// What is wrong with the options the config gave this rule, when there is something wrong.
    ///
    /// Rules without options keep the default: there is nothing in the config to misspell, and a
    /// rule that cannot be tuned should not be able to be misconfigured either.
    fn options_error(&self, _options: &serde_json::Value) -> Option<String> {
        None
    }
}

/// A rule that can only be answered by looking at every skill at once.
///
/// Two descriptions that compete are not visible from inside either of them, which is why these
/// exist as their own kind rather than as a rule that quietly reads its neighbours.
pub trait ProjectRule: Sync + Send {
    fn meta(&self) -> &'static RuleMeta;
    fn check(&self, skills: &[Skill], config: &Config, severity: Severity) -> Vec<Message>;

    /// The options this rule reads from the config, checked the same way a `Rule`'s are.
    fn options_error(&self, _options: &serde_json::Value) -> Option<String> {
        None
    }
}

/// Every rule that reads one skill.
pub fn registry() -> Vec<&'static dyn Rule> {
    let mut rules: Vec<&'static dyn Rule> = Vec::new();
    rules.extend(naming::rules());
    rules.extend(description::rules());
    rules.extend(body::rules());
    rules.extend(bundle::rules());
    rules.extend(metadata::rules());
    rules
}

/// Every rule that reads the whole set.
pub fn project_registry() -> Vec<&'static dyn ProjectRule> {
    project::rules()
}

/// Every rule in the tool, for `--print-rules` and for the documentation site.
pub fn all_meta() -> Vec<&'static RuleMeta> {
    let mut all: Vec<&'static RuleMeta> = registry().iter().map(|rule| rule.meta()).collect();
    all.extend(project_registry().iter().map(|rule| rule.meta()));
    all.extend(crate::llm::rules::all());
    all.sort_by_key(|meta| meta.name);
    all
}

pub fn meta_for(name: &str) -> Option<&'static RuleMeta> {
    all_meta().into_iter().find(|meta| meta.name == name)
}

/// Helpers every rule's own tests use.
///
/// A rule is a pure function of a parsed skill, so testing one needs no files and no config: build
/// the skill, run the rule, read the messages.
#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::skill;

    /// A skill that passes every rule, as the starting point for a test that breaks one thing.
    pub fn good_skill() -> Skill {
        let mut parsed = skill::parse(
            "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Import the RAW files.\n2. Flag keepers with P and rejects with X.\n3. Export the selects.\n",
        );

        parsed.directory = std::path::PathBuf::from("skills/photo-culling");
        parsed.document = "skills/photo-culling/SKILL.md".into();
        parsed
    }

    pub fn skill_named(name: &str) -> Skill {
        let mut parsed = good_skill();
        parsed.name = name.to_string();
        parsed.directory = std::path::PathBuf::from(format!("skills/{name}"));
        parsed
    }

    /// A skill whose description is the given text.
    ///
    /// Built by rewriting the document rather than by setting the field, so a rule that produces a
    /// fix has real byte offsets to point at.
    pub fn skill_described(description: &str) -> Skill {
        let source = format!(
            "---\nname: photo-culling\ndescription: {description}\n---\n\n## Culling\n\n1. Import the RAW files.\n"
        );

        let mut parsed = skill::parse(&source);
        parsed.directory = std::path::PathBuf::from("skills/photo-culling");
        parsed.document = "skills/photo-culling/SKILL.md".into();
        parsed
    }

    /// A skill whose body is the given text, with the frontmatter left intact.
    pub fn skill_with_body(body: &str) -> Skill {
        let source = format!(
            "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers. Use when triaging RAW files after a session.\n---\n{body}"
        );

        let mut parsed = skill::parse(&source);
        parsed.directory = std::path::PathBuf::from("skills/photo-culling");
        parsed.document = "skills/photo-culling/SKILL.md".into();
        parsed
    }

    pub fn check(rule: &dyn Rule, skill: &Skill) -> Vec<Message> {
        check_with(rule, skill, &Config::default())
    }

    pub fn check_with(rule: &dyn Rule, skill: &Skill, config: &Config) -> Vec<Message> {
        let meta = rule.meta();
        let mut context = RuleContext::new(skill, config, meta, meta.default_severity);
        rule.check(&mut context);
        context.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_cites_a_document_that_can_be_opened() {
        for meta in all_meta() {
            assert!(
                meta.reference_url.starts_with("https://"),
                "{} has no citation",
                meta.name
            );
            assert!(
                meta.reference_title.len() > 8,
                "{} has a thin citation",
                meta.name
            );
        }
    }

    #[test]
    fn every_rule_says_what_to_do_about_it() {
        for meta in all_meta() {
            assert!(meta.advice.len() > 20, "{} has no advice", meta.name);
            assert!(meta.rationale.len() > 20, "{} has no rationale", meta.name);
            assert!(meta.summary.len() > 10, "{} has no summary", meta.name);
        }
    }

    #[test]
    fn rule_names_are_unique_and_namespaced() {
        let mut seen = std::collections::BTreeSet::new();

        for meta in all_meta() {
            assert!(meta.name.contains('/'), "{} is not namespaced", meta.name);
            assert!(seen.insert(meta.name), "{} is registered twice", meta.name);
        }
    }

    #[test]
    fn most_of_the_catalogue_needs_no_model() {
        let all = all_meta();
        let static_rules = all.iter().filter(|meta| !meta.needs_model).count();

        // The brief's whole point: what can be answered from the text is answered from the text.
        assert!(
            static_rules * 2 > all.len(),
            "only {static_rules} of {} rules are static",
            all.len()
        );
    }

    #[test]
    fn no_rule_that_needs_a_model_claims_to_fix_anything() {
        for meta in all_meta() {
            assert!(
                !(meta.needs_model && meta.fixable),
                "{} would have a model write a fix",
                meta.name
            );
        }
    }
}
