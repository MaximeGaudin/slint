//! Talking to a model, whichever one it is.
//!
//! The wire formats are not ours to maintain. [`genai`] already speaks OpenAI, Anthropic, Gemini,
//! Ollama, OpenRouter and Groq natively — including the parts that differ in ways a thin wrapper always
//! gets wrong eventually — so a provider here is a name, a model id and, optionally, an address.
//!
//! What is ours is the seam: [`Chat`] is one method, which is what lets the review pass be tested
//! against a fake and lets a future provider arrive without touching a rule.

use anyhow::{Context, Result, bail};
use genai::chat::{ChatOptions, ChatRequest, ChatResponseFormat, JsonSpec, Tool, ToolChoice};
use genai::resolver::{AuthData, AuthResolver, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use serde::Serialize;
use serde_json::json;

use crate::config::{LlmConfig, Provider};

/// What is asked of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

/// How findings are forced from the provider (schema → tool → json_object).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingsFormat {
    /// `ChatResponseFormat::JsonSpec` / provider `json_schema`.
    JsonSchema,
    /// Forced `report_findings` tool/function call.
    Tool,
    /// `ChatResponseFormat::JsonMode` (`json_object`) plus robust text parse.
    JsonMode,
}

/// Preferred findings format for a configured provider (genai adapter capabilities).
pub fn findings_format_for(provider: Provider) -> FindingsFormat {
    match provider {
        // Native JsonSpec wiring in genai.
        Provider::Openai | Provider::Anthropic | Provider::Gemini => FindingsFormat::JsonSchema,
        // OpenAI-compatible: json_schema support varies by model; forced tool is more reliable.
        Provider::Groq | Provider::Openrouter => FindingsFormat::Tool,
        // Ollama adapter only maps JsonMode today.
        Provider::Ollama | Provider::None => FindingsFormat::JsonMode,
    }
}

/// JSON Schema for `{"findings":[...]}` — object root required by json_schema APIs.
pub fn findings_json_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "rule": { "type": "string" },
                        "message": { "type": "string" },
                        "line": { "type": ["integer", "null"] },
                        "confidence": { "type": ["number", "null"] }
                    },
                    // Strict json_schema (OpenAI) requires every property to be listed here.
                    "required": ["rule", "message", "line", "confidence"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["findings"],
        "additionalProperties": false
    })
}

const REPORT_FINDINGS_TOOL: &str = "report_findings";

/// Anything that can answer a prompt.
///
/// A trait rather than a function so the review pass can be tested without a network, which is most
/// of what makes this half of the tool testable at all.
pub trait Chat {
    fn complete(&self, prompt: &Prompt) -> Result<String>;
    /// Ask for findings using `format`; default delegates to [`Chat::complete`].
    fn complete_findings(&self, prompt: &Prompt, format: FindingsFormat) -> Result<String> {
        let _ = format;
        self.complete(prompt)
    }
    /// What the report says the review was produced with.
    fn describe(&self) -> String;
}

/// The adapter namespace genai knows a provider by.
pub fn adapter_name(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Openai => Some("openai"),
        // genai's namespace is `open_router` (underscore). `openrouter` is not recognized and
        // falls through to the Ollama default, which is exactly the wrong place to send a paid key.
        Provider::Openrouter => Some("open_router"),
        Provider::Groq => Some("groq"),
        Provider::Ollama => Some("ollama"),
        Provider::Gemini => Some("gemini"),
        Provider::Anthropic => Some("anthropic"),
        Provider::None => None,
    }
}

/// The model string genai resolves an adapter from.
///
/// A model id already carrying a namespace is left alone: someone who wrote `openai::gpt-5-mini`
/// has been more specific than the `provider` field, and overruling them would be surprising.
pub fn model_spec(config: &LlmConfig) -> Result<String> {
    if config.model.trim().is_empty() {
        bail!("no model is configured");
    }

    if config.model.contains("::") {
        return Ok(config.model.clone());
    }

    let adapter = adapter_name(config.provider).context("no provider is configured")?;

    Ok(format!("{adapter}::{}", config.model))
}

/// A model reached through genai.
pub struct GenAiChat {
    client: Client,
    model: String,
    provider: Provider,
    runtime: tokio::runtime::Runtime,
}

