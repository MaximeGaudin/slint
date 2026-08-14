//! The rules a regular expression cannot answer.
//!
//! They are declared here as metadata only. Nothing in this file reads a skill: a model does that,
//! once per skill, for all of these at the same time — partly for cost, but mostly because the
//! interesting ones need to see the whole document rather than a rule's worth of it.
//!
//! Copy guide: `summary` = what we check (plain English). `rationale` = what goes wrong for the
//! agent if you ignore it. `advice` = the concrete edit to make. No metaphors, no riddles.

use crate::diagnostics::Severity;
use crate::rules::{RuleMeta, sources};

pub static KNOWN_CONTEXT: RuleMeta = RuleMeta {
    name: "llm/no-known-context",
    summary: "Do not explain things a capable model already knows.",
    rationale: "Skill text costs tokens. Restating general knowledge (how moods work, what a book is, that users have preferences) crowds out the instructions that are unique to your skill — so the agent has less room for what it actually needs to follow.",
    advice: "Delete the general-knowledge paragraph. Keep only what is specific to this skill: your tools, your steps, your constraints, your domain data.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: true,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

pub static SPECIFICITY: RuleMeta = RuleMeta {
    name: "llm/specificity-matches-risk",
    summary: "Risky steps get precise instructions; judgement calls stay flexible.",
    rationale: "A destructive or irreversible step written vaguely invites the agent to improvise — and improvisation there can delete data or take the wrong irreversible action. Conversely, scripting a judgement call too tightly stops the agent using context it can see.",
    advice: "For steps that cannot be undone (deletes, payments, sends): write the exact command or checklist. For judgement calls (tone, ranking, wording): give the goal and constraints, not a word-for-word script.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: true,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

pub static AMBIGUITY: RuleMeta = RuleMeta {
    name: "llm/no-ambiguity",
    summary: "Every instruction has one clear meaning; none contradict another.",
    rationale: "If a sentence can be read two ways, the agent will sometimes pick the wrong one — and the run log will not tell you which reading it used. Contradictions do the same: the agent silently chooses a side.",
    advice: "Rewrite the ambiguous sentence so only one reading remains. If two instructions conflict, delete or merge until one path is left.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: true,
    reference_title: sources::PAPER.0,
    reference_url: sources::PAPER.1,
};

pub static TERMINOLOGY: RuleMeta = RuleMeta {
    name: "llm/consistent-terminology",
    summary: "Use one name for each concept everywhere in the skill.",
    rationale: "Calling the same thing a \"field\", then a \"box\", then a \"control\" forces the agent to guess they are one thing before it can follow any step. Inconsistent names cause skipped steps and wrong tool arguments.",
    advice: "Pick one term per concept and replace the synonyms — in SKILL.md and in every bundled file that talks about the same thing.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: true,
    reference_title: sources::PAPER.0,
    reference_url: sources::PAPER.1,
};

pub static OUTPUT_EXAMPLE: RuleMeta = RuleMeta {
    name: "llm/output-example",
    summary: "When the output format matters, show a concrete example.",
    rationale: "Agents copy structure more reliably from an example than from a prose description of a format. Without one, JSON fields, headings, or report shapes drift from what the caller expects.",
    advice: "Add a short worked example of the expected output (fenced code or a sample block) in the format the next step or human will consume.",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: true,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

pub static FAILURE_PATH: RuleMeta = RuleMeta {
    name: "llm/failure-path",
    summary: "Steps that can fail say what to do when they fail.",
    rationale: "If a step can error and you do not say what happens next, the agent invents a recovery — retry forever, skip silently, or stop — differently each run.",
    advice: "After each fallible step, add one line: on failure, retry N times / use this fallback / stop and report the error to the user.",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: true,
    reference_title: sources::PAPER.0,
    reference_url: sources::PAPER.1,
};

pub static DEFAULT_CHOICE: RuleMeta = RuleMeta {
    name: "llm/default-choice",
    summary: "When several approaches are listed, mark which one to use by default.",
    rationale: "A menu of equal options spends context and leaves the choice to the agent at the worst moment — mid-task, without your preferences. Runs then diverge for no good reason.",
    advice: "State a default (\"By default, do X\"). Keep alternatives as optional escapes (\"If X is impossible, then Y\").",
    default_severity: Severity::Info,
    fixable: false,
    needs_model: true,
    reference_title: sources::BEST_PRACTICES.0,
    reference_url: sources::BEST_PRACTICES.1,
};

pub static TRIGGER_COVERAGE: RuleMeta = RuleMeta {
    name: "llm/trigger-coverage",
    summary: "The description matches how users would actually ask for this skill.",
    rationale: "The agent picks a skill from the description alone. If users say \"summarize this PDF\" but the description only says \"document assistant\", the skill never gets selected — and the user will not know to ask for it by name.",
    advice: "Edit the description to include the words and situations a real request would use (tools, file types, tasks). Prefer the phrasing users type, not internal category names.",
    default_severity: Severity::Warning,
    fixable: false,
    needs_model: true,
    reference_title: sources::ENGINEERING.0,
    reference_url: sources::ENGINEERING.1,
};

pub fn all() -> Vec<&'static RuleMeta> {
    vec![
        &KNOWN_CONTEXT,
        &SPECIFICITY,
        &AMBIGUITY,
        &TERMINOLOGY,
        &OUTPUT_EXAMPLE,
        &FAILURE_PATH,
        &DEFAULT_CHOICE,
        &TRIGGER_COVERAGE,
    ]
}

pub fn meta_for(name: &str) -> Option<&'static RuleMeta> {
    all().into_iter().find(|meta| meta.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_rule_is_marked_as_needing_one() {
        for meta in all() {
            assert!(
                meta.needs_model,
                "{} is in the model set and does not say so",
                meta.name
            );
            assert!(
                !meta.fixable,
                "{} would have a model write a fix",
                meta.name
            );
        }
    }

    #[test]
    fn a_model_rule_can_be_looked_up_by_the_name_a_config_writes() {
        assert_eq!(
            meta_for("llm/trigger-coverage").map(|meta| meta.name),
            Some("llm/trigger-coverage")
        );
        assert!(meta_for("llm/invented").is_none());
    }

    #[test]
    fn advice_tells_the_author_what_to_edit() {
        for meta in all() {
            assert!(
                meta.advice.len() > 40,
                "{} advice is too short to be actionable",
                meta.name
            );
            assert!(
                !meta.summary.contains("Cut the paragraph"),
                "{} still uses the old cryptic copy",
                meta.name
            );
        }
    }
}
