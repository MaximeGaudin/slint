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
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::diagnostics::Severity;
use crate::plugin::Plugin;
use crate::rules;

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
#[serde(deny_unknown_fields)]
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
    /// Cap on the tokens the model may spend on its reply. None leaves the cap to the provider.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Transport-level failures (rate limits, 5xx, connection errors) are retried this many times
    /// with backoff before the failure becomes what the run does about it.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// How many model requests may be in flight at once. One per skill is how a large repository
    /// meets a provider's rate limit in the first second.
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: usize,
    /// Bodies longer than this are truncated before they are sent, and the report says so.
    #[serde(default = "default_max_input")]
    pub max_input_bytes: usize,
    /// Not a setting: the shape every other tool's config uses, read only so `load` can refuse
    /// it with the message it deserves instead of letting the secret sit in the file.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) api_key: Option<String>,
}

fn default_timeout() -> u64 {
    90
}

fn default_max_retries() -> u32 {
    2
}

fn default_max_concurrent_requests() -> usize {
    4
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
            max_tokens: None,
            max_retries: default_max_retries(),
            max_concurrent_requests: default_max_concurrent_requests(),
            max_input_bytes: default_max_input(),
            api_key: None,
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
///
/// `deny_unknown_fields` is the point: a section name off by one used to load as if the file were
/// empty, so the whole config silently did nothing and the run went on as if none had been read.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
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

/// Every recognised config file in `directory`, in the order `find` would take them.
///
/// A directory holding more than one of them is where "I edited the wrong file" lives, so the
/// list is what the warning names when it has something to say.
pub fn config_files_in(directory: &Path) -> Vec<PathBuf> {
    CONFIG_NAMES
        .iter()
        .map(|name| directory.join(name))
        .filter(|candidate| candidate.is_file())
        .collect()
}

/// Looks for a config file, starting at `from` and walking up to the root.
///
/// The walk is what makes running slint on a subdirectory of a repository behave the way everyone
/// expects: the settings belong to the project, not to the directory the terminal happens to be in.
///
/// When no project config exists anywhere up the tree, a user-global one is the fallback, so a
/// personal set of defaults does not have to be repeated in every repository.
///
/// When a directory holds several config files, the first of `CONFIG_NAMES` wins; the rest are
/// ignored, which is why the README states the order and the run says so when it sees one.
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
        if let Some(first) = config_files_in(current).into_iter().next() {
            return Some(first);
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

    // XDG says an absolute path or nothing. `has_root` is that test, spelled portably: on Unix it
    // is exactly "starts with /", and on Windows it also accepts a rooted path with no drive, so
    // an XDG variable pointing at `/slint-config` still works there.
    if let Some(directory) = xdg.filter(|path| path.has_root()) {
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

    if raw.llm.api_key.is_some() {
        bail!(
            "{}: [llm] api_key is not a setting — slint never takes a literal key in a config file, because a key in a file is a key in version control. Use api_key_env to name the variable that holds the key.",
            path.display()
        );
    }

    let mut config = Config {
        source: Some(path.to_path_buf()),
        rules: BTreeMap::new(),
        ignore: raw.ignore,
        llm: raw.llm,
        plugins: raw.plugins,
    };

    for (name, value) in raw.rules {
        let setting = setting_from(&name, &value)?;

        // A rule's options are read by the rule itself, so a value it cannot read would be
        // swallowed into its default: refuse the file instead.
        if let RuleSetting::Tuned(_, options) = &setting {
            crate::rules::validate_rule_options(&name, options)?;
        }

        config.rules.insert(name, setting);
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

/// Refuses a config that sets a rule that does not exist.
///
/// A setting for a rule that does not exist is never consulted — the lookup is by exact name —
/// so without this check the key is a no-op in the file the user edited, with zero diagnostic.
/// Refusing the run is the same way an unknown severity or provider is already refused in this
/// file.
///
/// The check stands down when it cannot see every rule the config may point at: a Wasm plugin
/// declares its rules by reporting them, and a plugin set that was not loaded (`--no-plugins`)
/// still leaves its rule names in the config.
pub fn check_rule_names(config: &Config, plugins: &[Plugin]) -> Result<()> {
    let wasm_loaded = plugins
        .iter()
        .any(|plugin| matches!(plugin, Plugin::Wasm { .. }));
    let plugins_skipped = !config.plugins.is_empty() && plugins.is_empty();

    if wasm_loaded || plugins_skipped {
        return Ok(());
    }

    let mut known: BTreeSet<String> = rules::all_meta()
        .iter()
        .map(|meta| meta.name.to_string())
        .collect();

    for plugin in plugins {
        known.extend(plugin.rule_names());
    }

    for name in config.rules.keys() {
        if !known.contains(name) {
            bail!(
                "{name}: no such rule. The setting would do nothing — check the name with `slint rules`."
            );
        }
    }

    Ok(())
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

    /// Regression for https://github.com/MaximeGaudin/slint/issues/42 —
    /// a directory holding two config files lists them in the order the winner is picked, so a
    /// message can say which file read the others out.
    #[test]
    fn two_config_files_in_one_directory_list_in_precedence_order() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        fs::write(root.join("slint.config.json"), "{}\n").unwrap();
        fs::write(root.join("slint.toml"), "[rules]\n").unwrap();

        let files = config_files_in(root);
        assert_eq!(
            files,
            vec![root.join("slint.toml"), root.join("slint.config.json")]
        );

        // And `find` still takes the first of the two.
        assert_eq!(find(root), Some(root.join("slint.toml")));
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
    fn token_concurrency_and_retry_defaults_are_conservative() {
        let llm = LlmConfig::default();

        assert_eq!(llm.max_tokens, None, "no reply cap invented silently");
        assert_eq!(llm.max_retries, 2);
        assert_eq!(llm.max_concurrent_requests, 4);
    }

    #[test]
    fn the_llm_knobs_are_settable_from_the_config_file() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.toml");
        fs::write(
            &path,
            "[llm]\nprovider = \"openai\"\nmodel = \"gpt-5-mini\"\nmax_tokens = 512\nmax_retries = 1\nmax_concurrent_requests = 8\n",
        )
        .unwrap();

        let config = load(&path).unwrap();

        assert_eq!(config.llm.max_tokens, Some(512));
        assert_eq!(config.llm.max_retries, 1);
        assert_eq!(config.llm.max_concurrent_requests, 8);
    }

    #[test]
    fn the_user_config_lives_under_a_rooted_xdg_config_home() {
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

    /// Regression for https://github.com/MaximeGaudin/slint/issues/38 —
    /// a literal key pasted where api_key_env belongs is the shape every other tool's config
    /// uses, and it used to be accepted and ignored while the secret sat in the file.
    #[test]
    fn a_literal_api_key_in_the_llm_block_is_refused_and_named() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.toml");
        fs::write(
            &path,
            "[llm]\nprovider = \"openai\"\nmodel = \"gpt-5-mini\"\napi_key = \"sk-test-0000\"\n",
        )
        .unwrap();

        let failure = load(&path).unwrap_err();
        let failure = format!("{failure:#}");

        assert!(failure.contains("api_key"), "{failure}");
        assert!(failure.contains("api_key_env"), "{failure}");
    }

    #[test]
    fn an_unknown_field_in_the_llm_block_is_refused_rather_than_silently_dropped() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.config.json");
        fs::write(
            &path,
            r#"{ "llm": { "provider": "openai", "model": "gpt-5-mini", "api_key_environment": "OPENAI_API_KEY" } }"#,
        )
        .unwrap();

        let failure = load(&path).unwrap_err();
        let failure = format!("{failure:#}");

        assert!(failure.contains("api_key_environment"), "{failure}");
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/27 —
    /// an out-of-range rule option used to be swallowed into the rule's default, so the body
    /// was judged against 500 lines with no sign that `max = -5` was never read.
    #[test]
    fn an_out_of_range_rule_option_fails_the_load_and_names_the_rule() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.toml");
        fs::write(
            &path,
            "[rules]\n\"body/max-lines\" = [\"warn\", { max = -5 }]\n",
        )
        .unwrap();

        let failure = load(&path).unwrap_err();
        let failure = format!("{failure:#}");

        assert!(failure.contains("body/max-lines"), "{failure}");
        assert!(failure.contains("-5"), "{failure}");
    }

    /// The same silent drop happened to a misspelt option key: the rule never read it.
    #[test]
    fn an_option_key_a_rule_does_not_read_fails_the_load() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.config.json");
        fs::write(
            &path,
            r#"{ "rules": { "description/min-length": ["warn", { "minumum": 100 }] } }"#,
        )
        .unwrap();

        let failure = load(&path).unwrap_err();
        let failure = format!("{failure:#}");

        assert!(failure.contains("description/min-length"), "{failure}");
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

    #[test]
    fn a_rule_name_that_no_loaded_rule_answers_for_is_refused() {
        let mut config = Config::default();
        config
            .rules
            .insert("name/not-genric".into(), RuleSetting::Off);

        let failure = check_rule_names(&config, &[]).unwrap_err().to_string();

        assert!(failure.contains("name/not-genric"), "{failure}");
        assert!(failure.contains("slint rules"), "{failure}");
    }

    #[test]
    fn rule_names_the_catalogue_and_a_loaded_pack_define_are_accepted() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pack.toml");
        fs::write(
            &path,
            "[[rules]]\nname = \"house/no-todo\"\nseverity = \"warning\"\nsummary = \"No TODO markers.\"\nrationale = \"A TODO reads as an instruction that was not followed.\"\nadvice = \"Finish the step.\"\npattern = \"TODO\"\ntarget = \"body\"\nreference = { title = \"House style\", url = \"https://example.com/style\" }\n",
        )
        .unwrap();

        let plugin = crate::plugin::load(
            &PluginRef {
                path: "pack.toml".into(),
            },
            temporary.path(),
        )
        .unwrap();

        let mut config = Config::default();
        config
            .rules
            .insert("body/posix-paths".into(), RuleSetting::Off);
        config
            .rules
            .insert("llm/consistent-terminology".into(), RuleSetting::Off);
        config
            .rules
            .insert("house/no-todo".into(), RuleSetting::Off);

        assert!(check_rule_names(&config, &[plugin]).is_ok());
    }

    #[test]
    fn a_name_a_wasm_plugin_might_provide_is_not_refused() {
        let plugin = crate::plugin::Plugin::Wasm {
            path: std::path::PathBuf::from("plugin.wasm"),
        };

        let mut config = Config::default();
        config.rules.insert("who/knows".into(), RuleSetting::Off);

        assert!(check_rule_names(&config, &[plugin]).is_ok());
    }

    #[test]
    fn a_pack_rule_name_is_not_refused_when_the_plugins_were_skipped() {
        let mut config = Config::default();
        config.plugins.push(PluginRef {
            path: "house.toml".into(),
        });
        config
            .rules
            .insert("house/no-todo".into(), RuleSetting::Off);

        assert!(check_rule_names(&config, &[]).is_ok());
    }

    /// Regression for https://github.com/MaximeGaudin/slint/issues/26 —
    /// a section name off by one used to load as if the file were empty, so the whole config
    /// silently did nothing.
    #[test]
    fn a_mis_spelled_top_level_section_is_refused_rather_than_silently_dropped() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("slint.toml");
        fs::write(&path, "[rule]\n\"name/not-generic\" = \"off\"\n").unwrap();

        let failure = load(&path).unwrap_err();
        let failure = format!("{failure:#}");

        assert!(failure.contains("unknown field `rule`"), "{failure}");
        assert!(failure.contains("rules"), "{failure}");
    }
}
