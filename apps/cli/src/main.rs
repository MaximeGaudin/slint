//! The command line.
//!
//! Exit codes follow the convention the other skill linters settled on, so a CI script written for
//! one works here: 0 clean, 1 errors, 2 warnings only, 3 slint itself failed, 4 nothing was linted.
//! Data goes to stdout and everything else to stderr, so `slint --format json | jq` is always safe.

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use slint::config::{self, Config, RuleSetting};
use slint::diagnostics::Report;
use slint::engine::{self, Passes};
use slint::plugin;
use slint::report::{self, Format};
use slint::rules::{self, RuleMeta};

/// Exit codes, named.
mod code {
    pub const CLEAN: u8 = 0;
    pub const ERRORS: u8 = 1;
    pub const WARNINGS: u8 = 2;
    pub const FAILED: u8 = 3;
    /// The run looked at zero skills: a typo'd path must not read as success.
    pub const NOTHING_LINTED: u8 = 4;
}

#[derive(Parser, Debug)]
#[command(
    name = "slint",
    version,
    about = "The linter for Agent Skills.",
    long_about = "Lints SKILL.md files and the bundles beside them.\n\nEverything that can be answered from the text is answered from the text: no network, no tokens, no waiting. The rules that need a reader are answered by whichever model the project configured, and the report always says which half ran."
)]
struct Cli {
    /// Directories or SKILL.md files to lint. Defaults to the current directory.
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Lint the document on stdin instead of the paths. Static rules only: the bundle beside a
    /// stdin document does not exist.
    #[arg(long)]
    stdin: bool,

    /// The name to report for the stdin document, and where its config is found from.
    #[arg(long = "stdin-filename", value_name = "PATH")]
    stdin_filename: Option<String>,

    /// Print the fully-resolved config as JSON — file, flags and all — and do not lint.
    #[arg(long)]
    print_config: bool,

    /// Print one rule's catalogue entry, then stop.
    #[arg(long, value_name = "RULE")]
    explain: Option<String>,

    /// Apply every computed fix, then lint again.
    #[arg(long)]
    fix: bool,

    /// How to print the report.
    #[arg(long, default_value = "stylish")]
    format: Format,

    /// Use this config rather than looking for one.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Look for no config at all: the built-in defaults and the command line only.
    #[arg(long, conflicts_with = "config")]
    no_config: bool,

    /// Read extra ignore patterns from this file, one glob per line.
    #[arg(long = "ignore-path", value_name = "FILE")]
    ignore_path: Option<PathBuf>,

    /// Lint everything, even what the config and --ignore-path would skip.
    #[arg(long)]
    no_ignore: bool,

    /// Run the rules that need a model. Off by default: a linter must not spend money uninvited.
    #[arg(long = "llm", visible_alias = "enable-llm-rules")]
    llm: bool,

    /// Kept so existing scripts keep working. The model pass is off unless --llm is given.
    #[arg(long, hide = true)]
    no_llm: bool,

    /// Override [llm].provider from the config (editors and one-off runs).
    #[arg(long = "llm-provider", value_name = "NAME")]
    llm_provider: Option<String>,

    /// Override [llm].model from the config.
    #[arg(long = "llm-model", value_name = "ID")]
    llm_model: Option<String>,

    /// Override [llm].base_url from the config.
    #[arg(long = "llm-base-url", value_name = "URL")]
    llm_base_url: Option<String>,

    /// Override [llm].api_key_env: the environment variable that holds the key, never the key.
    #[arg(long = "llm-api-key-env", value_name = "VAR")]
    llm_api_key_env: Option<String>,

    /// Skip plugins, whatever the config says.
    #[arg(long)]
    no_plugins: bool,

    /// Override one rule: --rule name/thing=error, repeatable.
    #[arg(long = "rule", value_name = "NAME=LEVEL")]
    overrides: Vec<String>,

    /// Fail when there are more warnings than this. -1 never fails on warnings.
    #[arg(long, default_value = "-1", allow_negative_numbers = true)]
    max_warnings: i64,

    /// Print errors only.
    #[arg(long, short)]
    quiet: bool,

    /// Say what the run is doing — which config, how many plugins — on stderr.
    #[arg(long, short = 'v')]
    verbose: bool,

