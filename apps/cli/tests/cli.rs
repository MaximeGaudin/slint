//! The tool as someone actually runs it.
//!
//! These drive the built binary over real directories, because the things most likely to break are
//! the seams the unit tests do not cross: exit codes, what lands on stdout rather than stderr, and
//! whether `--fix` leaves a file that lints clean.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// The colour environment variables, so a test can pin them and nothing leaks in from the shell.
const COLOUR_VARIABLES: [&str; 5] = [
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "FORCE_COLOR",
    "TERM",
];

fn slint(arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slint"));
    for name in COLOUR_VARIABLES {
        command.env_remove(name);
    }
    command.args(arguments).output().expect("running slint")
}

/// Runs slint with the colour environment pinned: the named variables are set, every other colour
/// variable is removed, so the run sees exactly what the test says it should.
fn slint_with_environment(variables: &[(&str, &str)], arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slint"));
    for name in COLOUR_VARIABLES {
        command.env_remove(name);
    }
    for (name, value) in variables {
        command.env(name, value);
    }
    command.args(arguments).output().expect("running slint")
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Runs the binary from inside `directory`, so config discovery sees only what is there.
fn slint_in(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slint"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("running slint")
}

/// Runs the binary with `input` as stdin, the way an editor integration would.
fn slint_from_stdin(directory: &Path, arguments: &[&str], input: &str) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_slint"))
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("running slint");

    child
        .stdin
        .as_mut()
        .expect("piping stdin")
        .write_all(input.as_bytes())
        .expect("writing the document to stdin");

    child.wait_with_output().expect("waiting for slint")
}

