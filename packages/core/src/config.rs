//! Configuration, in the shape ESLint taught everyone to expect.
//!
//! A rule is `off`, or a severity, or a severity and options. Anything not mentioned keeps the
//! severity the rule was written with, so a config file only ever says what someone disagreed with —
//! which is the property that keeps them three lines long instead of three hundred.
//!
//! Both `slint.toml` and `slint.config.json` are read, because a Rust project and a Node project
//! each have a file they already have opinions about.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::diagnostics::Severity;

/// The names a configuration file can have, in the order they are looked for.
pub const CONFIG_NAMES: [&str; 4] = [
    "slint.toml",
    "slint.config.json",
    ".slintrc.json",
    ".slintrc.toml",
];

/// Where the generated config schema is published, for `$schema` and for the docs.
pub const SCHEMA_URL: &str = "https://slint.dev/schemas/slint-config.json";

/// What a config says about one rule.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleSetting {
    Off,
    On(Severity),
    /// A severity and the rule's own options, which each rule reads for itself.
    Tuned(Severity, serde_json::Value),
}

impl RuleSetting {
    pub fn severity(&self) -> Option<Severity> {
        match self {
            RuleSetting::Off => None,
            RuleSetting::On(severity) => Some(*severity),
            RuleSetting::Tuned(severity, _) => Some(*severity),
        }
    }

    pub fn options(&self) -> Option<&serde_json::Value> {
        match self {
            RuleSetting::Tuned(_, options) => Some(options),
            _ => None,
        }
    }
}

/// Prints a setting the way a config file may write it, so `--print-config` shows the shapes
/// someone typed rather than Rust's idea of them.
impl Serialize for RuleSetting {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            RuleSetting::Off => serializer.serialize_str("off"),
            RuleSetting::On(severity) => serializer.serialize_str(severity.as_str()),
            RuleSetting::Tuned(severity, options) => {
                use serde::ser::SerializeSeq;

                let mut pair = serializer.serialize_seq(Some(2))?;
                pair.serialize_element(severity.as_str())?;
                pair.serialize_element(options)?;
                pair.end()
            }
        }
    }
}

/// Which service answers the rules a regular expression cannot.
///
/// One field decides the wire format and one decides the address, which is all the difference
/// between the providers actually amounts to. Anything OpenAI-compatible — OpenRouter, Groq,
/// Ollama, vLLM, LM Studio, a gateway inside a company — is the same variant with a different
/// `base_url`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Nothing is sent anywhere. The static rules still run, and the report says which did not.
    #[default]
    None,
    /// The OpenAI chat completions shape.
    Openai,
    /// The same shape, at OpenRouter's address.
    Openrouter,
    /// The same shape, at Groq's address ([GroqCloud](https://console.groq.com/)).
    Groq,
    /// The same shape, at a local Ollama.
    Ollama,
    /// Google's generative language API, which is its own shape.
    Gemini,
    /// Anthropic's messages API.
    Anthropic,
}

impl Provider {
    /// Parses a provider name the way the CLI and editors spell it.
    pub fn parse(text: &str) -> Result<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Provider::None),
            "openai" => Ok(Provider::Openai),
            "openrouter" => Ok(Provider::Openrouter),
            "groq" => Ok(Provider::Groq),
            "ollama" => Ok(Provider::Ollama),
            "gemini" => Ok(Provider::Gemini),
            "anthropic" => Ok(Provider::Anthropic),
            other => bail!(
                "{other} is not a provider (none, openai, openrouter, groq, ollama, gemini, anthropic)"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: Provider,
    /// Any model id the provider understands. No default is invented: a wrong model id fails
    /// loudly at the provider, and a guessed one fails after being charged for.
    #[serde(default)]
    pub model: String,
    /// The environment variable holding the key, never the key itself. A secret in a config file
    /// is a secret in version control.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Overrides the provider's own address. This is what makes a local model or a gateway work.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Seconds before a request is abandoned.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// Bodies longer than this are truncated before they are sent, and the report says so.
    #[serde(default = "default_max_input")]
    pub max_input_bytes: usize,
}

fn default_timeout() -> u64 {
    90
}