    /// Never colour the output. Beyond the flag, colour follows the usual environment conventions:
    /// NO_COLOR, CLICOLOR=0, and TERM=dumb turn it off; CLICOLOR_FORCE and FORCE_COLOR turn it on;
    /// and a stdout that is not a terminal is plain.
    #[arg(long)]
    no_color: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    /// Whether the rules that need a model should run.
    ///
    /// Off unless asked for. A linter that reaches a paid provider because someone typed its name
    /// is a linter people run once — and the static half is the half that answers instantly.
    fn model_pass(&self) -> bool {
        self.llm && !self.no_llm
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write a starter config in the current directory. An existing one is left alone.
    Init,
    /// Write a starter rule pack, to be edited and pointed at from the config. An existing one is
    /// left alone.
    InitPlugin,
    /// Print the rule catalogue: what each rule checks, and where the claim comes from.
    Rules {
        /// As JSON, which is what the documentation site is built from.
        #[arg(long)]
        json: bool,
    },
    /// Print the config file format as JSON Schema, for editor autocomplete.
    Schema,
    /// Write shell completions for the given shell to stdout.
    Completions {
        /// The shell whose completions to write.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            // Everything that is not the report goes to stderr, so piping stdout stays safe.
            eprintln!("slint: {failure:#}");
            ExitCode::from(code::FAILED)
        }
    }
}

fn run(cli: &Cli) -> Result<u8> {
    match &cli.command {
        Some(Command::Init) => return init(),
        Some(Command::InitPlugin) => return init_plugin(),
        Some(Command::Rules { json }) => return print_rules(*json),
        Some(Command::Schema) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&config::config_json_schema())?
            );
            return Ok(code::CLEAN);
        }
        Some(Command::Completions { shell }) => return print_completions(*shell),
        None => {}
    }

    if let Some(rule) = &cli.explain {
        return explain_rule(rule);
    }

    let mut config = resolve_config(cli)?;

    if let Some(path) = &cli.ignore_path {
        config.ignore.extend(read_ignore_file(path)?);
    }

    if cli.no_ignore {
        config.ignore.clear();
    }

    if cli.print_config {
        return print_effective_config(&config);
    }

    if cli.stdin {
        if cli.fix {
            bail!("--stdin cannot be combined with --fix: there is no file to rewrite");
        }

        return run_stdin(cli, &config);
    }

    let plugins = if cli.no_plugins {
        Vec::new()
    } else {
        plugin::load_all(&config)?
    };

    config::check_rule_names(&config, &plugins)?;

    if cli.verbose {
        say_what_the_run_is(&config, &plugins, cli);
    }

    let passes = Passes {
        plugins: !cli.no_plugins,
        model: cli.model_pass(),
    };

    let mut report = engine::run(&cli.paths, &config, &plugins, passes)?;

    if cli.fix {
        report = fix_until_converged(report, slint::fix::apply, |_report| {
            // Lint again so the report describes the files as they are now rather than as they
            // were: half of what --fix does is proving the fix worked.
            engine::run(&cli.paths, &config, &plugins, passes)
        })?;
    }

    if cli.quiet {
        keep_only_errors(&mut report);
    }

    let colour = colour_allowed(cli.no_color);
    let text = report::render(&report, cli.format, colour, cli.max_warnings);

    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        Ok(()) => {}
        // A downstream reader that hung up (`slint … | head`) is not a failure of the run: the
        // report went wherever it was still wanted, and the Unix convention is to bow out
        // quietly rather than complain about a pipe nobody is reading.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => return Ok(code::CLEAN),
        Err(error) => return Err(error).context("writing the report"),
    }

    Ok(exit_code(&report))
}