#[test]
fn a_clean_base_exits_zero_and_says_so() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("Nothing to report"));
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/39 —
/// a `!`-prefixed pattern used to compile as a literal glob that nothing matches, so the
/// "keep this one back" half of the list was silently dropped.
#[test]
fn a_negated_pattern_takes_a_path_back_from_the_ignore_list() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();

    write(
        root.join("fixtures").as_path(),
        "keep-me",
        "---\nname: keep-me\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Keep\n\n1. Import the files.\n",
    );
    write(root.join("fixtures").as_path(), "drop-me", BROKEN);
    fs::write(
        root.join("slint.toml"),
        "ignore = [\"**/fixtures/**\", \"!**/fixtures/keep-me/**\"]\n",
    )
    .unwrap();

    let output = slint(&[root.to_str().unwrap(), "--no-llm"]);

    let stdout = stdout(&output);
    assert!(
        stdout.contains("keep-me"),
        "the negated pattern must take its folder back: {stdout}"
    );
    assert!(
        !stdout.contains("drop-me"),
        "the plain pattern must still ignore its folder: {stdout}"
    );
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/29 —
/// the first path's config governed every path, so a later path's own config was read by
/// nobody and said nothing.
#[test]
fn a_later_path_with_its_own_config_is_named_when_the_first_path_governs() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();

    // Both names are generic enough to trip name/not-generic, different enough not to trip
    // the project rules.
    write(
        root.join("alpha").as_path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );
    write(
        root.join("beta").as_path(),
        "utils",
        "---\nname: utils\ndescription: Restores a corrupted drive image by verifying the checksums and repacking the volumes. Use when a restore fails halfway through.\n---\n\n## Utils\n\n1. Verify the checksums.\n",
    );
    fs::write(
        root.join("beta").join("slint.toml"),
        "[rules]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();

    // alpha first: no config of its own, so the run uses defaults — but beta's file must be
    // named, because it is the file that does not get to speak.
    let output = slint(&[
        root.join("alpha").to_str().unwrap(),
        root.join("beta").to_str().unwrap(),
        "--no-llm",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.contains("slint.toml"),
        "beta's config must be named: {stderr}"
    );
    assert!(stderr.contains("beta"), "{stderr}");
    // And the first path's config really did govern: beta's "off" was not applied.
    assert!(stdout(&output).contains("name/not-generic"));
    assert_eq!(output.status.code(), Some(2), "{stderr}");

    // beta first: its config is the one that governs, and alpha has no file, so there is
    // nothing to warn about.
    let output = slint(&[
        root.join("beta").to_str().unwrap(),
        root.join("alpha").to_str().unwrap(),
        "--no-llm",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.is_empty(),
        "no config is ignored in this order: {stderr}"
    );
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/42 —
/// two config files in one directory used to let the second be read by nobody, with no sign.
#[test]
fn two_config_files_in_one_directory_say_which_one_wins() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    fs::write(
        temporary.path().join("slint.toml"),
        "[rules]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();
    fs::write(
        temporary.path().join("slint.config.json"),
        r#"{ "rules": { "name/not-generic": "off" } }"#,
    )
    .unwrap();

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.contains("slint.toml"),
        "the file that won must be named: {stderr}"
    );
    assert!(
        stderr.contains("slint.config.json"),
        "the file that was ignored must be named: {stderr}"
    );
    // The warning is a warning: it changes what is printed, not the result.
    assert_eq!(output.status.code(), Some(0), "{stderr}");
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/53 —
/// a typo in an LLM flag used to hard-fail a static run in which the value is never read,
/// the way a leftover flag from a shared script does.
#[test]
fn llm_flags_are_not_validated_when_the_model_pass_is_off() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--llm-provider",
        "bogus",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "a flag the run never reads must not fail it: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // With the model pass on, the same typo still fails loudly at the provider list.
    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--llm",
        "--llm-provider",
        "bogus",
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("bogus is not a provider"), "{stderr}");
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/41 —
/// there was no way to run slint without the ambient config, so a one-off run or a CI job
/// pinning exact behaviour had to move the project's real config file out of the way.
#[test]
fn no_config_runs_on_defaults_even_when_a_config_file_is_there() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();

    write(
        root,
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );
    fs::write(
        root.join("slint.toml"),
        "[rules]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();

    let output = slint(&[root.to_str().unwrap(), "--no-llm", "--no-config"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "the config file must not be read"
    );
    assert!(
        stdout(&output).contains("name/not-generic"),
        "the config's off must not apply"
    );
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
    assert_eq!(
        output.status.code(),
        Some(3),
        "a bad option is a config failure: {stderr}"
    );
    assert!(stderr.contains("body/max-lines"), "{stderr}");
}

#[test]
fn linting_a_file_that_is_not_a_skill_says_so_instead_of_passing() {
    // https://github.com/MaximeGaudin/slint/issues/36
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("random.txt"), "hello\n").unwrap();

    let output = slint(&[
        temporary.path().join("random.txt").to_str().unwrap(),
        "--no-llm",
    ]);

    assert_eq!(
        output.status.code(),
        Some(4),
        "nothing was linted, which is not a clean pass"
    );
    let text = format!("{}{}", stdout(&output), stderr(&output));
    assert!(text.contains("random.txt"), "{text}");
    assert!(text.contains("not linted"), "{text}");
}

#[test]
fn an_empty_directory_is_a_failure_not_a_clean_run() {
    // https://github.com/MaximeGaudin/slint/issues/118
    let temporary = tempfile::tempdir().unwrap();
    let empty = temporary.path().join("no-skills-here");
    fs::create_dir_all(&empty).unwrap();

    let output = slint(&[empty.to_str().unwrap(), "--no-llm"]);

    assert_eq!(
        output.status.code(),
        Some(4),
        "a run that checked zero skills must not read as success"
    );
    assert!(
        !stdout(&output).contains("Nothing to report"),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("No SKILL.md"),
        "{}",
        stdout(&output)
    );
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/52 —
/// `-` used to die with a doubled OS error that never said what slint does read, and a
/// missing path printed the same OS error twice.
#[test]
fn dash_names_the_stdin_flag_and_a_missing_path_is_said_once() {
    // `-` is the conventional stdin placeholder: point at the flag that does read stdin.
    let output = slint(&["-", "--no-llm"]);
    let errors = stderr(&output);
    assert_eq!(output.status.code(), Some(3), "{errors}");
    assert!(
        errors.contains("--stdin"),
        "the message must name the flag that reads stdin: {errors}"
    );
    assert!(!errors.contains("os error"), "no raw OS error: {errors}");

    // A path that does not exist is said once, where the report can show it, and a run
    // left with nothing to lint fails with the nothing-linted code, not a bare OS error.
    let output = slint(&["/definitely/does/not/exist", "--no-llm"]);
    let errors = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(4),
        "nothing was linted, which is not a clean pass: {errors}"
    );
    let text = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        text.contains("/definitely/does/not/exist"),
        "the run must name the path: {text}"
    );
    assert!(
        text.contains("not linted"),
        "the run must say the path was not linted: {text}"
    );
    assert!(
        !errors.contains("No such file or directory"),
        "no raw OS error on stderr: {errors}"
    );
}

#[test]
fn a_missing_path_among_others_is_said_once_and_the_run_continues() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[
        temporary.path().join("photo-culling").to_str().unwrap(),
        "/definitely/does/not/exist",
        "--no-llm",
    ]);

    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stdout(&output).contains("Nothing to report"));
    let text = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        text.contains("/definitely/does/not/exist"),
        "the run must name the path it could not lint: {text}"
    );
    assert!(
        !stderr(&output).contains("No such file or directory"),
        "no raw OS error: {}",
        stderr(&output)
    );
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
fn a_warning_budget_does_not_relabel_a_warnings_only_run() {
    // https://github.com/MaximeGaudin/slint/issues/143: with no errors the run is still
    // "warnings only" (2) even when --max-warnings is breached — the code names the finding
    // class, and 2 is non-zero, so CI still fails.
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

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_narrow_column_wraps_the_stylish_report_instead_of_running_past_the_pane() {
    // https://github.com/MaximeGaudin/slint/issues/164: the report never measured the
    // terminal, so a narrow split pane received 100+ column lines. COLUMNS pins the width a
    // terminal would have reported; stdout here is a pipe, so this exercises the whole
    // detection path end to end.
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let roomy = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    let narrow = slint_with_environment(
        &[("COLUMNS", "40")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    // The long advice line runs whole when there is room, and is folded when there is not —
    // with every word kept. (A 40-column pane is narrower than the report's own columns, so
    // finding rows degrade to their natural length there; the advice lines, the widest thing
    // the report draws, always wrap.)
    let advice = "Expand to at least 80 characters — what it does, then when to use it, in the words a request would use.";
    assert!(stdout(&roomy).contains(advice), "{:?}", stdout(&roomy));
    assert!(
        !stdout(&narrow).contains(advice),
        "the advice must not run past 40 columns: {:?}",
        stdout(&narrow)
    );
    for word in advice.split_whitespace() {
        assert!(
            stdout(&narrow).contains(word),
            "wrapping must not lose a word: missing {word}"
        );
    }
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/22 —
/// --quiet used to strip the warnings from the report before the exit code was computed,
/// so `--quiet` turned a warnings-only run into a clean exit. The budget verdict lives in
/// the JSON envelope; the exit code only ever says errors, warnings-only, or clean.
#[test]
fn a_quiet_run_still_enforces_the_warning_budget() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--quiet",
        "--max-warnings",
        "0",
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "--quiet must not flip a warnings-only run to a clean exit"
    );
    // And --quiet still does what it says: the warning itself is not printed.
    assert!(!stdout(&output).contains("name/not-generic"));
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/23 — same root cause:
/// --quiet is about what is printed, not about what the run found.
#[test]
fn a_quiet_run_keeps_the_exit_code_the_full_run_would_have() {
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm", "--quiet"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "--quiet hides the warning from the report, not from the exit code"
    );
    assert!(!stdout(&output).contains("name/not-generic"));
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
    // https://github.com/MaximeGaudin/slint/issues/169: whatever the count, the pluralisation
    // is real words now, never the "(s)" placeholder.
    assert!(stdout(&fixed).contains(" applied."), "{:?}", stdout(&fixed));
    assert!(!stdout(&fixed).contains("(s)"));

    let document = fs::read_to_string(temporary.path().join("photo-culling/SKILL.md")).unwrap();
    assert!(document.contains("references/formats.md"));
    assert!(!document.contains('\\'));

    let after = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    assert!(!stdout(&after).contains("body/posix-paths"));
}

#[test]
fn two_fixable_rules_on_one_file_converge_in_a_single_fix() {
    // Reproduces https://github.com/MaximeGaudin/slint/issues/91: a description fix and a
    // posix-path fix on the same SKILL.md used to conflict, so one was left for a second --fix.
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "photo-culling",
        "---\nname: photo-culling\ndescription: <b>Culls</b> a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files.\n---\n\n## Culling\n\n1. Read references\\formats.md.\n",
    );

    fs::create_dir_all(temporary.path().join("photo-culling/references")).unwrap();
    fs::write(
        temporary.path().join("photo-culling/references/formats.md"),
        "# Formats\n",
    )
    .unwrap();

    let fixed = slint(&[temporary.path().to_str().unwrap(), "--no-llm", "--fix"]);
    let text = stdout(&fixed);
    assert!(text.contains("2 fixes applied"), "{text}");
    assert!(!text.contains("(s)"), "{text}");

    let document = fs::read_to_string(temporary.path().join("photo-culling/SKILL.md")).unwrap();
    assert!(!document.contains("<b>"), "{document}");
    assert!(!document.contains('\\'), "{document}");
    assert!(document.contains("references/formats.md"));
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
fn the_json_envelope_says_the_same_thing_the_exit_code_does() {
    // Reproduces https://github.com/MaximeGaudin/slint/issues/24: the process fails when the
    // --max-warnings budget is exceeded, so the envelope's `ok` flag must say false too — a
    // caller that branches on it before parsing anything gets the same verdict as the shell
    // does. (https://github.com/MaximeGaudin/slint/issues/143: the exit code itself stays 2,
    // "warnings only", because there are no errors — the verdict is still non-zero.)
    let temporary = tempfile::tempdir().unwrap();
    write(
        temporary.path(),
        "helper",
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "json",
        "--max-warnings",
        "0",
    ]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");

    assert_eq!(output.status.code(), Some(2), "warnings only, no errors");
    assert_eq!(
        parsed["ok"], false,
        "the envelope must agree that the run did not pass"
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

/// Regression for https://github.com/MaximeGaudin/slint/issues/28 —
/// a setting for a rule that does not exist used to be stored and never consulted, a no-op
/// with zero diagnostic.
#[test]
fn a_typo_d_rule_name_in_the_config_fails_the_run_and_names_it() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("slint.toml"),
        "[rules]\n\"name/not-genric\" = \"off\"\n",
    )
    .unwrap();

    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(
        output.status.code(),
        Some(3),
        "a setting for a rule that does not exist must fail the run: {stderr}"
    );
    assert!(stderr.contains("name/not-genric"), "{stderr}");
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/26 —
/// `[rule]` instead of `[rules]` used to load as if the config file were empty, and the run
/// went on as if no config had been written.
#[test]
fn a_mis_spelled_top_level_section_fails_the_run_and_names_it() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("slint.toml"),
        "[rule]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();

    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(
        output.status.code(),
        Some(3),
        "a misspelled section must fail the run: {stderr}"
    );
    assert!(stderr.contains("rule"), "{stderr}");
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

    let output = slint(&[temporary.path().to_str().unwrap(), "--no-llm", "--plugins"]);

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
fn init_writes_a_config_and_leaves_an_existing_one_alone() {
    // https://github.com/MaximeGaudin/slint/issues/35: running init twice is an expected,
    // idempotent no-op, not a failure of slint itself.
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
        Some(0),
        "nothing was asked for and nothing is broken"
    );
    assert!(stderr(&second).contains("already exists"));
    assert!(
        temporary.path().join("slint.toml").is_file(),
        "it does not clobber what is there"
    );
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/40 —
/// init in a subdirectory used to write a shadowing config without saying that a config
/// already governs the directory.
#[test]
fn init_in_a_subdirectory_names_the_parent_config_it_will_shadow() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();

    fs::write(
        root.join("slint.toml"),
        "[rules]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();
    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_slint"))
        .arg("init")
        .current_dir(&sub)
        .output()
        .expect("running slint");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.contains("slint.toml"),
        "the parent config must be named: {stderr}"
    );
    assert!(
        stderr.contains("shadow"),
        "what writing here will do must be said: {stderr}"
    );
    // The file is still written — init's job is to write, the warning is the news.
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(sub.join("slint.toml").is_file());

    // Without a config anywhere up the tree, there is nothing to warn about.
    let empty = tempfile::tempdir().unwrap();
    let quiet = Command::new(env!("CARGO_BIN_EXE_slint"))
        .arg("init")
        .current_dir(empty.path())
        .output()
        .expect("running slint");

    let stderr = String::from_utf8_lossy(&quiet.stderr).to_string();
    assert!(!stderr.contains("shadow"), "{stderr}");
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

#[test]
fn a_base_url_that_came_from_the_config_warns_before_anything_is_sent() {
    let temporary = tempfile::tempdir().unwrap();

    fs::write(
        temporary.path().join("slint.toml"),
        "[llm]\nprovider = \"openai\"\nmodel = \"gpt-mock\"\napi_key_env = \"SLINT_TEST_EXFIL_KEY\"\nbase_url = \"https://attacker.example/v1\"\n",
    )
    .unwrap();

    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[temporary.path().to_str().unwrap(), "--llm"]);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stderr.contains("attacker.example"),
        "a config-file base_url must be called out loudly before the key is sent: {stderr}"
    );
    assert!(
        stderr.contains("SLINT_TEST_EXFIL_KEY"),
        "the warning must name the variable whose key goes to that address: {stderr}"
    );

    // The same address, chosen on the command line, is the user's own decision: no warning.
    let overridden = slint(&[
        temporary.path().to_str().unwrap(),
        "--llm",
        "--llm-base-url",
        "https://openrouter.ai/api/",
    ]);
    let overridden_stderr = String::from_utf8_lossy(&overridden.stderr).to_string();
    assert!(
        !overridden_stderr.contains("attacker.example"),
        "an address the user typed is not the config's to warn about: {overridden_stderr}"
    );
}

#[test]
fn a_plugin_named_by_the_config_does_not_run_unless_asked_for() {
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

    // A config found inside the scanned tree names code that belongs to that tree. Running it
    // without being asked is the failure this test exists to keep out.
    let without = slint(&[temporary.path().to_str().unwrap(), "--no-llm"]);
    assert_eq!(
        without.status.code(),
        Some(0),
        "a plugin named by the scanned project's own config must not run by default"
    );
    assert!(!stdout(&without).contains("house/no-todo"));

    let with = slint(&[temporary.path().to_str().unwrap(), "--no-llm", "--plugins"]);
    assert_eq!(
        with.status.code(),
        Some(1),
        "opting in runs the pack's rules"
    );
    assert!(stdout(&with).contains("house/no-todo"));
}

// ---- https://github.com/MaximeGaudin/slint/issues/54 — CLI affordances ----

#[test]
fn stdin_is_linted_without_a_file_on_disk() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_from_stdin(temporary.path(), &["--stdin", "--no-llm"], BROKEN);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdin linting finds the error"
    );
    let text = stdout(&output);
    assert!(text.contains("bundle/no-dangling-path"), "{text}");
}

