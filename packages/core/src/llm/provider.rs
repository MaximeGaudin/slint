//! Talking to a model, whichever one it is.
//!
//! The wire formats are not ours to maintain. [`genai`] already speaks OpenAI, Anthropic, Gemini,
//! Ollama, OpenRouter and Groq natively — including the parts that differ in ways a thin wrapper always
//! gets wrong eventually — so a provider here is a name, a model id and, optionally, an address.
//!
//! What is ours is the seam: [`Chat`] is one method, which is what lets the review pass be tested
//! against a fake and lets a future provider arrive without touching a rule.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use genai::chat::{ChatOptions, ChatRequest, ChatResponseFormat, JsonSpec, Tool, ToolChoice};
use genai::resolver::{AuthData, AuthResolver, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget, WebConfig};
use serde::Serialize;
use serde_json::json;
use tokio::sync::Semaphore;

use crate::config::{LlmConfig, Provider};
use crate::llm::retry;

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
///
/// One instance holds one HTTP client and one runtime, and is safe to share across the threads
/// that review skills in parallel: build it once, hand out references, and let [`Self::new`]'s
/// limiter decide how many requests may be in flight at once.
pub struct GenAiChat {
    client: Client,
    model: String,
    provider: Provider,
    runtime: tokio::runtime::Runtime,
    concurrency: Arc<Semaphore>,
    max_tokens: Option<u32>,
    max_retries: u32,
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

        // Multi-threaded, because the skill review runs on many rayon threads at once and each of
        // them parks here waiting for its answer.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("starting the runtime the provider client needs")?;

        Ok(GenAiChat {
            client,
            model,
            provider: config.provider,
            runtime,
            concurrency: Arc::new(Semaphore::new(config.max_concurrent_requests.max(1))),
            max_tokens: config.max_tokens,
            max_retries: config.max_retries,
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

    // genai's own web config leaves every timeout unset, which means a provider that accepts the
    // connection and then says nothing holds a lint run — and an editor save hook — forever.
    builder = builder.with_web_config(
        WebConfig::default().with_timeout(Duration::from_secs(config.timeout_seconds.max(1))),
    );

    Ok(builder.build())
}

fn base_request(prompt: &Prompt) -> ChatRequest {
    ChatRequest::default()
        .with_system(prompt.system.clone())
        .append_message(genai::chat::ChatMessage::user(prompt.user.clone()))
}

/// What is sent, per format. Built per attempt: a retry needs a fresh request.
fn build_request(prompt: &Prompt, format: FindingsFormat) -> ChatRequest {
    match format {
        FindingsFormat::Tool => {
            let tool = Tool::new(REPORT_FINDINGS_TOOL)
                .with_description(
                    "Report skill review findings. Call exactly once with the findings array \
                     (empty when the skill is clean).",
                )
                .with_schema(findings_json_schema())
                .with_strict(true);
            base_request(prompt).with_tools(vec![tool])
        }
        _ => base_request(prompt),
    }
}

/// The options one findings request carries: temperature zero, the format's shape, and — when the
/// config asks for one — a cap on what the model may spend on its reply.
fn chat_options(format: FindingsFormat, max_tokens: Option<u32>) -> ChatOptions {
    let options = match format {
        FindingsFormat::JsonSchema => ChatOptions::default()
            .with_temperature(0.0)
            .with_response_format(ChatResponseFormat::JsonSpec(JsonSpec::new(
                "skill_findings",
                findings_json_schema(),
            ))),
        FindingsFormat::Tool => ChatOptions::default()
            .with_temperature(0.0)
            .with_tool_choice(ToolChoice::tool(REPORT_FINDINGS_TOOL)),
        FindingsFormat::JsonMode => ChatOptions::default()
            .with_temperature(0.0)
            .with_response_format(ChatResponseFormat::JsonMode),
    };

    match max_tokens {
        Some(cap) => options.with_max_tokens(cap),
        None => options,
    }
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
        let options = chat_options(format, self.max_tokens);

        let mut attempt = 0;
        let response = loop {
            let request = build_request(prompt, format);
            let semaphore = Arc::clone(&self.concurrency);
            let options = options.clone();

            match self.runtime.block_on(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("the request limiter is never closed");
                self.client
                    .exec_chat(&self.model, request, Some(&options))
                    .await
            }) {
                Ok(response) => break response,
                Err(failure) => {
                    // Transport-level failures — a rate limit, a 5xx, a connection that never
                    // came up — say nothing about the skill and are worth asking again. A 400 or
                    // a bad reply is a fact about the config, and retrying only hides it.
                    let Some(retry_after) = retry::transport_retry_after(&failure) else {
                        return Err(failure).with_context(|| format!("asking {}", self.model));
                    };

                    if attempt >= self.max_retries {
                        return Err(failure).with_context(|| format!("asking {}", self.model));
                    }

                    std::thread::sleep(retry::retry_delay(retry_after, attempt));
                    attempt += 1;
                }
            }
        };

        extract_findings_body(response)
    }