/// Reads, lints and reports on the document coming in on stdin.
///
/// Only the static rules run: a stdin document has no bundle beside it, so the bundle rules and a
/// model review would answer a question about files that do not exist. The note in the report says
/// so rather than leaving the reader guessing.
fn run_stdin(cli: &Cli, config: &Config) -> Result<u8> {
    let mut source = String::new();
    std::io::stdin()
        .read_to_string(&mut source)
        .context("reading the document from stdin")?;

    let name = cli
        .stdin_filename
        .clone()
        .unwrap_or_else(|| "<stdin>".to_string());

    let mut skill = slint::skill::parse(&source);
    skill.document = name.clone();

    if let Some(parent) = Path::new(&name)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        skill.directory = parent.to_path_buf();
    }

    if skill.name.is_empty() {
        skill.name = Path::new(&name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("stdin")
            .to_string();
    }

    let mut messages = engine::lint_skill(&skill, config);

    // Anything the document itself asked not to hear about still applies.
    let suppressions = engine::Suppressions::read(&source);
    messages.retain(|message| suppressions.allows(&skill.document, message));

    let mut notes = skill.notes.clone();
    if cli.model_pass() {
        notes.push(
            "The model rules do not run with --stdin. Save the file and lint the path instead."
                .to_string(),
        );
    } else {
        let count = slint::llm::rules::all().len();
        notes.push(format!(
            "Skipped {count} model rules (not requested). Pass --llm to run them."
        ));
    }

    let mut report = Report {
        skills: vec![slint::SkillReport {
            path: name.clone(),
            name: skill.name,
            messages,
            notes,
        }],
        fixed: 0,
        notes: Vec::new(),
    }
    .sorted();

    if cli.quiet {
        keep_only_errors(&mut report);
    }

    let colour = colour_allowed(cli.no_color);
    let text = report::render(&report, cli.format, colour, cli.max_warnings);

    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        Ok(()) => {}
        // A downstream reader that hung up (`slint … | head`) is not a failure of the run: the
        // report went wherever it was still wanted, and the Unix convention is to bow out
        // quietly rather than complain about a pipe nobody is reading.
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => return Ok(code::CLEAN),
        Err(error) => return Err(error).context("writing the report"),
    }

    Ok(exit_code(&report))
}

/// --print-config: the config as a caller can now rely on it, file and flags together.
fn print_effective_config(config: &Config) -> Result<u8> {
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "{}",
        serde_json::to_string_pretty(config).context("writing the resolved config")?
    )?;

    Ok(code::CLEAN)
}

/// --explain: one rule, with the same prose the catalogue prints.
fn explain_rule(name: &str) -> Result<u8> {
    let Some(meta) = rules::meta_for(name) else {
        bail!("no rule named {name} — try `slint rules` for the catalogue");
    };

    print_rule(meta);

    Ok(code::CLEAN)
}

/// Extra ignore patterns, one glob per line, `#` comments allowed.
fn read_ignore_file(path: &Path) -> Result<Vec<String>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// -v/--verbose: what the run is standing on, said before anything can go wrong.
fn say_what_the_run_is(config: &Config, plugins: &[plugin::Plugin], cli: &Cli) {
    let source = match &config.source {
        Some(path) => path.display().to_string(),
        None => "no config file found; defaults".to_string(),
    };

    eprintln!("slint: config: {source}");

    if !cli.no_plugins {
        eprintln!("slint: plugins: {}", plugins.len());
    }

    let model = if cli.model_pass() {
        "model pass on"
    } else {
        "model pass off"
    };
    eprintln!("slint: {model}");
}

fn print_completions(shell: clap_complete::Shell) -> Result<u8> {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut std::io::stdout().lock());

    Ok(code::CLEAN)
}

/// Whether the report may be coloured, from the flag, the environment, and stdout.
///
/// The conventions this follows are https://no-color.org and https://bixense.com/clicolors/, so
/// slint behaves like every other tool in a pipeline. Highest first:
///
/// 1. `--no-color`, an explicit refusal on the command line.
/// 2. NO_COLOR set to anything non-empty, CLICOLOR=0, or TERM=dumb — each turns colour off.
/// 3. CLICOLOR_FORCE or FORCE_COLOR set to anything non-empty but "0" — colour on, even piped.
/// 4. Otherwise colour only when stdout is a terminal.
fn colour_allowed(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }

    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }

    if std::env::var("CLICOLOR").is_ok_and(|value| value == "0") {
        return false;
    }

    if std::env::var("TERM").is_ok_and(|value| value == "dumb") {
        return false;
    }

    let forced = ["CLICOLOR_FORCE", "FORCE_COLOR"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty() && value != "0"));
    if forced {
        return true;
    }

    std::io::stdout().is_terminal()
}

/// What the run means, as a number.
///
/// The code names the class of what was found, so a CI script can tell an error it must read
/// from a warning it chose to tolerate: errors are 1, warnings-only is 2 — even when the
/// `--max-warnings` budget is breached, because relabelling a warnings-only run as "errors"
/// would contradict the documented contract. The budget verdict lives in the JSON envelope's
/// `ok` flag instead, and a warnings-only run still fails a CI job the usual way: 2 is non-zero.
fn exit_code(report: &Report) -> u8 {
    // Nothing was looked at: a typo'd path or an empty directory is not a clean pass, and a CI
    // job pointed at the wrong directory must not sit green forever.
    if report.skills.is_empty() {
        return code::NOTHING_LINTED;
    }

    if report.errors() > 0 {
        return code::ERRORS;
    }

    if report.warnings() > 0 {
        return code::WARNINGS;
    }

    code::CLEAN
}