#[test]
fn a_clean_stdin_document_exits_zero() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_from_stdin(temporary.path(), &["--stdin", "--no-llm"], GOOD);

    assert_eq!(output.status.code(), Some(0));
}

/// Regression for https://github.com/MaximeGaudin/slint/issues/22 — same root cause on the
/// stdin path: --quiet is about what is printed, not about what the run found.
#[test]
fn a_quiet_stdin_run_keeps_the_warnings_only_exit_code() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_from_stdin(
        temporary.path(),
        &["--stdin", "--no-llm", "--quiet"],
        "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n1. Import the files.\n",
    );

    assert_eq!(
        output.status.code(),
        Some(2),
        "--quiet hides the warning from the report, not from the exit code"
    );
    assert!(!stdout(&output).contains("name/not-generic"));
}

#[test]
fn stdin_filename_names_the_file_in_the_report() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_from_stdin(
        temporary.path(),
        &[
            "--stdin",
            "--stdin-filename",
            "skills/helper/SKILL.md",
            "--no-llm",
            "--format",
            "compact",
        ],
        BROKEN,
    );

    let text = stdout(&output);
    assert!(text.starts_with("skills/helper/SKILL.md:"), "{text}");
}

#[test]
fn print_config_shows_the_resolved_config_as_json() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("slint.toml"),
        "[rules]\n\"name/not-generic\" = \"off\"\n",
    )
    .unwrap();

    let output = slint_in(
        temporary.path(),
        &["--print-config", "--no-llm", "--rule", "body/max-lines=off"],
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("print-config prints valid JSON");

    assert_eq!(parsed["rules"]["name/not-generic"], "off");
    assert_eq!(
        parsed["rules"]["body/max-lines"], "off",
        "command-line overrides are part of the resolved config"
    );
    assert!(parsed["ignore"].is_array());
    assert!(parsed["llm"].is_object());
}