    fn describe(&self) -> String {
        self.model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn config(provider: Provider, model: &str) -> LlmConfig {
        LlmConfig {
            provider,
            model: model.into(),
            api_key_env: Some("SLINT_TEST_KEY".into()),
            base_url: None,
            timeout_seconds: 5,
            max_tokens: None,
            max_retries: 0,
            max_concurrent_requests: 4,
            max_input_bytes: 1024,
            api_key: None,
        }
    }

    use crate::llm::mock::MockServer;

    /// A provider at a local mock server, needing no credential and no network.
    fn local_client(server: &MockServer, adjust: impl FnOnce(&mut LlmConfig)) -> GenAiChat {
        let mut llm = config(Provider::Ollama, "llama3.2");
        llm.api_key_env = None;
        llm.base_url = Some(format!("http://{}/v1", server.address));
        adjust(&mut llm);
        GenAiChat::new(&llm).expect("the mock provider needs no credential")
    }

    fn prompt() -> Prompt {
        Prompt {
            system: "system".into(),
            user: "user".into(),
        }
    }

    #[test]
    fn a_hanging_provider_is_abandoned_at_the_configured_timeout_instead_of_never() {
        let server = MockServer::start(String::new(), true);
        let client = local_client(&server, |llm| llm.timeout_seconds = 1);

        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            sender.send(client.complete_findings(&prompt(), FindingsFormat::JsonMode))
        });

        let outcome = receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("the request must end at the configured timeout instead of hanging forever");

        assert!(
            outcome.is_err(),
            "a provider that never answers must be abandoned: {outcome:?}"
        );
    }

    #[test]
    fn model_requests_are_bounded_by_max_concurrent_requests() {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        let server = MockServer::start(MockServer::ollama_reply(), false);
        let client = Arc::new(local_client(&server, |llm| {
            llm.max_concurrent_requests = 2;
        }));

        let handles: Vec<_> = (0..6)
            .map(|_| {
                let client = Arc::clone(&client);
                std::thread::spawn(move || {
                    client.complete_findings(&prompt(), FindingsFormat::JsonMode)
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .unwrap()
                .expect("every request succeeds against the mock");
        }

        assert_eq!(server.requests.load(Ordering::SeqCst), 6);
        let observed = server.max_in_flight.load(Ordering::SeqCst);
        assert!(
            observed <= 2,
            "the limiter must hold requests back, not let {observed} run at once"
        );
    }

    #[test]
    fn a_rate_limited_request_is_retried_honouring_retry_after() {
        use std::sync::atomic::Ordering;

        let server =
            MockServer::start(MockServer::status(429, "Too Many Requests", Some(0)), false);
        let client = local_client(&server, |llm| llm.max_retries = 2);

        let started = std::time::Instant::now();
        let outcome = client.complete_findings(&prompt(), FindingsFormat::JsonMode);

        assert!(
            outcome.is_err(),
            "once the retries are spent the failure stands"
        );
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            3,
            "one attempt plus max_retries"
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "retry-after: 0 must replace the default backoff, not add to it (took {elapsed:?})"
        );
    }

    #[test]
    fn a_server_error_is_retried_before_the_failure_stands() {
        use std::sync::atomic::Ordering;

        let server = MockServer::start(MockServer::status(502, "Bad Gateway", None), false);
        let client = local_client(&server, |llm| llm.max_retries = 2);

        let outcome = client.complete_findings(&prompt(), FindingsFormat::JsonMode);

        assert!(outcome.is_err());
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            3,
            "one attempt plus max_retries"
        );
    }

    #[test]
    fn a_client_error_is_not_worth_retrying() {
        use std::sync::atomic::Ordering;

        let server = MockServer::start(MockServer::status(400, "Bad Request", None), false);
        let client = local_client(&server, |llm| llm.max_retries = 2);

        let outcome = client.complete_findings(&prompt(), FindingsFormat::JsonMode);

        assert!(outcome.is_err());
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            1,
            "a 400 is a fact about the config, not a transient failure"
        );
    }

    #[test]
    fn a_max_tokens_cap_is_applied_to_the_reply_options() {
        let options = chat_options(FindingsFormat::JsonMode, Some(512));
        assert_eq!(options.max_tokens, Some(512));
        assert_eq!(options.temperature, Some(0.0));

        let options = chat_options(FindingsFormat::JsonMode, None);
        assert_eq!(options.max_tokens, None, "no cap is invented silently");

        let schema = chat_options(FindingsFormat::JsonSchema, Some(128));
        assert_eq!(schema.max_tokens, Some(128));
        assert!(
            schema.response_format.is_some(),
            "the findings schema must survive the cap"
        );

        let tool = chat_options(FindingsFormat::Tool, Some(128));
        assert_eq!(tool.max_tokens, Some(128));
        assert!(
            tool.tool_choice.is_some(),
            "the forced tool must survive the cap"
        );
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