/// Rounds one `--fix` invocation may take. Each round re-lints and applies what the new pass
/// found; the bound only matters if a fix never settles, so the report still gets out.
const MAX_FIX_ROUNDS: usize = 8;

/// Applies fixes and re-lints until a pass has nothing left to fix.
///
/// One round can leave work behind: two fixes computed against the same text overlap, and one of
/// them is deferred rather than applied against a moved target. Re-linting recomputes the
/// remaining fixes against the file as it is now, so a single `--fix` invocation still converges
/// instead of sending the user off to run it a second time.
fn fix_until_converged(
    report: Report,
    mut apply: impl FnMut(&Report) -> Result<slint::fix::Applied>,
    mut relint: impl FnMut(Report) -> Result<Report>,
) -> Result<Report> {
    let mut report = report;
    let mut rounds = 0;
    let mut total = 0;

    loop {
        let applied = apply(&report)?;

        if applied.fixes == 0 {
            return Ok(report);
        }

        total += applied.fixes;
        report = relint(report)?;
        report.fixed = total;

        rounds += 1;
        if rounds == MAX_FIX_ROUNDS {
            return Ok(report);
        }
    }
}

fn resolve_config(cli: &Cli) -> Result<Config> {
    let path = if cli.no_config {
        None
    } else {
        resolve_config_path(cli)
    };

    let mut config = match path {
        Some(path) => config::load(&path)?,
        None => Config::default(),
    };

    for text in &cli.overrides {
        let (name, setting) = config::parse_override(text)?;
        config.rules.insert(name, setting);
    }

    apply_llm_overrides(&mut config, cli)?;

    if !cli.model_pass() {
        for meta in slint::llm::rules::all() {
            config.rules.insert(meta.name.to_string(), RuleSetting::Off);
        }
    }

    Ok(config)
}

/// The explicit `--config` file, or the one found by walking up from the first path.
fn resolve_config_path(cli: &Cli) -> Option<PathBuf> {
    let path = &cli.config;

    match path {
        Some(path) => Some(path.clone()),
        None => {
            let from = cli
                .paths
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("."));

            let anchor = if from.is_dir() {
                from
            } else {
                from.parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
            };

            config::find(&anchor)
        }
    }
}

/// Flags from the editor (or a one-off shell) win over whatever the file said.
fn apply_llm_overrides(config: &mut Config, cli: &Cli) -> Result<()> {
    if let Some(name) = &cli.llm_provider {
        config.llm.provider = slint::config::Provider::parse(name)?;
    }

    if let Some(model) = &cli.llm_model {
        config.llm.model = model.clone();
    }

    if let Some(url) = &cli.llm_base_url {
        config.llm.base_url = Some(url.clone());
    }

    if let Some(variable) = &cli.llm_api_key_env {
        config.llm.api_key_env = Some(variable.clone());
    }

    Ok(())
}

fn init() -> Result<u8> {
    let path = PathBuf::from("slint.toml");

    if path.exists() {
        // An expected no-op, not a failure: nothing was asked for and nothing is broken, so the
        // exit code stays clean and the explanation goes to stderr, where the report never goes.
        eprintln!(
            "slint: {} already exists, so nothing was written",
            path.display()
        );
        return Ok(code::CLEAN);
    }

    // A config higher up the tree already governs this directory, and the file being written
    // now takes its place for everything linted below. That is a decision worth seeing. The
    // directory is canonicalized first, because the walk-up stops at a relative anchor's parent
    // of ".", which is the empty path.
    let here = std::fs::canonicalize(".").with_context(|| "resolving the current directory")?;
    if let Some(inherited) = config::find(&here) {
        eprintln!(
            "slint: {} already governs this directory; writing {} will shadow it for everything linted below",
            inherited.display(),
            path.display()
        );
    }

    std::fs::write(&path, config::STARTER_CONFIG)
        .with_context(|| format!("writing {}", path.display()))?;

    println!(
        "Wrote {}. Every rule is on; the file is for the ones you disagree with.",
        path.display()
    );
    Ok(code::CLEAN)
}