#[test]
fn explain_prints_one_rule_rather_than_the_whole_catalogue() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_in(temporary.path(), &["--explain", "body/max-lines"]);
    let text = stdout(&output);

    assert_eq!(output.status.code(), Some(0));
    assert!(text.contains("body/max-lines"), "{text}");
    assert!(
        !text.contains("description/says-when"),
        "one rule was asked for:\n{text}"
    );
}

#[test]
fn explaining_an_unknown_rule_fails_the_run() {
    let temporary = tempfile::tempdir().unwrap();

    let output = slint_in(temporary.path(), &["--explain", "no/such-rule"]);

    assert_eq!(output.status.code(), Some(3));
    let complaints = stderr(&output);
    assert!(complaints.contains("no/such-rule"), "{complaints}");
}

#[test]
fn the_sarif_format_is_a_valid_sarif_log() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "sarif",
    ]);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("sarif prints valid JSON");

    assert_eq!(parsed["version"], "2.1.0");
    assert_eq!(parsed["runs"][0]["tool"]["driver"]["name"], "slint");

    let results = parsed["runs"][0]["results"].as_array().expect("results");
    let dangling = results
        .iter()
        .find(|result| result["ruleId"] == "bundle/no-dangling-path")
        .expect("the error is in the results");

    assert_eq!(dangling["level"], "error");
    assert!(
        dangling["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("SKILL.md")
    );
    assert!(
        dangling["locations"][0]["physicalLocation"]["region"]["startLine"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

// https://github.com/MaximeGaudin/slint/issues/172: CI systems that render test reports —
// Jenkins, GitLab — read JUnit XML, not annotations.
#[test]
fn the_junit_format_is_xml_a_ci_can_ingest() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "junit",
    ]);

    let text = stdout(&output);

    assert!(
        text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{text}"
    );
    assert!(text.contains("<testsuites name=\"slint\""), "{text}");
    assert!(text.contains("<testsuite name="), "{text}");
    assert!(
        text.contains("<error message=") || text.contains("<failure message="),
        "the dangling path is at least one failed testcase:\n{text}"
    );
    assert!(text.contains("bundle/no-dangling-path"), "{text}");
}