impl GenAiChat {
    /// Builds a client, or explains why it cannot.
    ///
    /// A key that is named but not set is an error rather than a silent skip: a review that quietly
    /// did not run reads exactly like a review that found nothing, which is the failure this whole
    /// tool exists to avoid.
    pub fn new(config: &LlmConfig) -> Result<Self> {
        let model = model_spec(config)?;

        if let Some(variable) = &config.api_key_env {
            std::env::var(variable)
                .with_context(|| format!("{variable} is named in the config and is not set"))?;
        } else if !matches!(config.provider, Provider::Ollama) {
            // A local Ollama needs no key; everything else does, and genai's own environment
            // defaults are only reached when the config does not name a variable.
            bail!(
                "no API key: name the environment variable holding it as api_key_env, or use a local provider"
            );
        }

        let client = build_client(config)?;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("starting the runtime the provider client needs")?;

        Ok(GenAiChat {
            client,
            model,
            provider: config.provider,
            runtime,
        })
    }
}

fn build_client(config: &LlmConfig) -> Result<Client> {
    let mut builder = Client::builder();

    if let Some(variable) = config.api_key_env.clone() {
        let resolver = AuthResolver::from_resolver_fn(
            move |_: ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> {
                Ok(Some(AuthData::from_env(&variable)))
            },
        );

        builder = builder.with_auth_resolver(resolver);
    }

    // An address of our own is what makes a gateway, a proxy or a self-hosted server work, and it
    // is the one thing a provider list can never enumerate.
    if let Some(base) = config.base_url.clone() {
        let address = if base.ends_with('/') {
            base
        } else {
            format!("{base}/")
        };

        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                let ServiceTarget { model, auth, .. } = target;
                Ok(ServiceTarget {
                    endpoint: Endpoint::from_owned(address.clone()),
                    auth,
                    model,
                })
            },
        );

        builder = builder.with_service_target_resolver(resolver);
    }

    Ok(builder.build())
}

fn base_request(prompt: &Prompt) -> ChatRequest {
    ChatRequest::default()
        .with_system(prompt.system.clone())
        .append_message(genai::chat::ChatMessage::user(prompt.user.clone()))
}

fn extract_findings_body(response: genai::chat::ChatResponse) -> Result<String> {
    // Forced tool path: arguments are already the findings object.
    if let Some(call) = response.tool_calls().into_iter().next() {
        return serde_json::to_string(&call.fn_arguments)
            .context("serializing report_findings tool arguments");
    }

    response
        .first_text()
        .map(|text| text.to_string())
        .context("the provider answered with no text and no tool call")
}

impl Chat for GenAiChat {
    fn complete(&self, prompt: &Prompt) -> Result<String> {
        self.complete_findings(prompt, findings_format_for(self.provider))
    }

    fn complete_findings(&self, prompt: &Prompt, format: FindingsFormat) -> Result<String> {
        // Temperature zero: a linter that reports different findings on the same text twice is a
        // linter nobody can put in CI.
        let (request, options) = match format {
            FindingsFormat::JsonSchema => {
                let request = base_request(prompt);
                let options = ChatOptions::default()
                    .with_temperature(0.0)
                    .with_response_format(ChatResponseFormat::JsonSpec(JsonSpec::new(
                        "skill_findings",
                        findings_json_schema(),
                    )));
                (request, options)
            }
            FindingsFormat::Tool => {
                let tool = Tool::new(REPORT_FINDINGS_TOOL)
                    .with_description(
                        "Report skill review findings. Call exactly once with the findings array \
                         (empty when the skill is clean).",
                    )
                    .with_schema(findings_json_schema())
                    .with_strict(true);
                let request = base_request(prompt).with_tools(vec![tool]);
                let options = ChatOptions::default()
                    .with_temperature(0.0)
                    .with_tool_choice(ToolChoice::tool(REPORT_FINDINGS_TOOL));
                (request, options)
            }
            FindingsFormat::JsonMode => {
                let request = base_request(prompt);
                let options = ChatOptions::default()
                    .with_temperature(0.0)
                    .with_response_format(ChatResponseFormat::JsonMode);
                (request, options)
            }
        };

        let response = self
            .runtime
            .block_on(self.client.exec_chat(&self.model, request, Some(&options)))
            .with_context(|| format!("asking {}", self.model))?;

        extract_findings_body(response)
    }