fn init_plugin() -> Result<u8> {
    let path = PathBuf::from("slint-house-rules.toml");

    if path.exists() {
        // As with init: an expected no-op, not a failure of slint itself.
        eprintln!(
            "slint: {} already exists, so nothing was written",
            path.display()
        );
        return Ok(code::CLEAN);
    }

    std::fs::write(&path, plugin::STARTER_PACK)
        .with_context(|| format!("writing {}", path.display()))?;

    println!(
        "Wrote {}. Point at it from slint.toml:\n\n[[plugins]]\npath = \"./{}\"",
        path.display(),
        path.display()
    );
    Ok(code::CLEAN)
}

fn print_rules(as_json: bool) -> Result<u8> {
    let all = rules::all_meta();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&all)?);
        return Ok(code::CLEAN);
    }

    for meta in all {
        print_rule(meta);
    }

    Ok(code::CLEAN)
}

/// One rule's entry in the catalogue, as prose.
fn print_rule(meta: &RuleMeta) {
    let needs = if meta.needs_model {
        " · needs a model"
    } else {
        ""
    };
    let fixable = if meta.fixable { " · fixable" } else { "" };

    println!("{}  [{}{needs}{fixable}]", meta.name, meta.default_severity);
    println!("    {}", meta.summary);
    println!("    {}", meta.rationale);
    println!("    → {}", meta.advice);
    println!("    {} — {}\n", meta.reference_title, meta.reference_url);
}