#[test]
fn the_junit_format_reports_a_clean_run_as_a_passing_testcase() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint(&[
        temporary.path().to_str().unwrap(),
        "--no-llm",
        "--format",
        "junit",
    ]);

    let text = stdout(&output);

    assert!(text.contains("failures=\"0\" errors=\"0\""), "{text}");
    assert!(
        text.contains("<testcase"),
        "at least one testcase exists: {text}"
    );
}

// Colour is decided by more than the `--no-color` flag: the standard environment conventions
// (https://no-color.org and https://bixense.com/clicolors/) apply as well. These tests run the
// binary with stdout piped, which is never a terminal, so anything coloured must have been forced.

#[test]
fn clicolor_force_colours_the_report_even_when_stdout_is_a_pipe() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        stdout(&output).contains('\x1b'),
        "CLICOLOR_FORCE should force ANSI colour into the piped report:\n{}",
        stdout(&output)
    );
}

#[test]
fn ignore_path_adds_patterns_from_a_file() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    write(temporary.path(), "helper", BROKEN);
    fs::write(temporary.path().join("my-ignores"), "**/helper\n").unwrap();

    let output = slint_in(
        temporary.path(),
        &["--no-llm", "--ignore-path", "my-ignores"],
    );

    assert_eq!(output.status.code(), Some(0), "helper is ignored");
    let text = stdout(&output);
    assert!(!text.contains("bundle/no-dangling-path"), "{text}");
}

