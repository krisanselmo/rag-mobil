//! Multi-provider generator built on the open-source [`genai`] crate
//! (MIT/Apache-2.0): Claude, OpenAI, Gemini, Mistral, Groq, DeepSeek, a local
//! Ollama model, etc. The provider is picked from the model name:
//!
//! | Model name                  | Provider  | API key            |
//! |-----------------------------|-----------|--------------------|
//! | `claude-…`                  | Anthropic | `ANTHROPIC_API_KEY`|
//! | `gpt-…`                     | OpenAI    | `OPENAI_API_KEY`   |
//! | `gemini-…`                  | Gemini    | `GEMINI_API_KEY`   |
//! | `ollama::<model>` or other  | Ollama    | none (local)       |
//!
//! With `RAG_MOBIL_OPENAI_BASE_URL` set, `openai::<model>` goes to that
//! OpenAI-compatible server (llama.cpp, vLLM, LM Studio…) instead of
//! api.openai.com, and needs no key.

use futures_util::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{
    CacheControl, ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, JsonSpec,
    ReasoningEffort, StopReason, Usage,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ServiceTarget};
use serde_json::Value;

use crate::augment::Prompt;
use crate::error::{RagError, Result};
use crate::generator::{Generation, StoryGenerator, TokenUsage};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Model retried once when Claude declines a request (safety classifier).
pub const DEFAULT_FALLBACK_MODEL: &str = "claude-opus-4-8";
/// Claude needs an explicit `max_tokens`; streaming makes a high cap safe.
const CLAUDE_MAX_TOKENS: u32 = 64_000;
/// An OpenAI-compatible server for `openai::<model>`, e.g. `http://localhost:8080/v1`.
pub const OPENAI_BASE_URL_ENV: &str = "RAG_MOBIL_OPENAI_BASE_URL";
/// Sent when `OPENAI_API_KEY` is unset: local servers ignore the key, genai requires one.
const NO_KEY: &str = "none";

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub model: String,
    /// `None` = provider default (`medium` for Claude, untouched elsewhere:
    /// many local models reject reasoning settings).
    pub effort: Option<ReasoningEffort>,
    /// Retried once on a refusal. Only used for Claude models.
    pub fallback_model: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            effort: None,
            fallback_model: Some(DEFAULT_FALLBACK_MODEL.to_owned()),
        }
    }
}

pub struct LlmGenerator {
    client: Client,
    config: LlmConfig,
}

impl LlmGenerator {
    /// Uses genai's defaults: provider from the model name, API key from the
    /// provider's usual environment variable. `RAG_MOBIL_OPENAI_BASE_URL`
    /// redirects the OpenAI provider to a compatible server.
    pub fn new(config: LlmConfig) -> Self {
        let client = match openai_base_url(std::env::var(OPENAI_BASE_URL_ENV).ok().as_deref()) {
            Some(url) => openai_compatible_client(&url, std::env::var("OPENAI_API_KEY").is_ok()),
            None => Client::default(),
        };
        Self::with_client(client, config)
    }

    /// Custom genai client (other endpoint, auth resolver, tests...).
    pub fn with_client(client: Client, config: LlmConfig) -> Self {
        Self { client, config }
    }

    async fn stream_once<F>(
        &self,
        model: &str,
        request: &ChatRequest,
        format: Option<&JsonSpec>,
        on_text: &mut F,
    ) -> Result<Attempt>
    where
        F: FnMut(&str) + Send,
    {
        let kind = self
            .client
            .resolve_service_target(model)
            .await?
            .model
            .adapter_kind;
        let mut options = chat_options(kind, self.config.effort.clone());
        if let Some(format) = format {
            options = options.with_response_format(format.clone());
        }
        let response = self
            .client
            .exec_chat_stream(model, request.clone(), Some(&options))
            .await?;

        let mut attempt = Attempt {
            kind,
            model: response.model_iden.model_name.to_string(),
            text: String::new(),
            stop_reason: None,
            usage: TokenUsage::default(),
        };
        let mut stream = response.stream;
        while let Some(event) = stream.next().await {
            match event? {
                ChatStreamEvent::Chunk(chunk) => {
                    attempt.text.push_str(&chunk.content);
                    on_text(&chunk.content);
                }
                ChatStreamEvent::End(end) => {
                    attempt.stop_reason = end.captured_stop_reason;
                    attempt.usage = end.captured_usage.map(token_usage).unwrap_or_default();
                }
                // Start, reasoning (thinking) and tool-call events: not shown.
                _ => {}
            }
        }
        Ok(attempt)
    }
}

/// Result of one streamed request.
struct Attempt {
    kind: AdapterKind,
    model: String,
    text: String,
    stop_reason: Option<StopReason>,
    usage: TokenUsage,
}

