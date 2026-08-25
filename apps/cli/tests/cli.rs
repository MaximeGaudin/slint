//! The tool as someone actually runs it.
//!
//! These drive the built binary over real directories, because the things most likely to break are
//! the seams the unit tests do not cross: exit codes, what lands on stdout rather than stderr, and
//! whether `--fix` leaves a file that lints clean.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn slint(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slint"))
        .args(arguments)
        .output()
        .expect("running slint")
}

fn write(root: &Path, name: &str, document: &str) {
    let directory = root.join(name);
    fs::create_dir_all(&directory).expect("creating the skill directory");
    fs::write(directory.join("SKILL.md"), document).expect("writing SKILL.md");
}

const GOOD: &str = "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Import the RAW files.\n2. Flag the keepers with P.\n";

const BROKEN: &str = "---\nname: helper\ndescription: Photo helper.\n---\n\n## Helper\n\n1. Read scripts\\notes.md.\n2. Run scripts/cull.py.\n";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn a_clean_base_exits_zero_and_says_so() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Nothing to report"));
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/27 —
/// a negative limit used to be read as the default limit, and the run went on as if the
/// config had said nothing at all.
#[test]
fn an_out_of_range_rule_option_fails_the_run_and_names_the_rule() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);
    fs::write(
        temporary.path().join("slint.toml"),
        "[rules]\n\"body/max-lines\" = [\"warn\", { max = -5 }]\n",
    )
    .unwrap();

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(output.status.code(), Some(3), "a bad option is a config failure: {stderr}");
    assert!(stderr.contains("body/max-lines"), "{stderr}");
}

#[test]
fn an_error_exits_one() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(output.status.code(), Some(1), "a dangling path is an error");
    assert!(stdout(&output).contains("bundle/no-dangling-path"));
}

#[test]
fn warnings_alone_exit_two() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a generic name is only a warning"
    );
}

#[test]
fn a_warning_budget_turns_warnings_into_a_failed_run() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--max-warnings",
        "0",
    ]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_rule_can_be_turned_off_from_the_command_line() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--rule",
        "bundle/no-dangling-path=off",
    ]);

    assert_ne!(
        output.status.code(),
        Some(1),
        "the only error was turned off"
    );
    assert!(!stdout(&output).contains("bundle/no-dangling-path"));
}

#[test]
fn fixing_rewrites_the_file_and_the_next_run_is_cleaner() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "photo-culling",
        "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Read references\\formats.md.\n",
    );

    fs::create_dir_all(temporary.path().join("photo-culling/references")).unwrap();
    fs::write(
        temporary.path().join("photo-culling/references/formats.md"),
        "# Formats\n",
    )
    .unwrap();

    let before = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    assert!(stdout(&before).contains("body/posix-paths"));

    let fixed = slint(&[temporary.path().to_str().unwrap(), "--no-llm", "--fix"]);
    assert!(stdout(&fixed).contains("fix(es) applied"));

    let document = fs::read_to_string(temporary.path().join("photo-culling/SKILL.md")).unwrap();
    assert!(document.contains("references/formats.md"));
    assert!(!document.contains('\\'));

    let after = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    assert!(!stdout(&after).contains("body/posix-paths"));
}

#[test]
fn the_json_format_is_an_envelope_a_caller_can_branch_on() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "json",
    ]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");

    assert_eq!(parsed["ok"], false);
    assert!(parsed["summary"]["errors"].as_u64().unwrap() >= 1);
    assert!(
        parsed["data"]["skills"][0]["messages"][0]["reference"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
}

#[test]
fn the_json_format_puts_nothing_but_json_on_stdout() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "json",
    ]);

    // Anything else — notes, progress, complaints — belongs on stderr, or a pipe into jq breaks.
    serde_json::from_str::<serde_json::Value>(&stdout(&output)).expect("stdout is only JSON");
}

#[test]
fn a_config_file_is_found_by_walking_up_from_the_path_being_linted() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("slint.toml"),
        "[rules]\n\"bundle/no-dangling-path\" = \"off\"\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();

    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().join("helper").to_str().unwrap(),
        "--no-llm",
    ]);

    assert!(!stdout(&output).contains("bundle/no-dangling-path"));
    assert!(!stdout(&output).contains("name/not-generic"));
}

#[test]
fn a_rule_pack_named_by_the_config_runs_beside_the_built_in_rules() {
    let temporary = tempfile::tempdir().unwrap();

    fs::write(
        temporary.path().join("slint.toml"),
        "[[plugins]]\npath = \"./house.toml\"\n",
    )
    .unwrap();

    fs::write(
        temporary.path().join("house.toml"),
        "[[rules]]\nname = \"house/no-todo\"\nseverity = \"error\"\nsummary = \"No TODO markers in instructions.\"\nrationale = \"An agent follows what is written, and a TODO reads as an instruction to it.\"\nadvice = \"Finish the step or take the line out.\"\npattern = \"TODO\"\ntarget = \"body\"\nreference = { title = \"House style\", url = \"https://example.com/style\" }\n",
    )
    .unwrap();

    write(
        temporary.path(),
        "photo-culling",
        "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. TODO write this properly.\n",
    );

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(output.status.code(), Some(1), "the pack's rule is an error");
    assert!(stdout(&output).contains("house/no-todo"));
    assert!(stdout(&output).contains("https://example.com/style"));
}

