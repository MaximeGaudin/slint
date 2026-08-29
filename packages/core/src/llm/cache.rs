//! Remembering what the model already said, so an unchanged skill is not re-billed.
//!
//! The cache is content-addressed: the key is a digest of everything that decides what the model
//! is asked and how it may answer. A reply is only ever served back for the exact same request,
//! and the stored key is verified on read, so a digest collision degrades to a miss rather than
//! to somebody else's findings. Replies live in the system's temporary directory — cheap to
//! lose, cheap to keep, and never a file a repository has to ignore.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;
use crate::llm::provider::{Chat, FindingsFormat, Prompt};

/// Moves the cache somewhere else (tests, sandboxes).
const DIR_ENV: &str = "SLINT_CACHE_DIR";
/// Turns the cache off entirely.
const DISABLE_ENV: &str = "SLINT_NO_LLM_CACHE";

/// Where replies are remembered between runs.
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Cache {
    /// The cache the run was asked for: on, unless the environment says otherwise.
    pub fn from_env() -> Self {
        if std::env::var_os(DISABLE_ENV).is_some() {
            return Cache { dir: None };
        }

        let dir = match std::env::var_os(DIR_ENV) {
            Some(dir) => PathBuf::from(dir),
            None => std::env::temp_dir().join("slint-llm-cache"),
        };

        Cache { dir: Some(dir) }
    }

    /// A cache that remembers nothing.
    pub fn disabled() -> Self {
        Cache { dir: None }
    }

    /// The reply this exact request produced last time, when there was one.
    pub fn read(&self, key: &str) -> Option<String> {
        let entry: Entry = serde_json::from_str(
            &std::fs::read_to_string(entry_path(self.dir.as_ref()?, key)).ok()?,
        )
        .ok()?;

        (entry.key == key).then_some(entry.reply)
    }

    /// Remembers a reply. Failure to cache is never failure to review.
    pub fn write(&self, key: &str, reply: &str) {
        let Some(dir) = self.dir.as_ref() else {
            return;
        };

        if std::fs::create_dir_all(dir).is_err() {
            return;
        }

        let Ok(json) = serde_json::to_string(&Entry {
            key: key.to_string(),
            reply: reply.to_string(),
        }) else {
            return;
        };

        let path = entry_path(dir, key);
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, json).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    key: String,
    reply: String,
}

fn entry_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.json"))
}

/// The stable key of one exact request: everything that decides what comes back.
pub fn cache_key(llm: &LlmConfig, format: FindingsFormat, prompt: &Prompt) -> String {
    let material = format!(
        "v1\u{0}{:?}\u{0}{}\u{0}{format:?}\u{0}{}\u{0}{}\u{0}{:?}",
        llm.provider, llm.model, prompt.system, prompt.user, llm.max_tokens
    );

    digest(&material)
}

/// FNV-1a, hex-encoded. Not cryptographic — the stored key is verified on read, so a collision
/// can only cost a re-request, never produce the wrong findings.
fn digest(material: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A [`Chat`] that answers a repeated identical request from the cache instead of the provider.
pub struct Cached<'a> {
    client: &'a dyn Chat,
    cache: Cache,
    llm: &'a LlmConfig,
}

impl<'a> Cached<'a> {
    pub fn new(client: &'a dyn Chat, llm: &'a LlmConfig) -> Self {
        Cached {
            client,
            cache: Cache::disabled(),
            llm,
        }
    }
}