    fn describe(&self) -> String {
        self.model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(provider: Provider, model: &str) -> LlmConfig {
        LlmConfig {
            provider,
            model: model.into(),
            api_key_env: Some("SLINT_TEST_KEY".into()),
            base_url: None,
            timeout_seconds: 5,
            max_input_bytes: 1024,
        }
    }

    #[test]
    fn every_provider_maps_to_an_adapter_genai_knows() {
        assert_eq!(adapter_name(Provider::Openai), Some("openai"));
        assert_eq!(adapter_name(Provider::Openrouter), Some("open_router"));
        assert_eq!(adapter_name(Provider::Groq), Some("groq"));
        assert_eq!(adapter_name(Provider::Ollama), Some("ollama"));
        assert_eq!(adapter_name(Provider::Gemini), Some("gemini"));
        assert_eq!(adapter_name(Provider::Anthropic), Some("anthropic"));
        assert_eq!(adapter_name(Provider::None), None);
    }

    #[test]
    fn findings_format_ladder_prefers_schema_then_tool_then_json_mode() {
        // Adapters with first-class JsonSpec support.
        assert_eq!(
            findings_format_for(Provider::Openai),
            FindingsFormat::JsonSchema
        );
        assert_eq!(
            findings_format_for(Provider::Anthropic),
            FindingsFormat::JsonSchema
        );
        assert_eq!(
            findings_format_for(Provider::Gemini),
            FindingsFormat::JsonSchema
        );
        // OpenAI-compatible hosts where json_schema is uneven; force a tool instead.
        assert_eq!(findings_format_for(Provider::Groq), FindingsFormat::Tool);
        assert_eq!(
            findings_format_for(Provider::Openrouter),
            FindingsFormat::Tool
        );
        // Ollama adapter only wires JsonMode today.
        assert_eq!(
            findings_format_for(Provider::Ollama),
            FindingsFormat::JsonMode
        );
    }

    #[test]
    fn findings_json_schema_is_an_object_with_a_findings_array() {
        let schema = findings_json_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["findings"]["type"], "array");
        assert!(
            schema["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|value| value == "findings"))
        );
    }

    #[test]
    fn a_model_is_namespaced_by_the_configured_provider() {
        assert_eq!(
            model_spec(&config(Provider::Openai, "gpt-5-mini")).unwrap(),
            "openai::gpt-5-mini"
        );
        assert_eq!(
            model_spec(&config(Provider::Openrouter, "deepseek/flash")).unwrap(),
            "open_router::deepseek/flash"
        );
        assert_eq!(
            model_spec(&config(Provider::Ollama, "llama3.2")).unwrap(),
            "ollama::llama3.2"
        );
    }

    #[test]
    fn a_model_that_already_names_its_adapter_is_left_alone() {
        assert_eq!(
            model_spec(&config(Provider::Openai, "anthropic::claude-haiku-4-5")).unwrap(),
            "anthropic::claude-haiku-4-5"
        );
    }

    #[test]
    fn a_config_with_no_model_or_no_provider_cannot_produce_one() {
        assert!(model_spec(&config(Provider::Openai, "")).is_err());
        assert!(model_spec(&config(Provider::None, "gpt-5-mini")).is_err());
    }

    #[test]
    fn a_client_refuses_to_exist_when_the_key_it_was_told_about_is_missing() {
        let mut missing = config(Provider::Openai, "gpt-5-mini");
        missing.api_key_env = Some("SLINT_TEST_MISSING_KEY".into());

        let failure = match GenAiChat::new(&missing) {
            Err(failure) => failure.to_string(),
            Ok(_) => panic!("a key that is named and missing has to be an error"),
        };
        assert!(failure.contains("SLINT_TEST_MISSING_KEY"));
    }

    #[test]
    fn a_hosted_provider_with_no_key_named_at_all_is_refused() {
        let mut anonymous = config(Provider::Openai, "gpt-5-mini");
        anonymous.api_key_env = None;

        assert!(GenAiChat::new(&anonymous).is_err());
    }

    #[test]
    fn a_local_provider_needs_no_key() {
        let mut ollama = config(Provider::Ollama, "llama3.2");
        ollama.api_key_env = None;

        let client = GenAiChat::new(&ollama).expect("a local model needs no credential");
        assert_eq!(client.describe(), "ollama::llama3.2");
    }

    #[test]
    fn a_base_url_builds_a_client_rather_than_being_ignored() {
        let mut gateway = config(Provider::Ollama, "llama3.2");
        gateway.api_key_env = None;
        gateway.base_url = Some("http://gateway.internal/v1".into());

        assert!(GenAiChat::new(&gateway).is_ok());
    }
}