#[test]
fn plugins_can_be_skipped_without_editing_the_config() {
    let temporary = tempfile::tempdir().unwrap();

    fs::write(
        temporary.path().join("slint.toml"),
        "[[plugins]]\npath = \"./house.toml\"\n",
    )
    .unwrap();
    fs::write(
        temporary.path().join("house.toml"),
        "[[rules]]\nname = \"house/no-todo\"\nseverity = \"error\"\nsummary = \"No TODO markers.\"\nrationale = \"An agent follows what is written, and a TODO reads as an instruction.\"\nadvice = \"Finish the step.\"\npattern = \"TODO\"\nreference = { title = \"House style\", url = \"https://example.com/style\" }\n",
    )
    .unwrap();

    write(
        temporary.path(),
        "photo-culling",
        "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. TODO write this properly.\n",
    );

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--no-plugins",
    ]);

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn init_writes_a_config_and_refuses_to_overwrite_one() {
    let temporary = tempfile::tempdir().unwrap();

    let first = Command::new(env!("CARGO_BIN_EXE_slint"))
        .arg("init")
        .current_dir(temporary.path())
        .output()
        .unwrap();

    assert_eq!(first.status.code(), Some(0));
    assert!(temporary.path().join("slint.toml").is_file());

    let second = Command::new(env!("CARGO_BIN_EXE_slint"))
        .arg("init")
        .current_dir(temporary.path())
        .output()
        .unwrap();

    assert_eq!(
        second.status.code(),
        Some(3),
        "it does not clobber what is there"
    );
}

#[test]
fn the_rule_catalogue_can_be_printed_as_json_for_the_documentation() {
    let output = slint(&["rules", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");

    let rules = parsed.as_array().expect("an array of rules");
    assert!(rules.len() > 20);

    for rule in rules {
        assert!(rule["name"].as_str().unwrap().contains('/'));
        assert!(
            rule["reference_url"]
                .as_str()
                .unwrap()
                .starts_with("https://")
        );
        assert!(rule["advice"].as_str().unwrap().len() > 20);
    }
}

#[test]
fn the_catalogue_reads_as_prose_when_it_is_not_asked_for_as_json() {
    let output = slint(&["rules"]);
    let text = stdout(&output);

    assert!(text.contains("description/says-when"));
    assert!(text.contains("→"), "each rule prints its advice");
    assert!(text.contains("https://"));
}

#[test]
fn a_suppression_comment_in_the_document_is_honoured() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n<!-- slint-disable name/not-generic -->\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn the_model_pass_is_off_until_it_is_asked_for() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    // No flags at all: the default must never reach a paid provider, and must say how to.
    let output = slint(&[temporary.path().to_str().unwrap()]);
    let text = stdout(&output);

    assert_eq!(output.status.code(), Some(0));
    assert!(text.contains("Skipped"), "{text}");
    assert!(text.contains("model rules"), "{text}");
    assert!(text.contains("--llm"), "{text}");
}

#[test]
fn asking_for_the_model_pass_without_a_provider_says_what_is_missing() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--llm"]);
    let text = stdout(&output);

    assert!(text.contains("no provider is configured"), "{text}");
    assert!(text.contains("api_key_env"), "{text}");
}

#[test]
fn the_readable_output_lines_its_columns_up_and_cites_each_rule_once() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let text = stdout(&slint(&[temporary.path().to_str().unwrap()]));

    // The rule name is a column of its own rather than trailing the message.
    assert!(text.contains("error    bundle/no-dangling-path"), "{text}");
}

#[test]
fn a_rule_that_fires_twice_is_cited_once() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "photo-culling",
        "---\nname: photo-culling\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Culling\n\n1. Import from /Users/mgaudin/shoots.\n2. Export to ~/selects.\n",
    );

    let text = stdout(&slint(&[temporary.path().to_str().unwrap()]));

    assert_eq!(
        text.matches("body/relative-paths").count(),
        2,
        "both are reported:\n{text}"
    );
    assert_eq!(
        text.matches("agentskills.io/specification").count(),
        1,
        "and the source is printed once:\n{text}"
    );
}

#[test]
fn an_undeclared_host_specific_tool_in_the_body_is_reported() {
    // Reproduces https://github.com/MaximeGaudin/slint/issues/3: a skill that hard-requires
    // Cursor's AskQuestion tool without listing it in allowed-tools and without a portable fallback.
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "grill-brief",
        "---\nname: grill-brief\ndescription: Interrogates briefs with batched questions until they are spec-ready. Use when grilling briefs.\n---\n\n# Grill Brief\n\n2. Ask a batch of 3 to 6 multiple-choice questions with the AskQuestion tool. Rules for every batch:\n   - Use `allow_multiple: true` for questions that are genuinely a set.\n",
    );

    let text = stdout(&slint(&[temporary.path().to_str().unwrap(), "--no-llm"]));

    assert!(
        text.contains("body/undeclared-tool"),
        "expected undeclared host tool finding:\n{text}"
    );
    assert!(
        text.contains("AskQuestion"),
        "finding should name the tool:\n{text}"
    );
}