fn default_max_input() -> usize {
    64 * 1024
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            provider: Provider::None,
            model: String::new(),
            api_key_env: None,
            base_url: None,
            timeout_seconds: default_timeout(),
            max_input_bytes: default_max_input(),
        }
    }
}

impl LlmConfig {
    pub fn is_configured(&self) -> bool {
        self.provider != Provider::None && !self.model.is_empty()
    }
}

/// A plugin, as a config refers to one.
///
/// Plugins are files rather than compiled objects: a rule pack is data, and an external plugin is a
/// program slint talks to over a pipe. Neither can crash the linter, and neither needs the person
/// writing it to know any Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PluginRef {
    /// Path to a rule pack (`.toml` or `.json`) or to a `.wasm` plugin, relative to the config file.
    ///
    /// The extension decides which kind it is, so a config never says the same thing twice.
    pub path: String,
}

/// The whole configuration, after merging.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Config {
    /// Where it was loaded from, for messages and for resolving relative paths.
    pub source: Option<PathBuf>,
    pub rules: BTreeMap<String, RuleSetting>,
    pub ignore: Vec<String>,
    pub llm: LlmConfig,
    pub plugins: Vec<PluginRef>,
}

/// The file, before the severities are turned into something typed.
///
/// The `JsonSchema` derive is what `slint schema` prints: one description of the config format,
/// generated from this struct so the schema cannot drift from what slint actually reads.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
struct RawConfig {
    /// Which rules to change, keyed by rule name (`area/thing`).
    #[serde(default)]
    rules: BTreeMap<String, serde_json::Value>,
    /// Glob patterns for directories that should never be linted.
    #[serde(default)]
    ignore: Vec<String>,
    /// Which model answers the rules a regular expression cannot.
    #[serde(default)]
    llm: LlmConfig,
    /// Rule packs and external plugins to load.
    #[serde(default)]
    plugins: Vec<PluginRef>,
}

impl Config {
    /// What a rule should do, given what the config says and what it was written with.
    pub fn severity_for(&self, rule: &str, default: Severity) -> Option<Severity> {
        match self.rules.get(rule) {
            None => Some(default),
            Some(setting) => setting.severity(),
        }
    }

    pub fn options_for(&self, rule: &str) -> Option<&serde_json::Value> {
        self.rules.get(rule).and_then(|setting| setting.options())
    }
}

/// Looks for a config file, starting at `from` and walking up to the root.
///
/// The walk is what makes running slint on a subdirectory of a repository behave the way everyone
/// expects: the settings belong to the project, not to the directory the terminal happens to be in.
/// When no project config exists anywhere up the tree, a user-global one is the fallback, so a
/// personal set of defaults does not have to be repeated in every repository.
pub fn find(from: &Path) -> Option<PathBuf> {
    walk_up(from).or_else(|| {
        let candidate = user_config_path()?;
        candidate.is_file().then_some(candidate)
    })
}

/// The project config search alone: every parent of `from`, nearest first.
fn walk_up(from: &Path) -> Option<PathBuf> {
    let mut directory = Some(from);

    while let Some(current) = directory {
        for name in CONFIG_NAMES {
            let candidate = current.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        directory = current.parent();
    }

    None
}

/// Where the user's own config lives, below any project config.
///
/// `$XDG_CONFIG_HOME/slint/config.toml`, or `~/.config/slint/config.toml` when XDG is unset — or
/// relative, which the spec says to treat as unset. Where there is no home either, `%APPDATA%`
/// stands in, which is the closest thing Windows has to both.
pub fn user_config_path() -> Option<PathBuf> {
    user_config_path_from(std::env::vars_os())
}

/// The same decision, from an explicit set of environment variables.
///
/// Taking the variables instead of reading them here keeps the choice testable without mutating
/// the process environment from inside a parallel test run.
pub fn user_config_path_from<V, K, S>(variables: V) -> Option<PathBuf>
where
    V: IntoIterator<Item = (K, S)>,
    K: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    let mut xdg = None;
    let mut home = None;
    let mut appdata = None;

    for (key, value) in variables {
        match key.as_ref().to_string_lossy().as_ref() {
            "XDG_CONFIG_HOME" => xdg = Some(PathBuf::from(value.as_ref())),
            "HOME" => home = Some(PathBuf::from(value.as_ref())),
            "APPDATA" => appdata = Some(PathBuf::from(value.as_ref())),
            _ => {}
        }
    }

    if let Some(directory) = xdg.filter(|path| path.is_absolute()) {
        return Some(directory.join("slint").join("config.toml"));
    }

    if let Some(home) = home {
        return Some(home.join(".config").join("slint").join("config.toml"));
    }

    appdata.map(|directory| directory.join("slint").join("config.toml"))
}