/// genai's usage, in our terms (for Claude, genai counts the cached input
/// tokens in `prompt_tokens`).
fn token_usage(usage: Usage) -> TokenUsage {
    let count = |t: Option<i32>| t.and_then(|t| u64::try_from(t).ok());
    let details = usage.prompt_tokens_details.as_ref();
    TokenUsage {
        input: count(usage.prompt_tokens),
        cache_read: count(details.and_then(|d| d.cached_tokens)),
        cache_write: count(details.and_then(|d| d.cache_creation_tokens)),
        output: count(usage.completion_tokens),
    }
}

/// The request for `prompt`. For Claude, the prefix of the user message
/// that the next calls repeat ([`Prompt::cached`]) is a message of its own,
/// marked for the prompt cache: those calls read it (and the system prompt
/// before it) back instead of paying for it in full. Claude joins the two
/// user messages into one turn. Other providers get the prompt as is (OpenAI
/// and Gemini cache repeated prefixes by themselves).
fn chat_request(prompt: &Prompt, kind: AdapterKind) -> ChatRequest {
    let mut messages = vec![ChatMessage::system(prompt.system.as_str())];
    match prompt.split_user() {
        (shared, rest) if kind == AdapterKind::Anthropic && !shared.is_empty() => {
            messages.push(ChatMessage::user(shared).with_options(CacheControl::Ephemeral));
            if !rest.trim().is_empty() {
                messages.push(ChatMessage::user(rest));
            }
        }
        _ => messages.push(ChatMessage::user(prompt.user.as_str())),
    }
    ChatRequest::new(messages)
}

impl Attempt {
    fn refused(&self) -> bool {
        self.stop_reason.as_ref().is_some_and(is_refusal)
    }
}

/// Claude reports `"refusal"`; other providers use their content-filter reasons.
fn is_refusal(reason: &StopReason) -> bool {
    matches!(reason, StopReason::ContentFilter(_)) || reason.raw() == "refusal"
}

/// The base URL genai expects: blank means unset, and a trailing `/` so that
/// `chat/completions` is appended to it rather than replacing its last segment.
fn openai_base_url(raw: Option<&str>) -> Option<String> {
    let url = raw?.trim();
    if url.is_empty() {
        return None;
    }
    Some(if url.ends_with('/') {
        url.to_owned()
    } else {
        format!("{url}/")
    })
}

/// A client whose OpenAI provider talks to `base_url`; every other provider is
/// untouched. Without `has_key`, a placeholder key replaces `OPENAI_API_KEY`.
pub fn openai_compatible_client(base_url: &str, has_key: bool) -> Client {
    let base_url = openai_base_url(Some(base_url)).unwrap_or_default();
    let resolver = ServiceTargetResolver::from_resolver_fn(
        move |target: ServiceTarget| -> genai::resolver::Result<ServiceTarget> {
            if target.model.adapter_kind != AdapterKind::OpenAI {
                return Ok(target);
            }
            let auth = if has_key {
                target.auth
            } else {
                AuthData::from_single(NO_KEY)
            };
            Ok(ServiceTarget {
                endpoint: Endpoint::from_owned(base_url.clone()),
                auth,
                model: target.model,
            })
        },
    );
    Client::builder()
        .with_service_target_resolver(resolver)
        .build()
}

fn chat_options(kind: AdapterKind, effort: Option<ReasoningEffort>) -> ChatOptions {
    let mut options = ChatOptions::default().with_capture_usage(true);
    if kind == AdapterKind::Anthropic {
        // genai turns this into `thinking: {type: "adaptive"}` +
        // `output_config.effort` on current Claude models.
        options = options
            .with_max_tokens(CLAUDE_MAX_TOKENS)
            .with_reasoning_effort(effort.clone().unwrap_or(ReasoningEffort::Medium));
    } else if let Some(effort) = effort {
        options = options.with_reasoning_effort(effort);
    }
    options
}

impl LlmGenerator {
    /// One generation, with the refusal fallback. `format` asks for
    /// structured JSON output (`output_config.format` on Claude).
    async fn run<F>(
        &self,
        prompt: &Prompt,
        format: Option<&JsonSpec>,
        mut on_text: F,
    ) -> Result<Generation>
    where
        F: FnMut(&str) + Send,
    {
        let kind = self
            .client
            .resolve_service_target(&self.config.model)
            .await?
            .model
            .adapter_kind;
        let request = chat_request(prompt, kind);

        let mut attempt = self
            .stream_once(&self.config.model, &request, format, &mut on_text)
            .await?;

        if attempt.refused()
            && attempt.kind == AdapterKind::Anthropic
            && let Some(fallback) = &self.config.fallback_model
        {
            // A refusal can land mid-stream: the partial text is dropped.
            eprintln!(
                "\n[rag-mobil] {} declined the request, retrying with {fallback} \
                 (the text above is dropped)",
                attempt.model
            );
            attempt = self
                .stream_once(fallback, &request, format, &mut on_text)
                .await?;
        }

        if attempt.refused() {
            return Err(RagError::LlmRefusal {
                model: attempt.model,
            });
        }

        Ok(Generation {
            truncated: matches!(attempt.stop_reason, Some(StopReason::MaxTokens(_))),
            text: attempt.text,
            model: attempt.model,
            usage: attempt.usage,
        })
    }
}

