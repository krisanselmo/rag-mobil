//! Claude Messages API client (raw HTTP + SSE streaming; there is no
//! official Anthropic SDK for Rust).

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;

use crate::augment::Prompt;
use crate::error::{RagError, Result};
use crate::generator::sse::SseDecoder;
use crate::generator::{Generation, StoryGenerator};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
/// Lets the API retry a safety-classifier decline on a fallback model
/// server-side (paired with `fallbacks: "default"`).
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// `low` | `medium` | `high` | `xhigh` | `max`.
    pub effort: String,
    pub max_tokens: u32,
}

impl AnthropicConfig {
    /// Reads `ANTHROPIC_API_KEY` (required) and `ANTHROPIC_BASE_URL` (optional).
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or(RagError::MissingEnv("ANTHROPIC_API_KEY"))?;
        Ok(Self {
            api_key,
            base_url: std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned()),
            model: DEFAULT_MODEL.to_owned(),
            effort: "medium".to_owned(),
            max_tokens: 64_000,
        })
    }
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    stream: bool,
    system: &'a str,
    messages: [Message<'a>; 1],
    thinking: Thinking,
    output_config: OutputConfig<'a>,
    fallbacks: &'static str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct OutputConfig<'a> {
    effort: &'a str,
}

pub struct AnthropicClient {
    http: reqwest::Client,
    config: AnthropicConfig,
}

impl AnthropicClient {
    pub fn new(config: AnthropicConfig) -> Result<Self> {
        // No global timeout: a long story streams for a while. The connect
        // timeout still catches an unreachable endpoint.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self { http, config })
    }
}

/// Reads `{"type":"error","error":{"type":..,"message":..}}`.
fn api_error_fields(body: &Value) -> (String, String) {
    let error = &body["error"];
    (
        error["type"].as_str().unwrap_or("unknown_error").to_owned(),
        error["message"]
            .as_str()
            .unwrap_or("(pas de message)")
            .to_owned(),
    )
}

/// Running state while consuming the event stream.
#[derive(Default)]
struct StreamState {
    text: String,
    model: Option<String>,
    stop_reason: Option<String>,
    refusal_category: Option<String>,
    output_tokens: Option<u64>,
}

impl StreamState {
    /// Applies one SSE `data:` payload. Returns any text to show live.
    fn apply(&mut self, data: &Value) -> Result<Option<String>> {
        match data["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.model = data["message"]["model"].as_str().map(str::to_owned);
            }
            "content_block_start" if data["content_block"]["type"] == "fallback" => {
                let to = data["content_block"]["to"]["model"].as_str().unwrap_or("?");
                eprintln!("\n[rag-mobil] requête reprise par le modèle de repli {to}");
                self.model = Some(to.to_owned());
            }
            "content_block_delta" if data["delta"]["type"] == "text_delta" => {
                let chunk = data["delta"]["text"].as_str().unwrap_or_default();
                self.text.push_str(chunk);
                return Ok(Some(chunk.to_owned()));
            }
            "message_delta" => {
                if let Some(reason) = data["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(category) = data["delta"]["stop_details"]["category"].as_str() {
                    self.refusal_category = Some(category.to_owned());
                }
                if let Some(tokens) = data["usage"]["output_tokens"].as_u64() {
                    self.output_tokens = Some(tokens);
                }
            }
            "error" => {
                let (kind, message) = api_error_fields(data);
                return Err(RagError::LlmStream { kind, message });
            }
            // ping, thinking blocks, content_block_stop, message_stop...
            _ => {}
        }
        Ok(None)
    }
}

impl StoryGenerator for AnthropicClient {
    async fn generate<F>(&self, prompt: &Prompt, mut on_text: F) -> Result<Generation>
    where
        F: FnMut(&str) + Send,
    {
        let body = MessagesRequest {
            model: &self.config.model,
            max_tokens: self.config.max_tokens,
            stream: true,
            system: &prompt.system,
            messages: [Message {
                role: "user",
                content: &prompt.user,
            }],
            thinking: Thinking { kind: "adaptive" },
            output_config: OutputConfig {
                effort: &self.config.effort,
            },
            fallbacks: "default",
        };

        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", API_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            let (kind, message) = api_error_fields(&body);
            return Err(RagError::LlmApi {
                status: status.as_u16(),
                kind,
                message,
            });
        }

        let mut decoder = SseDecoder::default();
        let mut state = StreamState::default();
        let mut bytes = response.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            for event in decoder.push(&chunk?) {
                if event.data.is_empty() {
                    continue;
                }
                let data: Value = serde_json::from_str(&event.data)?;
                if let Some(text) = state.apply(&data)? {
                    on_text(&text);
                }
            }
        }

        // A refusal can arrive mid-stream: the partial text must be discarded.
        if state.stop_reason.as_deref() == Some("refusal") {
            return Err(RagError::LlmRefusal {
                category: state
                    .refusal_category
                    .unwrap_or_else(|| "non précisée".into()),
            });
        }

        Ok(Generation {
            text: state.text,
            model: state.model.unwrap_or_else(|| self.config.model.clone()),
            truncated: state.stop_reason.as_deref() == Some("max_tokens"),
            output_tokens: state.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accumulates_text_and_detects_refusal() {
        let mut state = StreamState::default();
        state
            .apply(&json!({"type":"message_start","message":{"model":"claude-opus-5-5"}}))
            .unwrap();
        let shown = state
            .apply(&json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"Il était"}}))
            .unwrap();
        assert_eq!(shown.as_deref(), Some("Il était"));
        state
            .apply(&json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":""}}))
            .unwrap();
        state
            .apply(&json!({"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"category":"cyber"}},"usage":{"output_tokens":3}}))
            .unwrap();
        assert_eq!(state.text, "Il était");
        assert_eq!(state.stop_reason.as_deref(), Some("refusal"));
        assert_eq!(state.refusal_category.as_deref(), Some("cyber"));
        assert_eq!(state.output_tokens, Some(3));
    }

    #[test]
    fn stream_error_event_is_an_error() {
        let mut state = StreamState::default();
        let err = state
            .apply(
                &json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
            )
            .unwrap_err();
        assert!(matches!(err, RagError::LlmStream { kind, .. } if kind == "overloaded_error"));
    }

    #[test]
    fn request_body_shape() {
        let body = MessagesRequest {
            model: "claude-opus-5-5",
            max_tokens: 64_000,
            stream: true,
            system: "s",
            messages: [Message {
                role: "user",
                content: "u",
            }],
            thinking: Thinking { kind: "adaptive" },
            output_config: OutputConfig { effort: "medium" },
            fallbacks: "default",
        };
        let value = serde_json::to_value(&body).unwrap();
        assert_eq!(value["thinking"], json!({"type": "adaptive"}));
        assert_eq!(value["output_config"], json!({"effort": "medium"}));
        assert_eq!(
            value["messages"][0],
            json!({"role": "user", "content": "u"})
        );
        assert_eq!(value["fallbacks"], "default");
    }
}