/// The config file format, as JSON Schema, for editors.
///
/// Generated from [`RawConfig`], so the schema and the reader are the same code. Point a config
/// file's `$schema` at the published copy (`https://slint.dev/schemas/slint-config.json`) and an
/// editor flags a misspelt field before slint ever runs.
pub fn config_json_schema() -> serde_json::Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(RawConfig))
        .expect("the config format has a representable schema");

    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "$id".into(),
            serde_json::Value::String(SCHEMA_URL.to_string()),
        );
        object.insert(
            "title".into(),
            serde_json::Value::String("slint configuration".into()),
        );
        object.insert(
            "description".into(),
            serde_json::Value::String(
                "Configuration for slint, the linter for Agent Skills. Save it as slint.toml or \
                 slint.config.json; rules not mentioned keep the severity they were written with."
                    .into(),
            ),
        );
        object.insert(
            "additionalProperties".into(),
            serde_json::Value::Bool(false),
        );
    }

    schema
}

pub fn load(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let raw: RawConfig = if path.extension().and_then(|one| one.to_str()) == Some("json") {
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    };

    let mut config = Config {
        source: Some(path.to_path_buf()),
        rules: BTreeMap::new(),
        ignore: raw.ignore,
        llm: raw.llm,
        plugins: raw.plugins,
    };

    for (name, value) in raw.rules {
        config
            .rules
            .insert(name.clone(), setting_from(&name, &value)?);
    }

    Ok(config)
}

/// Turns `"warn"`, `"off"` or `["warn", { … }]` into a setting.
fn setting_from(rule: &str, value: &serde_json::Value) -> Result<RuleSetting> {
    match value {
        serde_json::Value::String(text) => {
            if text == "off" {
                return Ok(RuleSetting::Off);
            }

            Severity::parse(text)
                .map(RuleSetting::On)
                .with_context(|| format!("{rule}: {text} is not a severity"))
        }
        serde_json::Value::Number(number) => {
            let level = number.as_i64().unwrap_or(-1);
            match level {
                0 => Ok(RuleSetting::Off),
                1 => Ok(RuleSetting::On(Severity::Warning)),
                2 => Ok(RuleSetting::On(Severity::Error)),
                _ => bail!("{rule}: {level} is not a severity (0, 1 or 2)"),
            }
        }
        serde_json::Value::Array(parts) => {
            let Some(first) = parts.first() else {
                bail!("{rule}: an empty list says nothing");
            };

            let severity = match setting_from(rule, first)? {
                RuleSetting::Off => return Ok(RuleSetting::Off),
                RuleSetting::On(severity) => severity,
                RuleSetting::Tuned(severity, _) => severity,
            };

            match parts.get(1) {
                Some(options) => Ok(RuleSetting::Tuned(severity, options.clone())),
                None => Ok(RuleSetting::On(severity)),
            }
        }
        other => bail!("{rule}: {other} is not a severity"),
    }
}

/// `--rule name=level` from the command line, which overrides the file.
pub fn parse_override(text: &str) -> Result<(String, RuleSetting)> {
    let Some((name, level)) = text.split_once('=') else {
        bail!("expected rule=level, got {text}");
    };

    let setting = if level == "off" {
        RuleSetting::Off
    } else {
        RuleSetting::On(
            Severity::parse(level).with_context(|| format!("{level} is not a severity"))?,
        )
    };

    Ok((name.to_string(), setting))
}

/// The file `slint --init` writes.
pub const STARTER_CONFIG: &str = r#"# slint — the linter for Agent Skills.
#
# Every rule already has a severity; this file is for the ones you disagree with.
# See https://slint.dev/rules for what each one checks and where it comes from.

