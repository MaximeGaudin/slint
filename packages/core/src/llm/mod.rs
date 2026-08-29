//! The half of the catalogue a regular expression cannot answer.
//!
//! Everything here is optional and says so. With no provider configured the static rules still run,
//! and the report carries a note naming what did not — which is the difference between a review
//! that passed and a review that never happened.
//!
//! The wire formats are not ours: `genai` speaks OpenAI, Anthropic, Gemini, Ollama, OpenRouter and
//! Groq natively, and a base URL of our own covers every gateway and self-hosted server those shapes
//! reach. What is ours is the seam and the prompt.

pub mod cache;
pub mod provider;
pub mod retry;
pub mod review;
pub mod rules;

#[cfg(test)]
pub(crate) mod mock;

pub use provider::{Chat, FindingsFormat, GenAiChat, Prompt, findings_format_for};
pub use review::{
    UnparseableFindings, is_unparseable_findings, parse_response, review, review_with,
};