impl Chat for Cached<'_> {
    fn complete(&self, prompt: &Prompt) -> Result<String> {
        self.client.complete(prompt)
    }

    fn complete_findings(&self, prompt: &Prompt, format: FindingsFormat) -> Result<String> {
        let key = cache_key(self.llm, format, prompt);
        if let Some(reply) = self.cache.read(&key) {
            return Ok(reply);
        }

        let reply = self.client.complete_findings(prompt, format)?;
        self.cache.write(&key, &reply);
        Ok(reply)
    }

    fn describe(&self) -> String {
        self.client.describe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Provider;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn llm() -> LlmConfig {
        LlmConfig {
            provider: Provider::Openai,
            model: "gpt-5-mini".into(),
            ..LlmConfig::default()
        }
    }

    fn prompt(body: &str) -> Prompt {
        Prompt {
            system: "system".into(),
            user: body.into(),
        }
    }

    #[test]
    fn a_written_reply_is_served_back_for_the_same_request() {
        let cache = Cache {
            dir: Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        };

        assert_eq!(cache.read(&cache_key(&llm(), FindingsFormat::JsonMode, &prompt("a"))), None);

        cache.write(&cache_key(&llm(), FindingsFormat::JsonMode, &prompt("a")), "[]");
        assert_eq!(
            cache.read(&cache_key(&llm(), FindingsFormat::JsonMode, &prompt("a"))),
            Some("[]".to_string())
        );
    }

    #[test]
    fn a_different_request_is_a_miss_even_at_the_same_path() {
        let cache = Cache {
            dir: Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        };

        cache.write(&cache_key(&llm(), FindingsFormat::JsonMode, &prompt("a")), "[]");

        assert_eq!(
            cache.read("0123456789abcdef"),
            None,
            "a key nobody wrote must be a miss, whatever is on disk"
        );
    }

    #[test]
    fn a_disabled_cache_never_reads_or_writes() {
        let cache = Cache::disabled();

        cache.write("anything", "[]");
        assert_eq!(cache.read("anything"), None);
    }

    /// Counts how often the provider behind the cache was really asked.
    struct Counting {
        calls: AtomicUsize,
    }

    impl Chat for Counting {
        fn complete(&self, _prompt: &Prompt) -> Result<String> {
            anyhow::bail!("only complete_findings is exercised here")
        }

        fn complete_findings(&self, _prompt: &Prompt, _format: FindingsFormat) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(r#"[{"rule":"llm/no-ambiguity","message":"Step 2 can be read two ways.","line":8,"confidence":0.7}]"#.into())
        }

        fn describe(&self) -> String {
            "fake/counting".into()
        }
    }

    #[test]
    fn an_unchanged_request_is_answered_from_the_cache_not_the_provider() {
        let provider = Counting {
            calls: AtomicUsize::new(0),
        };
        let cache = Cache {
            dir: Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        };
        let cached = Cached {
            client: &provider,
            cache,
            llm: &llm(),
        };

        let first = cached
            .complete_findings(&prompt("a"), FindingsFormat::JsonMode)
            .unwrap();
        let second = cached
            .complete_findings(&prompt("a"), FindingsFormat::JsonMode)
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            1,
            "the same request twice must reach the provider once"
        );
    }

    #[test]
    fn a_changed_request_reaches_the_provider_again() {
        let provider = Counting {
            calls: AtomicUsize::new(0),
        };
        let cache = Cache {
            dir: Some(tempfile::tempdir().unwrap().path().to_path_buf()),
        };
        let cached = Cached {
            client: &provider,
            cache,
            llm: &llm(),
        };

        cached
            .complete_findings(&prompt("a"), FindingsFormat::JsonMode)
            .unwrap();
        cached
            .complete_findings(&prompt("b"), FindingsFormat::JsonMode)
            .unwrap();

        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_key_covers_everything_that_decides_the_reply() {
        let base = cache_key(&llm(), FindingsFormat::JsonMode, &prompt("a"));

        let mut other_model = llm();
        other_model.model = "gpt-5".into();
        assert_ne!(cache_key(&other_model, FindingsFormat::JsonMode, &prompt("a")), base);

        let mut other_cap = llm();
        other_cap.max_tokens = Some(64);
        assert_ne!(cache_key(&other_cap, FindingsFormat::JsonMode, &prompt("a")), base);

        let mut other_provider = llm();
        other_provider.provider = Provider::Groq;
        assert_ne!(cache_key(&other_provider, FindingsFormat::JsonMode, &prompt("a")), base);

        assert_ne!(cache_key(&llm(), FindingsFormat::Tool, &prompt("a")), base);
        assert_ne!(cache_key(&llm(), FindingsFormat::JsonMode, &prompt("b")), base);
    }
}
