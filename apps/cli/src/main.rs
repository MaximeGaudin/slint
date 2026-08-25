//! The command line.
//!
//! Exit codes follow the convention the other skill linters settled on, so a CI script written for
//! one works here: 0 clean, 1 errors, 2 warnings only, 3 slint itself failed. Data goes to stdout
//! and everything else to stderr, so `slint --format json | jq` is always safe.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use slint::config::{self, Config, RuleSetting};
use slint::diagnostics::Report;
use slint::engine::{self, Passes};
use slint::plugin;
use slint::report::{self, Format};
use slint::rules;

/// Exit codes, named.
mod code {
    pub const CLEAN: u8 = 0;
    pub const ERRORS: u8 = 1;
    pub const WARNINGS: u8 = 2;
    pub const FAILED: u8 = 3;
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

    /// Never colour the output. Colour is off automatically when stdout is not a terminal.
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
    /// Write a starter config in the current directory.
    Init,
    /// Write a starter rule pack, to be edited and pointed at from the config.
    InitPlugin,
    /// Print the rule catalogue: what each rule checks, and where the claim comes from.
    Rules {
        /// As JSON, which is what the documentation site is built from.
        #[arg(long)]
        json: bool,
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
        None => {}
    }

    let config = resolve_config(cli)?;

    let plugins = if cli.no_plugins {
        Vec::new()
    } else {
        plugin::load_all(&config)?
    };

    let passes = Passes {
        plugins: !cli.no_plugins,
        model: cli.model_pass(),
    };

    let mut report = engine::run(&cli.paths, &config, &plugins, passes)?;

    if cli.fix {
        let applied = slint::fix::apply(&report)?;

        if applied.fixes > 0 {
            // Lint again so the report describes the files as they are now rather than as they
            // were: half of what --fix does is proving the fix worked.
            report = engine::run(&cli.paths, &config, &plugins, passes)?;
            report.fixed = applied.fixes;
        }
    }

    if cli.quiet {
        for skill in &mut report.skills {
            skill
                .messages
                .retain(|message| message.severity == slint::Severity::Error);
        }
    }

    let colour = !cli.no_color && std::io::stdout().is_terminal();
    let text = report::render(&report, cli.format, colour);

    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}").context("writing the report")?;

    Ok(exit_code(&report, cli.max_warnings))
}

/// What the run means, as a number.
fn exit_code(report: &Report, max_warnings: i64) -> u8 {
    if report.errors() > 0 {
        return code::ERRORS;
    }

    if max_warnings >= 0 && report.warnings() as i64 > max_warnings {
        return code::ERRORS;
    }

    if report.warnings() > 0 {
        return code::WARNINGS;
    }

    code::CLEAN
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
        eprintln!(
            "slint: {} already exists, so nothing was written",
            path.display()
        );
        return Ok(code::FAILED);
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
        eprintln!(
            "slint: {} already exists, so nothing was written",
            path.display()
        );
        return Ok(code::FAILED);
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

    Ok(code::CLEAN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::diagnostics::{Location, Message, Reference, Severity, SkillReport, Source};
    use std::fs;

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
        }
    }

    #[test]
    fn a_clean_run_exits_zero() {
        assert_eq!(exit_code(&report_with(0, 0), -1), code::CLEAN);
    }

    #[test]
    fn errors_exit_one() {
        assert_eq!(exit_code(&report_with(1, 0), -1), code::ERRORS);
    }

    #[test]
    fn warnings_alone_exit_two_so_ci_can_tell_them_apart() {
        assert_eq!(exit_code(&report_with(0, 3), -1), code::WARNINGS);
    }

    #[test]
    fn a_warning_budget_turns_warnings_into_a_failure_once_it_is_passed() {
        assert_eq!(exit_code(&report_with(0, 3), 5), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 3), 3), code::WARNINGS);
        assert_eq!(exit_code(&report_with(0, 4), 3), code::ERRORS);
        assert_eq!(exit_code(&report_with(0, 1), 0), code::ERRORS);
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

    #[test]
    fn no_llm_turns_every_model_rule_off_in_the_resolved_config() {
        let cli = Cli::try_parse_from(["slint", "--no-llm"]).unwrap();
        let config = resolve_config(&cli).unwrap();

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

        let cli =
            Cli::try_parse_from(["slint", temporary.path().to_str().unwrap(), "--no-config"])
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
        let cli = Cli::try_parse_from(["slint", "--rule", "name/not-generic=error"]).unwrap();
        let config = resolve_config(&cli).unwrap();

        assert_eq!(
            config.rules.get("name/not-generic"),
            Some(&RuleSetting::On(Severity::Error))
        );
    }

    #[test]
    fn llm_flags_override_the_file_so_an_editor_can_supply_them() {
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
}