ignore = ["**/fixtures/**"]

[rules]
# "description/says-when" = "error"
# "body/max-lines" = ["warn", { max = 400 }]
# "bundle/unused-file" = "off"

# The rules a regular expression cannot answer need a model. Nothing is sent anywhere
# until this is filled in, and the report always says which rules did not run.
[llm]
provider = "none"      # none | openai | openrouter | groq | ollama | gemini | anthropic
model = ""             # any model id the provider understands
api_key_env = "OPENAI_API_KEY"   # the variable holding the key, never the key
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_missing_rule_keeps_the_severity_it_was_written_with() {
        let config = Config::default();
        assert_eq!(
            config.severity_for("description/says-when", Severity::Warning),
            Some(Severity::Warning)
        );
    }

    #[test]
    fn off_means_the_rule_does_not_run() {
        let mut config = Config::default();
        config
            .rules
            .insert("body/max-lines".into(), RuleSetting::Off);

        assert_eq!(
            config.severity_for("body/max-lines", Severity::Warning),
            None
        );
    }

    #[test]
    fn a_config_can_raise_a_warning_to_an_error() {
        let mut config = Config::default();
        config.rules.insert(
            "description/says-when".into(),
            RuleSetting::On(Severity::Error),
        );

        assert_eq!(
            config.severity_for("description/says-when", Severity::Warning),
            Some(Severity::Error)
        );
    }

    #[test]
    fn severities_are_read_from_every_shape_a_config_may_use() {
        let cases = [
            ("\"error\"", RuleSetting::On(Severity::Error)),
            ("\"warn\"", RuleSetting::On(Severity::Warning)),
            ("\"off\"", RuleSetting::Off),
            ("0", RuleSetting::Off),
            ("1", RuleSetting::On(Severity::Warning)),
            ("2", RuleSetting::On(Severity::Error)),
            ("[\"warn\"]", RuleSetting::On(Severity::Warning)),
        ];

        for (text, expected) in cases {
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                setting_from("a-rule", &value).unwrap(),
                expected,
                "for {text}"
            );
        }
    }

    #[test]
    fn options_ride_along_with_the_severity() {
        let value: serde_json::Value =
            serde_json::from_str("[\"warn\", { \"max\": 400 }]").unwrap();
        let setting = setting_from("body/max-lines", &value).unwrap();

        assert_eq!(setting.severity(), Some(Severity::Warning));
        assert_eq!(setting.options().unwrap()["max"], serde_json::json!(400));
    }

    #[test]
    fn a_rule_turned_off_with_options_stays_off() {
        let value: serde_json::Value = serde_json::from_str("[\"off\", { \"max\": 400 }]").unwrap();
        assert_eq!(
            setting_from("body/max-lines", &value).unwrap(),
            RuleSetting::Off
        );
    }

    #[test]
    fn a_severity_nobody_defined_is_an_error_rather_than_a_silent_default() {
        let value: serde_json::Value = serde_json::json!("shout");
        assert!(setting_from("a-rule", &value).is_err());

        let numeric: serde_json::Value = serde_json::json!(7);
        assert!(setting_from("a-rule", &numeric).is_err());
    }

    #[test]
    fn toml_and_json_configs_say_the_same_thing() {
        let temporary = tempfile::tempdir().unwrap();

        let toml_path = temporary.path().join("slint.toml");
        fs::write(
            &toml_path,
            "ignore = [\"**/fixtures/**\"]\n\n[rules]\n\"description/says-when\" = \"error\"\n\n[llm]\nprovider = \"openai\"\nmodel = \"gpt-5-mini\"\n",
        )
        .unwrap();

        let json_path = temporary.path().join("slint.config.json");
        fs::write(
            &json_path,
            r#"{ "ignore": ["**/fixtures/**"], "rules": { "description/says-when": "error" }, "llm": { "provider": "openai", "model": "gpt-5-mini" } }"#,
        )
        .unwrap();

        let from_toml = load(&toml_path).unwrap();
        let from_json = load(&json_path).unwrap();

        assert_eq!(from_toml.rules, from_json.rules);
        assert_eq!(from_toml.ignore, from_json.ignore);
        assert_eq!(from_toml.llm, from_json.llm);
        assert!(from_toml.llm.is_configured());
    }

    #[test]
    fn the_config_is_found_by_walking_up_from_where_slint_was_run() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        fs::write(root.join("slint.toml"), "[rules]\n").unwrap();

        let deep = root
            .join("skills")
            .join("photography")
            .join("photo-culling");
        fs::create_dir_all(&deep).unwrap();

        assert_eq!(find(&deep), Some(root.join("slint.toml")));
    }

    #[test]
    fn no_config_anywhere_is_a_normal_state() {
        let temporary = tempfile::tempdir().unwrap();
        // A parent of the temporary directory could hold a config in a developer's home, so this
        // only asserts the walk terminates rather than that it finds nothing.
        let _ = find(temporary.path());
    }

    #[test]
    fn a_command_line_override_parses_into_a_setting() {
        let (name, setting) = parse_override("description/says-when=error").unwrap();

        assert_eq!(name, "description/says-when");
        assert_eq!(setting, RuleSetting::On(Severity::Error));

        let (_, off) = parse_override("body/max-lines=off").unwrap();
        assert_eq!(off, RuleSetting::Off);

        assert!(parse_override("nonsense").is_err());
        assert!(parse_override("a-rule=shout").is_err());
    }

    #[test]
    fn an_llm_block_is_only_configured_once_it_names_a_model() {
        let mut llm = LlmConfig::default();
        assert!(!llm.is_configured());

        llm.provider = Provider::Openai;
        assert!(!llm.is_configured());

        llm.model = "gpt-5-mini".into();
        assert!(llm.is_configured());
    }

    #[test]
    fn the_user_config_lives_under_an_absolute_xdg_config_home() {
        let variables = [
            (
                "XDG_CONFIG_HOME".to_string(),
                "/userdata/config".to_string(),
            ),
            ("HOME".to_string(), "/userdata/home".to_string()),
        ];

        assert_eq!(
            user_config_path_from(variables),
            Some(PathBuf::from("/userdata/config/slint/config.toml"))
        );
    }

    #[test]
    fn without_xdg_the_user_config_is_under_the_home() {
        let variables = [("HOME".to_string(), "/userdata/home".to_string())];

        assert_eq!(
            user_config_path_from(variables),
            Some(PathBuf::from("/userdata/home/.config/slint/config.toml"))
        );
    }

    #[test]
    fn a_relative_xdg_config_home_is_ignored_as_the_spec_demands() {
        let variables = [
            ("XDG_CONFIG_HOME".to_string(), "relative/config".to_string()),
            ("HOME".to_string(), "/userdata/home".to_string()),
        ];

        assert_eq!(
            user_config_path_from(variables),
            Some(PathBuf::from("/userdata/home/.config/slint/config.toml"))
        );
    }

    #[test]
    fn the_user_config_falls_back_to_appdata_where_there_is_no_home() {
        let variables = [(
            "APPDATA".to_string(),
            "C:\\Users\\someone\\AppData\\Roaming".to_string(),
        )];

        assert_eq!(
            user_config_path_from(variables),
            Some(
                PathBuf::from("C:\\Users\\someone\\AppData\\Roaming")
                    .join("slint")
                    .join("config.toml")
            )
        );
    }

    #[test]
    fn with_nothing_set_there_is_no_user_config() {
        let variables: [(String, String); 0] = [];
        assert_eq!(user_config_path_from(variables), None);
    }

    #[test]
    fn a_provider_name_parses_the_way_the_cli_and_editors_spell_it() {
        assert_eq!(Provider::parse("openrouter").unwrap(), Provider::Openrouter);
        assert_eq!(Provider::parse("groq").unwrap(), Provider::Groq);
        assert_eq!(Provider::parse("OpenAI").unwrap(), Provider::Openai);
        assert!(Provider::parse("claude").is_err());
    }

    #[test]
    fn the_starter_config_is_valid_and_turns_nothing_on() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.toml");
        fs::write(&path, STARTER_CONFIG).unwrap();

        let config = load(&path).unwrap();

        assert!(config.rules.is_empty());
        assert_eq!(config.llm.provider, Provider::None);
        assert!(!config.llm.is_configured());
    }
}
