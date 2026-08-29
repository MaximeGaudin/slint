//! Whether a model request that just failed is worth attempting again, and after how long.
//!
//! The provider's own transport errors — a rate limit, a 5xx, a connection that never came up —
//! are the failures a linter can survive on its own: they say nothing about the skill or the
//! config, and the shape of the run (one request per skill, in parallel) is exactly the shape
//! that trips a rate limit in the first place. Everything else — a 400, an auth failure, a reply
//! the adapter could not read — is a fact about the setup, and retrying it only hides it.

use std::time::Duration;

use genai::Error as GenAiError;

/// Retries wait at most this long between attempts, whatever the provider asks for.
const RETRY_DELAY_CAP: Duration = Duration::from_secs(60);

/// The longest gap in the default backoff schedule.
const BACKOFF_CAP_SECS: u64 = 8;

/// What to do about a transport failure.
///
/// `None`: not transport-level, or not worth retrying — the error is final. `Some(None)`: retry
/// after the default backoff. `Some(Some(delay))`: retry, and wait exactly this long — the
/// provider asked for it by name.
pub fn transport_retry_after(failure: &GenAiError) -> Option<Option<Duration>> {
    match failure {
        GenAiError::WebModelCall { webc_error, .. } | GenAiError::WebAdapterCall { webc_error, .. } => {
            match webc_error {
                genai::webc::Error::ResponseFailedStatus { status, headers, .. } => {
                    if !(status.as_u16() == 429 || status.is_server_error()) {
                        return None;
                    }

                    Some(
                        headers
                            .get("retry-after")
                            .and_then(|value| value.to_str().ok())
                            .and_then(parse_retry_after),
                    )
                }
                // A connection that never came up, or a request the timeout had to kill.
                genai::webc::Error::Reqwest(reqwest_error)
                    if reqwest_error.is_connect() || reqwest_error.is_timeout() =>
                {
                    Some(None)
                }
                _ => None,
            }
        }
        // The streaming path reports statuses with no headers attached.
        GenAiError::HttpError { status, .. } => {
            if !(status.as_u16() == 429 || status.is_server_error()) {
                return None;
            }

            Some(None)
        }
        _ => None,
    }
}

/// The default backoff for the attempt that just failed: 1s, 2s, 4s, then 8s for good.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.min(3)).min(BACKOFF_CAP_SECS))
}

/// The delay a provider asked for in `Retry-After`, when it sent one we can read.
///
/// Only whole seconds are understood; an HTTP-date (or anything else) is not worth a calendar.
fn parse_retry_after(text: &str) -> Option<Duration> {
    text.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// The wait before the next attempt: what the provider asked for, else the backoff, capped.
pub fn retry_delay(retry_after: Option<Duration>, attempt: u32) -> Duration {
    retry_after
        .unwrap_or_else(|| backoff(attempt))
        .min(RETRY_DELAY_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::adapter::AdapterKind;
    use genai::ModelIden;
    use reqwest::header::{HeaderMap, HeaderValue};
    use reqwest::StatusCode;

    fn status_error(status: StatusCode, retry_after: Option<&str>) -> GenAiError {
        let mut headers = HeaderMap::new();
        if let Some(value) = retry_after {
            headers.insert("retry-after", HeaderValue::from_str(value).unwrap());
        }

        GenAiError::WebModelCall {
            model_iden: ModelIden::new(AdapterKind::Ollama, "llama3.2"),
            webc_error: genai::webc::Error::ResponseFailedStatus {
                status,
                body: "{}".into(),
                headers: Box::new(headers),
            },
        }
    }

    #[test]
    fn a_rate_limit_is_retried_after_what_the_provider_asks() {
        let failure = status_error(StatusCode::TOO_MANY_REQUESTS, Some("7"));
        assert_eq!(
            transport_retry_after(&failure),
            Some(Some(Duration::from_secs(7)))
        );
    }

    #[test]
    fn a_server_error_is_retried_after_the_default_backoff() {
        let failure = status_error(StatusCode::BAD_GATEWAY, None);
        assert_eq!(transport_retry_after(&failure), Some(None));
    }

    #[test]
    fn a_client_error_is_final() {
        let failure = status_error(StatusCode::BAD_REQUEST, None);
        assert_eq!(transport_retry_after(&failure), None);
        let failure = status_error(StatusCode::UNAUTHORIZED, Some("1"));
        assert_eq!(transport_retry_after(&failure), None);
    }

    #[test]
    fn a_retry_after_nobody_can_read_falls_back_to_backoff() {
        let failure = status_error(StatusCode::TOO_MANY_REQUESTS, Some("Thu, 01 Jan 2026 00:00:00 GMT"));
        assert_eq!(
            transport_retry_after(&failure),
            Some(None),
            "an HTTP-date Retry-After is not worth a calendar; use the backoff"
        );
    }

    #[test]
    fn a_genai_status_error_is_treated_like_the_webc_one() {
        let failure = GenAiError::HttpError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            canonical_reason: "unavailable".into(),
            body: "{}".into(),
        };
        assert_eq!(transport_retry_after(&failure), Some(None));
    }

    #[test]
    fn a_non_transport_failure_is_never_retried() {
        let failure = GenAiError::ChatResponse {
            model_iden: ModelIden::new(AdapterKind::Ollama, "llama3.2"),
            body: serde_json::json!({"error": "nope"}),
        };
        assert_eq!(transport_retry_after(&failure), None);
    }

    #[test]
    fn the_backoff_doubles_and_then_stops_growing() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(2), Duration::from_secs(4));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(9), Duration::from_secs(8));
    }

    #[test]
    fn an_explicit_retry_after_wins_over_the_backoff_and_is_capped() {
        assert_eq!(
            retry_delay(Some(Duration::from_secs(3)), 2),
            Duration::from_secs(3)
        );
        assert_eq!(
            retry_delay(Some(Duration::from_secs(3_600)), 0),
            Duration::from_secs(60),
            "an hour of hanging a save hook is not a retry, it is an outage"
        );
        assert_eq!(retry_delay(None, 0), backoff(0));
    }
}