/// --quiet: the warnings can come back when they are wanted.
fn keep_only_errors(report: &mut Report) {
    for skill in &mut report.skills {
        skill
            .messages
            .retain(|message| message.severity == slint::Severity::Error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};
    use std::fs;
    use std::path::Path;

    fn report_with(errors: usize, warnings: usize) -> Report {
        let message = |severity: Severity| Message {
            rule: "a/rule".into(),
            severity,
            message: "m".into(),
            advice: "a".into(),
            location: Location::at(1, 1),
            source: Source::Static,
            file: "SKILL.md".into(),
            fix: None,
            reference: Reference {
                title: "t".into(),
                url: "https://example.com".into(),
            },
            confidence: 1.0,
        };

        let mut messages = vec![message(Severity::Error); errors];
        messages.extend(vec![message(Severity::Warning); warnings]);

        Report {
            skills: vec![SkillReport {
                path: "skills/a".into(),
                name: "a".into(),
                messages,
                notes: vec![],
            }],
            fixed: 0,
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_clean_run_exits_zero() {
        assert_eq!(exit_code(&report_with(0, 0)), code::CLEAN);
    }

    #[test]
    fn errors_exit_one() {
        assert_eq!(exit_code(&report_with(1, 0)), code::ERRORS);
    }

    #[test]
    fn warnings_alone_exit_two_so_ci_can_tell_them_apart() {
        assert_eq!(exit_code(&report_with(0, 3)), code::WARNINGS);
    }

    #[test]
    fn a_warning_budget_does_not_relabel_a_warnings_only_run() {
        // https://github.com/MaximeGaudin/slint/issues/143: the exit code names the class of
        // finding, not how comfortable the budget is — a run with no errors is "warnings only"
        // (2) even when --max-warnings is breached. The budget verdict lives in the JSON
        // envelope's `ok`, and 2 still fails a CI job.
        assert_eq!(exit_code(&report_with(0, 3)), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 3)), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 4)), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 1)), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 0)), code::CLEAN);
    }

    #[test]
    fn fixing_converges_once_a_pass_has_nothing_left_to_fix() {
        // Reproduces https://github.com/MaximeGaudin/slint/issues/91: the round that finds work
        // the first pass deferred must run in the same invocation, not wait for another --fix.
        let mut calls = 0;

        let report = fix_until_converged(
            report_with(1, 0),
            |_| {
                calls += 1;
                let fixes = usize::from(calls <= 2);
                Ok(slint::fix::Applied {
                    files: usize::from(fixes > 0),
                    fixes,
                    deferred: 0,
                })
            },
            |_| Ok(report_with(0, 0)),
        )
        .unwrap();

        assert_eq!(
            calls, 3,
            "two rounds, then one to see there is nothing left"
        );
        assert_eq!(report.fixed, 2);
    }

    #[test]
    fn fixing_gives_up_after_a_bound_instead_of_looping_forever() {
        let mut calls = 0;

        let report = fix_until_converged(
            report_with(1, 0),
            |_| {
                calls += 1;
                Ok(slint::fix::Applied {
                    files: 1,
                    fixes: 1,
                    deferred: 0,
                })
            },
            |_| Ok(report_with(0, 0)),
        )
        .unwrap();

        assert_eq!(calls, MAX_FIX_ROUNDS);
        assert_eq!(report.fixed, MAX_FIX_ROUNDS);
    }

    #[test]
    fn a_run_that_lints_nothing_is_its_own_verdict_not_a_clean_pass() {
        // https://github.com/MaximeGaudin/slint/issues/118
        let report = Report {
            skills: vec![],
            fixed: 0,
            notes: Vec::new(),
        };

        assert_eq!(exit_code(&report), code::NOTHING_LINTED);
    }

    #[test]
    fn the_command_line_parses_the_shapes_the_readme_promises() {
        let cli = Cli::try_parse_from([
            "slint",
            "skills",
            "--fix",
            "--format",
            "json",
            "--rule",
            "name/not-generic=off",
            "--max-warnings",
            "0",
        ])
        .unwrap();

        assert_eq!(cli.paths, vec![PathBuf::from("skills")]);
        assert!(cli.fix);
        assert_eq!(cli.format, Format::Json);
        assert_eq!(cli.overrides, vec!["name/not-generic=off"]);
        assert_eq!(cli.max_warnings, 0);
    }

    #[test]
    fn no_paths_means_here() {
        let cli = Cli::try_parse_from(["slint"]).unwrap();
        assert_eq!(cli.paths, vec![PathBuf::from(".")]);
    }

    #[test]
    fn an_unknown_format_is_refused_with_the_list_of_real_ones() {
        let failure = Cli::try_parse_from(["slint", "--format", "yaml"])
            .unwrap_err()
            .to_string();
        assert!(failure.contains("stylish"));
    }

    /// A throwaway project directory with its own slint.toml.
    ///
    /// The config walk-up starts at the anchor named on the command line, so pointing the anchor
    /// at a fixture keeps resolution inside a tree the test owns: nothing above it on the machine
    /// — in particular not the repository's own apps/cli/slint.toml, which these tests used to
    /// load by accident (issue #89) — can take part. Every test gets its own tree, so the
    /// parallel harness never shares state.
    fn fixture_project(marker: &str) -> (tempfile::TempDir, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("slint.toml"),
            format!(
                "ignore = [\"**/from-fixture/**\"]\n\n\
                 [rules]\n\
                 \"name/not-generic\" = \"off\"\n\n\
                 [llm]\n\
                 provider = \"openai\"\n\
                 model = \"fixture/{marker}\"\n\
                 base_url = \"https://fixture.example/v1/\"\n\
                 api_key_env = \"FIXTURE_API_KEY\"\n\
                 timeout_seconds = 7\n"
            ),
        )
        .unwrap();

        (temporary, root)
    }

    #[test]
    fn config_resolution_uses_the_nearest_config_and_nothing_above_it() {
        // Issue #89: the walk-up used to start at the process CWD, so under cargo test it
        // discovered the repository's own apps/cli/slint.toml. Anchored at the fixture, it
        // stops at the nearest config and never looks above it.
        let (_temporary, root) = fixture_project("nearest");
        let cli = Cli::try_parse_from(["slint", root.to_str().unwrap()]).unwrap();
        let config = resolve_config(&cli).unwrap();

        let expected = root.join("slint.toml");
        assert_eq!(config.source.as_deref(), Some(expected.as_path()));
        assert_eq!(config.llm.model, "fixture/nearest");
    }

    #[test]
    fn no_llm_turns_every_model_rule_off_in_the_resolved_config() {
        let (_temporary, root) = fixture_project("no-llm");
        let cli = Cli::try_parse_from(["slint", "--no-llm", root.to_str().unwrap()]).unwrap();
        let config = resolve_config(&cli).unwrap();

        // The values that reach this test are the fixture's, never the repository's own config.
        let expected = root.join("slint.toml");
        assert_eq!(config.source.as_deref(), Some(expected.as_path()));
        assert_eq!(config.ignore, vec!["**/from-fixture/**".to_string()]);

        for meta in slint::llm::rules::all() {
            assert_eq!(config.rules.get(meta.name), Some(&RuleSetting::Off));
        }
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/41 —
    /// a run with `--no-config` reads no file, whatever the tree above the path holds.
    #[test]
    fn no_config_runs_on_defaults_even_when_a_config_file_is_there() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("slint.toml"),
            "[rules]\n\"name/not-generic\" = \"off\"\n",
        )
        .unwrap();

        let cli = Cli::try_parse_from(["slint", temporary.path().to_str().unwrap(), "--no-config"])
            .unwrap();
        let config = resolve_config(&cli).unwrap();

        assert_eq!(config.source, None);
        assert_eq!(config.rules.get("name/not-generic"), None);
    }

    #[test]
    fn no_config_conflicts_with_an_explicit_config() {
        let failure = Cli::try_parse_from(["slint", "--no-config", "--config", "slint.toml"])
            .unwrap_err()
            .to_string();

        assert!(failure.contains("cannot be used with"), "{failure}");
    }

    #[test]
    fn a_rule_override_from_the_command_line_reaches_the_config() {
        let (_temporary, root) = fixture_project("override");
        let cli = Cli::try_parse_from([
            "slint",
            "--rule",
            "name/not-generic=error",
            root.to_str().unwrap(),
        ])
        .unwrap();
        let config = resolve_config(&cli).unwrap();

        // The fixture turns the rule off; the command line turns it into an error and wins.
        let expected = root.join("slint.toml");
        assert_eq!(config.source.as_deref(), Some(expected.as_path()));
        assert_eq!(
            config.rules.get("name/not-generic"),
            Some(&RuleSetting::On(Severity::Error))
        );
    }

    #[test]
    fn llm_flags_override_the_file_so_an_editor_can_supply_them() {
        let (_temporary, root) = fixture_project("llm-flags");
        let cli = Cli::try_parse_from([
            "slint",
            "--llm",
            "--llm-provider",
            "openrouter",
            "--llm-model",
            "deepseek/flash",
            "--llm-base-url",
            "https://openrouter.ai/api/",
            "--llm-api-key-env",
            "SLINT_EDITOR_API_KEY",
            root.to_str().unwrap(),
        ])
        .unwrap();

        let config = resolve_config(&cli).unwrap();

        assert_eq!(config.llm.provider, slint::config::Provider::Openrouter);
        assert_eq!(config.llm.model, "deepseek/flash");
        assert_eq!(
            config.llm.base_url.as_deref(),
            Some("https://openrouter.ai/api/")
        );
        assert_eq!(
            config.llm.api_key_env.as_deref(),
            Some("SLINT_EDITOR_API_KEY")
        );
        // A field the flags do not mention still comes from the file — proof the fixture, not
        // some ancestor directory's config, was loaded and merged.
        assert_eq!(config.llm.timeout_seconds, 7);
    }

    #[test]
    fn every_subcommand_the_help_lists_can_be_parsed() {
        assert!(matches!(
            Cli::try_parse_from(["slint", "init"]).unwrap().command,
            Some(Command::Init)
        ));
        assert!(matches!(
            Cli::try_parse_from(["slint", "init-plugin"])
                .unwrap()
                .command,
            Some(Command::InitPlugin)
        ));
        assert!(matches!(
            Cli::try_parse_from(["slint", "rules", "--json"])
                .unwrap()
                .command,
            Some(Command::Rules { json: true })
        ));
    }

    #[test]
    fn the_editor_affordances_parse_the_way_an_editor_sends_them() {
        let cli = Cli::try_parse_from([
            "slint",
            "--stdin",
            "--stdin-filename",
            "skills/helper/SKILL.md",
            "--no-ignore",
            "--ignore-path",
            ".gitignore",
            "--verbose",
        ])
        .unwrap();

        assert!(cli.stdin);
        assert_eq!(
            cli.stdin_filename.as_deref(),
            Some("skills/helper/SKILL.md")
        );
        assert!(cli.no_ignore);
        assert_eq!(cli.ignore_path.as_deref(), Some(Path::new(".gitignore")));
        assert!(cli.verbose);
    }

    #[test]
    fn explain_names_a_rule() {
        let cli = Cli::try_parse_from(["slint", "--explain", "body/max-lines"]).unwrap();

        assert_eq!(cli.explain.as_deref(), Some("body/max-lines"));
    }

    #[test]
    fn completions_name_a_shell() {
        assert!(matches!(
            Cli::try_parse_from(["slint", "completions", "bash"])
                .unwrap()
                .command,
            Some(Command::Completions {
                shell: clap_complete::Shell::Bash
            })
        ));
    }
}
