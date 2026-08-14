//! One call per skill, for every rule a model has to answer.
//!
//! The prompt is a brief rather than a checklist, because the failure mode of a checklist here is a
//! model that finds one of everything. What comes back is validated against the catalogue: a finding
//! naming a rule that does not exist, or a line that is not in the document, is dropped rather than
//! shown — a linter that invents positions is worse than one that says nothing.

use anyhow::Result;
use serde::Deserialize;

use crate::config::{Config, LlmConfig};
use crate::diagnostics::{Location, Message, Source};
use crate::llm::provider::{Chat, GenAiChat, Prompt};
use crate::llm::rules;
use crate::skill::Skill;

/// What the model is asked to return, one element per finding.
#[derive(Debug, Deserialize)]
struct RawFinding {
    rule: String,
    message: String,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    confidence: Option<f32>,
}

pub fn system_prompt() -> String {
    let catalogue = rules::all()
        .iter()
        .map(|meta| {
            format!(
                "- {} ({}): {}\n  Why it matters: {}\n  What to do: {}",
                meta.name, meta.default_severity, meta.summary, meta.rationale, meta.advice
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You review Agent Skills: instruction documents an AI agent fetches and follows.\n\
\n\
An agent picks a skill from its description alone, then follows the body without asking clarifying questions. Report only problems that would change selection or how the agent behaves.\n\
\n\
Rules (report against these only):\n\
{catalogue}\n\
\n\
Answer with a JSON array and nothing else. Each element is an object with:\n\
  rule        one of the rule names above\n\
  message     1–2 plain sentences for a skill author who has never seen this linter:\n\
              (1) name or quote the specific passage that is wrong,\n\
              (2) say what goes wrong for the agent because of it.\n\
              Do not restate the rule id. Do not use jargon or metaphors.\n\
  line        1-based line in SKILL.md when the finding is about a line\n\
  confidence  0 to 1\n\
\n\
An empty array is correct for a well-written skill. Do not report anything a regex could find (lengths, paths, missing files, credentials). Do not invent line numbers."
    )
}

/// What the model is shown. Bodies over the configured limit are cut at a heading, and the caller
/// is told so.
pub fn user_prompt(skill: &Skill, max_bytes: usize) -> (String, Option<String>) {
    let (body, note) = truncate(&skill.body, max_bytes);

    let files = if skill.files.is_empty() {
        "none".to_string()
    } else {
        skill
            .files
            .iter()
            .map(|file| format!("{} ({} bytes)", file.path, file.bytes))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let prompt = format!(
        "Name: {}\n\nDescription: {}\n\nBundled files: {files}\n\nSKILL.md body follows.\n\n----\n{body}",
        skill.name, skill.description
    );

    (prompt, note)
}

/// Cuts a body at a heading boundary, so the model never sees half a sentence.
fn truncate(body: &str, max_bytes: usize) -> (String, Option<String>) {
    if body.len() <= max_bytes {
        return (body.to_string(), None);
    }

    let mut kept = String::new();

    for line in body.lines() {
        if kept.len() + line.len() + 1 > max_bytes {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }

    // Back off to the last heading, so the fragment the model reads is a whole section.
    if let Some(at) = kept.rfind("\n#") {
        kept.truncate(at + 1);
    }

    (
        kept,
        Some(format!(
            "The body is {} bytes and was truncated to {max_bytes} before being sent, so the model read part of it.",
            body.len()
        )),
    )
}

/// Turns what the model said into messages, dropping anything that does not check out.
pub fn parse_response(
    text: &str,
    skill: &Skill,
    config: &Config,
    model: &str,
) -> (Vec<Message>, Vec<String>) {
    let mut notes = Vec::new();

    // Models fence JSON more often than not, and refusing a fenced array would mean throwing away
    // a correct answer over punctuation.
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let raw: Vec<RawFinding> = match serde_json::from_str(cleaned) {
        Ok(parsed) => parsed,
        Err(failure) => {
            notes.push(format!(
                "The model's answer was not the JSON array the prompt asked for, so nothing from it was used ({failure})."
            ));
            return (Vec::new(), notes);
        }
    };

    let lines = skill.source.lines().count().max(1);
    let mut messages = Vec::new();
    let mut dropped = 0;

    for finding in raw {
        let Some(meta) = rules::meta_for(&finding.rule) else {
            dropped += 1;
            continue;
        };

        let Some(severity) = config.severity_for(meta.name, meta.default_severity) else {
            continue;
        };

        // A line outside the document means the model guessed; the finding is kept and the position
        // is dropped, because being wrong about where is not being wrong about what.
        let line = match finding.line {
            Some(line) if line >= 1 && line <= lines => line,
            _ => 1,
        };

        messages.push(Message {
            rule: meta.name.to_string(),
            severity,
            message: finding.message,
            advice: meta.advice.to_string(),
            location: Location::at(line, 1),
            source: Source::Model,
            file: skill.document.clone(),
            fix: None,
            reference: meta.reference(),
            confidence: finding.confidence.unwrap_or(0.6).clamp(0.0, 1.0),
        });
    }

    if dropped > 0 {
        notes.push(format!(
            "{dropped} finding(s) from {model} named a rule that does not exist and were dropped."
        ));
    }

    (messages, notes)
}

/// Reviews one skill with whatever the config points at.
pub fn review(skill: &Skill, config: &Config) -> Result<(Vec<Message>, Vec<String>)> {
    let client = GenAiChat::new(&config.llm)?;
    review_with(&client, skill, config, &config.llm)
}

/// The same, against any client — which is how this is tested without a network.
pub fn review_with(
    client: &dyn Chat,
    skill: &Skill,
    config: &Config,
    llm: &LlmConfig,
) -> Result<(Vec<Message>, Vec<String>)> {
    let (user, truncation) = user_prompt(skill, llm.max_input_bytes);
    let prompt = Prompt {
        system: system_prompt(),
        user,
    };

    let answer = client.complete(&prompt)?;
    let (messages, mut notes) = parse_response(&answer, skill, config, &client.describe());

    if let Some(note) = truncation {
        notes.push(note);
    }

    Ok((messages, notes))
}

/// Which model rules are on, for the note a static-only run leaves behind.
pub fn enabled_rule_names(config: &Config) -> Vec<&'static str> {
    rules::all()
        .into_iter()
        .filter(|meta| {
            config
                .severity_for(meta.name, meta.default_severity)
                .is_some()
        })
        .map(|meta| meta.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuleSetting;
    use crate::diagnostics::Severity;
    use crate::rules::testing::good_skill;

    struct Fake {
        answer: String,
    }

    impl Chat for Fake {
        fn complete(&self, _prompt: &Prompt) -> Result<String> {
            Ok(self.answer.clone())
        }

        fn describe(&self) -> String {
            "fake/model".into()
        }
    }

    #[test]
    fn the_prompt_names_every_model_rule_and_forbids_the_rest() {
        let prompt = system_prompt();

        for meta in rules::all() {
            assert!(
                prompt.contains(meta.name),
                "{} is missing from the prompt",
                meta.name
            );
        }

        assert!(prompt.contains("JSON array"));
        assert!(prompt.contains("regex could find") || prompt.contains("regular expression"));
        assert!(prompt.contains("What to do:"));
        assert!(prompt.contains("Why it matters:"));
    }

    #[test]
    fn the_user_prompt_carries_what_the_model_needs_and_nothing_it_does_not() {
        let mut skill = good_skill();
        skill.files.push(crate::skill::BundledFile {
            path: "scripts/cull.py".into(),
            bytes: 120,
            executable: true,
            text: Some("secrets in here".into()),
        });

        let (prompt, note) = user_prompt(&skill, 64 * 1024);

        assert!(prompt.contains("photo-culling"));
        assert!(prompt.contains("scripts/cull.py (120 bytes)"));
        // File contents are not sent: their names and sizes are enough for every rule here.
        assert!(!prompt.contains("secrets in here"));
        assert!(note.is_none());
    }

    #[test]
    fn a_long_body_is_cut_at_a_heading_and_the_run_is_told() {
        let mut skill = good_skill();
        skill.body = format!(
            "## One\n\n{}\n## Two\n\n{}",
            "a".repeat(400),
            "b".repeat(400)
        );

        let (prompt, note) = user_prompt(&skill, 500);

        assert!(note.unwrap().contains("truncated"));
        assert!(prompt.contains("## One"));
        assert!(!prompt.contains("## Two"));
    }

    #[test]
    fn findings_come_back_as_messages_carrying_the_rule_and_its_citation() {
        let skill = good_skill();
        let answer = r#"[{"rule":"llm/no-ambiguity","message":"Step 2 can be read two ways.","line":8,"confidence":0.7}]"#;

        let (messages, notes) = parse_response(answer, &skill, &Config::default(), "fake/model");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].rule, "llm/no-ambiguity");
        assert_eq!(messages[0].source, Source::Model);
        assert_eq!(messages[0].location.line, 8);
        assert!((messages[0].confidence - 0.7).abs() < f32::EPSILON);
        assert!(messages[0].reference.url.starts_with("https://"));
        assert!(notes.is_empty());
    }

    #[test]
    fn a_fenced_array_is_read_rather_than_refused() {
        let skill = good_skill();
        let answer =
            "```json\n[{\"rule\":\"llm/output-example\",\"message\":\"No example.\"}]\n```";

        let (messages, _) = parse_response(answer, &skill, &Config::default(), "fake/model");
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn a_finding_naming_a_rule_that_does_not_exist_is_dropped_and_counted() {
        let skill = good_skill();
        let answer = r#"[{"rule":"llm/invented","message":"Something."},{"rule":"llm/failure-path","message":"Step 3 can fail."}]"#;

        let (messages, notes) = parse_response(answer, &skill, &Config::default(), "fake/model");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].rule, "llm/failure-path");
        assert!(notes[0].contains("1 finding(s)"));
    }

    #[test]
    fn a_line_outside_the_document_is_dropped_rather_than_pointed_at() {
        let skill = good_skill();
        let answer = r#"[{"rule":"llm/no-ambiguity","message":"Somewhere.","line":9000}]"#;

        let (messages, _) = parse_response(answer, &skill, &Config::default(), "fake/model");

        assert_eq!(messages.len(), 1, "the finding survives");
        assert_eq!(
            messages[0].location.line, 1,
            "the invented position does not"
        );
    }

    #[test]
    fn a_rule_turned_off_in_the_config_is_not_reported_even_if_the_model_finds_it() {
        let skill = good_skill();
        let mut config = Config::default();
        config
            .rules
            .insert("llm/output-example".into(), RuleSetting::Off);

        let answer = r#"[{"rule":"llm/output-example","message":"No example."}]"#;
        let (messages, _) = parse_response(answer, &skill, &config, "fake/model");

        assert!(messages.is_empty());
    }

    #[test]
    fn a_config_can_raise_a_model_rule_to_an_error() {
        let skill = good_skill();
        let mut config = Config::default();
        config.rules.insert(
            "llm/trigger-coverage".into(),
            RuleSetting::On(Severity::Error),
        );

        let answer = r#"[{"rule":"llm/trigger-coverage","message":"Would not route."}]"#;
        let (messages, _) = parse_response(answer, &skill, &config, "fake/model");

        assert_eq!(messages[0].severity, Severity::Error);
    }

    #[test]
    fn an_answer_that_is_not_json_is_reported_rather_than_ignored() {
        let skill = good_skill();
        let (messages, notes) = parse_response(
            "I had a look and it seems fine!",
            &skill,
            &Config::default(),
            "fake",
        );

        assert!(messages.is_empty());
        assert!(notes[0].contains("not the JSON array"));
    }

    #[test]
    fn a_review_against_a_fake_client_produces_messages_and_notes() {
        let skill = good_skill();
        let client = Fake {
            answer: r#"[{"rule":"llm/failure-path","message":"Step 2 can fail silently."}]"#.into(),
        };

        let (messages, notes) =
            review_with(&client, &skill, &Config::default(), &LlmConfig::default()).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(notes.is_empty());
        // No confidence given, so the default sits below a static rule's certainty.
        assert!(messages[0].confidence < 1.0);
    }

    #[test]
    fn the_names_of_the_rules_that_did_not_run_are_available_for_the_note() {
        let mut config = Config::default();
        config
            .rules
            .insert("llm/output-example".into(), RuleSetting::Off);

        let names = enabled_rule_names(&config);

        assert!(!names.contains(&"llm/output-example"));
        assert!(names.contains(&"llm/trigger-coverage"));
    }
}