#[test]
fn no_ignore_lints_what_the_config_ignored() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("slint.toml"),
        "ignore = [\"**/helper\"]\n",
    )
    .unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    write(temporary.path(), "helper", BROKEN);

    let ignored = slint_in(temporary.path(), &["--no-llm"]);
    assert_eq!(ignored.status.code(), Some(0), "the config ignores it");

    let output = slint_in(temporary.path(), &["--no-llm", "--no-ignore"]);
    assert_eq!(output.status.code(), Some(1), "--no-ignore lints it anyway");
    assert!(stdout(&output).contains("bundle/no-dangling-path"));
}

#[test]
fn completions_can_be_printed_for_the_common_shells() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let output = slint(&["completions", shell]);

        assert_eq!(output.status.code(), Some(0), "{shell}");
        assert!(
            !stdout(&output).is_empty(),
            "completions for {shell} are not empty"
        );
    }
}

#[test]
fn verbose_says_which_config_was_used() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    fs::write(temporary.path().join("slint.toml"), "[rules]\n").unwrap();

    let output = slint_in(temporary.path(), &["--no-llm", "--verbose"]);

    assert_eq!(output.status.code(), Some(0));
    let complaints = stderr(&output);
    assert!(complaints.contains("slint.toml"), "{complaints}");
}

