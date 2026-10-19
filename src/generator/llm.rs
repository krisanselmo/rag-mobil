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

use futures_util::StreamExt;
use genai::Client;
use genai::adapter::AdapterKind;
use genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatStreamEvent, ReasoningEffort, StopReason,
};

use crate::augment::Prompt;
use crate::error::{RagError, Result};
use crate::generator::{Generation, StoryGenerator};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Model retried once when Claude declines a request (safety classifier).
pub const DEFAULT_FALLBACK_MODEL: &str = "claude-opus-4-8";
/// Claude needs an explicit `max_tokens`; streaming makes a high cap safe.
const CLAUDE_MAX_TOKENS: u32 = 64_000;

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
    /// provider's usual environment variable.
    pub fn new(config: LlmConfig) -> Self {
        Self::with_client(Client::default(), config)
    }

    /// Custom genai client (other endpoint, auth resolver, tests...).
    pub fn with_client(client: Client, config: LlmConfig) -> Self {
        Self { client, config }
    }

    async fn stream_once<F>(
        &self,
        model: &str,
        request: &ChatRequest,
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
        let options = chat_options(kind, self.config.effort.clone());
        let response = self
            .client
            .exec_chat_stream(model, request.clone(), Some(&options))
            .await?;

        let mut attempt = Attempt {
            kind,
            model: response.model_iden.model_name.to_string(),
            text: String::new(),
            stop_reason: None,
            output_tokens: None,
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
                    attempt.output_tokens = end
                        .captured_usage
                        .and_then(|u| u.completion_tokens)
                        .and_then(|t| u64::try_from(t).ok());
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
    output_tokens: Option<u64>,
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

impl StoryGenerator for LlmGenerator {
    async fn generate<F>(&self, prompt: &Prompt, mut on_text: F) -> Result<Generation>
    where
        F: FnMut(&str) + Send,
    {
        let request = ChatRequest::new(vec![
            ChatMessage::system(prompt.system.as_str()),
            ChatMessage::user(prompt.user.as_str()),
        ]);

        let mut attempt = self
            .stream_once(&self.config.model, &request, &mut on_text)
            .await?;

        if attempt.refused()
            && attempt.kind == AdapterKind::Anthropic
            && let Some(fallback) = &self.config.fallback_model
        {
            // A refusal can land mid-stream: the partial text is dropped.
            eprintln!(
                "\n[rag-mobil] {} a décliné la requête, nouvelle tentative avec {fallback} \
                 (le texte ci-dessus est abandonné)",
                attempt.model
            );
            attempt = self.stream_once(fallback, &request, &mut on_text).await?;
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
            output_tokens: attempt.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn detects_refusals_across_providers() {
        assert!(is_refusal(&StopReason::from("refusal".to_owned())));
        assert!(is_refusal(&StopReason::from("SAFETY".to_owned())));
        assert!(!is_refusal(&StopReason::from("end_turn".to_owned())));
        assert!(!is_refusal(&StopReason::from("max_tokens".to_owned())));
    }
}