impl StoryGenerator for LlmGenerator {
    async fn generate<F>(&self, prompt: &Prompt, on_text: F) -> Result<Generation>
    where
        F: FnMut(&str) + Send,
    {
        self.run(prompt, None, on_text).await
    }

    async fn generate_json(
        &self,
        prompt: &Prompt,
        name: &str,
        schema: &Value,
    ) -> Result<Generation> {
        let format = JsonSpec::new(name, schema.clone());
        self.run(prompt, Some(&format), |_| {}).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_blank_or_ends_with_a_slash() {
        assert_eq!(openai_base_url(None), None);
        assert_eq!(openai_base_url(Some("  ")), None);
        assert_eq!(
            openai_base_url(Some("http://llm:8080/v1")).as_deref(),
            Some("http://llm:8080/v1/")
        );
        assert_eq!(
            openai_base_url(Some("http://llm:8080/v1/")).as_deref(),
            Some("http://llm:8080/v1/")
        );
    }

    #[tokio::test]
    async fn only_the_openai_provider_is_redirected() {
        let client = openai_compatible_client("http://llm:8080/v1", false);

        let local = client
            .resolve_service_target("openai::ministral-3-3b")
            .await
            .unwrap();
        assert_eq!(local.endpoint.base_url(), "http://llm:8080/v1/");
        assert_eq!(local.model.adapter_kind, AdapterKind::OpenAI);
        assert_eq!(local.auth.single_key_value().unwrap(), NO_KEY);

        let claude = client
            .resolve_service_target("claude-opus-5-5")
            .await
            .unwrap();
        assert_eq!(claude.model.adapter_kind, AdapterKind::Anthropic);
        assert_ne!(claude.endpoint.base_url(), "http://llm:8080/v1/");
    }

    #[test]
    fn claude_gets_max_tokens_and_medium_effort_by_default() {
        let options = chat_options(AdapterKind::Anthropic, None);
        assert_eq!(options.max_tokens, Some(CLAUDE_MAX_TOKENS));
        assert!(matches!(
            options.reasoning_effort,
            Some(ReasoningEffort::Medium)
        ));
        assert_eq!(options.capture_usage, Some(true));
    }

    #[test]
    fn other_providers_keep_their_defaults() {
        let options = chat_options(AdapterKind::Ollama, None);
        assert_eq!(options.max_tokens, None);
        assert!(options.reasoning_effort.is_none());

        let options = chat_options(AdapterKind::OpenAI, Some(ReasoningEffort::High));
        assert!(matches!(
            options.reasoning_effort,
            Some(ReasoningEffort::High)
        ));
    }

    fn prompt(cached: usize) -> Prompt {
        Prompt {
            system: "rules".into(),
            user: "<set>…</set>\n\nWrite part 2.".into(),
            cached,
        }
    }

    fn texts(request: &ChatRequest) -> Vec<(Option<CacheControl>, String)> {
        request
            .messages
            .iter()
            .map(|m| {
                (
                    m.options.as_ref().and_then(|o| o.cache_control.clone()),
                    m.content.joined_texts().unwrap_or_default(),
                )
            })
            .collect()
    }

    #[test]
    fn claude_gets_the_shared_prefix_cached() {
        let shared = "<set>…</set>\n\n".len();
        let request = chat_request(&prompt(shared), AdapterKind::Anthropic);
        assert_eq!(
            texts(&request),
            [
                (None, "rules".to_owned()),
                (Some(CacheControl::Ephemeral), "<set>…</set>\n\n".to_owned()),
                (None, "Write part 2.".to_owned()),
            ]
        );

        // Nothing to cache, or another provider: one user message.
        for request in [
            chat_request(&prompt(0), AdapterKind::Anthropic),
            chat_request(&prompt(shared), AdapterKind::OpenAI),
        ] {
            assert_eq!(
                texts(&request),
                [
                    (None, "rules".to_owned()),
                    (None, "<set>…</set>\n\nWrite part 2.".to_owned()),
                ]
            );
        }
    }

    #[test]
    fn detects_refusals_across_providers() {
        assert!(is_refusal(&StopReason::from("refusal".to_owned())));
        assert!(is_refusal(&StopReason::from("SAFETY".to_owned())));
        assert!(!is_refusal(&StopReason::from("end_turn".to_owned())));
        assert!(!is_refusal(&StopReason::from("max_tokens".to_owned())));
    }
}