#[test]
fn verbose_without_a_config_says_defaults_are_in_use() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);

    let output = slint_in(temporary.path(), &["--no-llm", "--verbose"]);

    assert_eq!(output.status.code(), Some(0));
    let complaints = stderr(&output);
    assert!(complaints.contains("config"), "{complaints}");
}

// ---- https://github.com/MaximeGaudin/slint/issues/55 — a JSON Schema for the config ----

#[test]
fn slint_prints_a_json_schema_for_its_config_format() {
    let output = slint(&["schema"]);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("schema prints valid JSON");

    assert!(parsed["$schema"].is_string(), "it declares its dialect");
    assert!(parsed["properties"]["rules"].is_object());
    assert!(parsed["properties"]["ignore"].is_object());
    assert!(parsed["properties"]["llm"].is_object());
    assert!(parsed["properties"]["plugins"].is_object());

    let provider = &parsed["$defs"]["LlmConfig"]["properties"]["provider"];
    let providers: Vec<&serde_json::Value> = parsed["$defs"]["Provider"]["oneOf"]
        .as_array()
        .expect("provider is a choice of named constants")
        .iter()
        .map(|variant| &variant["const"])
        .collect();
    assert!(provider["$ref"].is_string(), "llm names its provider type");
    assert!(
        providers.iter().any(|provider| **provider == "openrouter"),
        "every provider the binary accepts is in the schema: {providers:?}"
    );
}

#[test]
fn the_committed_schema_is_the_one_the_binary_prints() {
    // The file the docs site and the editor extension publish has to be the schema this binary
    // would print, or an editor would autocomplete a config slint does not read. Regenerate it
    // with `slint schema > apps/docs/public/schemas/slint-config.json` after changing the format.
    let committed: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/public/schemas/slint-config.json"))
            .expect("the committed schema is valid JSON");

    let generated: serde_json::Value =
        serde_json::from_str(&stdout(&slint(&["schema"]))).expect("schema prints valid JSON");

    assert_eq!(generated, committed);
}

// ---- https://github.com/MaximeGaudin/slint/issues/56 — a user-global config ----

/// Sets `XDG_CONFIG_HOME` for one run and puts a user config beneath it.
///
/// The config only sets a provider and no model, so no other test's run changes
/// behaviour while the variable is set: the model pass stays unconfigured.
struct UserConfig {
    _home: tempfile::TempDir,
}

impl UserConfig {
    fn with_provider(provider: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join("slint");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("config.toml"),
            format!("[llm]\nprovider = \"{provider}\"\n"),
        )
        .unwrap();

        // SAFETY: the binary is a separate process, and the variable is removed when the guard
        // drops, before any later test reads the environment.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", home.path()) };
        UserConfig { _home: home }
    }
}

impl Drop for UserConfig {
    fn drop(&mut self) {
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    }
}

#[test]
fn a_user_global_config_is_used_when_no_project_config_exists() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    let _user = UserConfig::with_provider("openai");

    let output = slint_in(temporary.path(), &["--print-config", "--no-llm"]);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("print-config prints valid JSON");
    assert_eq!(
        parsed["llm"]["provider"], "openai",
        "the user config is the fallback: {parsed}"
    );
}

#[test]
fn a_project_config_wins_over_the_user_config() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "photo-culling", GOOD);
    fs::write(
        temporary.path().join("slint.toml"),
        "[llm]\nprovider = \"groq\"\n",
    )
    .unwrap();
    let _user = UserConfig::with_provider("openai");

    let output = slint_in(temporary.path(), &["--print-config", "--no-llm"]);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("print-config prints valid JSON");
    assert_eq!(
        parsed["llm"]["provider"], "groq",
        "the project's own config is what counts: {parsed}"
    );
}

#[test]
fn force_color_colours_the_report_even_when_stdout_is_a_pipe() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("FORCE_COLOR", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        stdout(&output).contains('\x1b'),
        "FORCE_COLOR should force ANSI colour into the piped report:\n{}",
        stdout(&output)
    );
}

#[test]
fn no_color_beats_being_forced() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        !stdout(&output).contains('\x1b'),
        "NO_COLOR should keep the report plain"
    );
}

#[test]
fn clicolor_zero_beats_being_forced() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("CLICOLOR", "0"), ("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        !stdout(&output).contains('\x1b'),
        "CLICOLOR=0 should keep the report plain"
    );
}

#[test]
fn term_dumb_beats_being_forced() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("TERM", "dumb"), ("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        !stdout(&output).contains('\x1b'),
        "TERM=dumb should keep the report plain"
    );
}

#[test]
fn the_no_color_flag_beats_being_forced() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm", "--no-color"],
    );

    assert!(
        !stdout(&output).contains('\x1b'),
        "--no-color should keep the report plain"
    );
}

#[test]
fn an_empty_no_color_is_not_a_request_to_stop() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("NO_COLOR", ""), ("CLICOLOR_FORCE", "1")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        stdout(&output).contains('\x1b'),
        "an empty NO_COLOR is treated as unset by the convention"
    );
}

#[test]
fn clicolor_force_zero_does_not_force_anything() {
    let temporary = tempfile::tempdir().unwrap();
    write(temporary.path(), "helper", BROKEN);

    let output = slint_with_environment(
        &[("CLICOLOR_FORCE", "0")],
        &[temporary.path().to_str().unwrap(), "--no-llm"],
    );

    assert!(
        !stdout(&output).contains('\x1b'),
        "CLICOLOR_FORCE=0 leaves the piped report plain"
    );
}

#[test]
fn a_closed_pipe_exits_cleanly_instead_of_failing_the_run() {
    let temporary = tempfile::tempdir().unwrap();

    // One skill with thousands of findings, so the report is far larger than any pipe buffer and
    // the writer is guaranteed to hit the closed end rather than drain into it, which is what
    // `slint <path> | head -1` does once head has its line.
    let steps: String = (0..2_000)
        .map(|index| format!("{index}. Read scripts\\notes-{index}.md.\n"))
        .collect();
    write(
        temporary.path(),
        "helper",
        &format!(
            "---\nname: helper\ndescription: Culls a photo shoot in Lightroom by flagging the keepers and rejecting the rest. Use when triaging RAW files after a session.\n---\n\n## Helper\n\n{steps}"
        ),
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_slint"))
        .arg(temporary.path().join("helper"))
        .args(["--no-llm"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawning slint");

    // The reader hangs up before the report arrives, as head does.
    drop(child.stdout.take());

    let status = child.wait().expect("waiting for slint");

    assert_eq!(
        status.code(),
        Some(0),
        "a downstream reader closing the pipe is not a lint failure"
    );
}
