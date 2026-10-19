//! Drives `LlmGenerator` against a local mock of the Claude Messages API, to
//! check the HTTP request genai actually sends and the refusal fallback.

use std::sync::{Arc, Mutex};

use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use rag_mobil::RagError;
use rag_mobil::augment::Prompt;
use rag_mobil::generator::{LlmConfig, LlmGenerator, StoryGenerator};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One recorded HTTP request.
struct Recorded {
    head: String,
    body: Value,
}

/// Serves the scripted SSE bodies in order, one per connection.
async fn mock_server(responses: Vec<String>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let log = recorded.clone();

    tokio::spawn(async move {
        for sse in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            // Read headers, then exactly Content-Length bytes of body.
            let (head, body) = loop {
                let n = socket.read(&mut buf).await.unwrap();
                raw.extend_from_slice(&buf[..n]);
                let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map_or(0, |v| v.trim().parse().unwrap());
                while raw.len() < split + 4 + len {
                    let n = socket.read(&mut buf).await.unwrap();
                    raw.extend_from_slice(&buf[..n]);
                }
                break (head, raw[split + 4..split + 4 + len].to_vec());
            };
            log.lock().unwrap().push(Recorded {
                head,
                body: serde_json::from_slice(&body).unwrap(),
            });

            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{sse}",
                sse.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (url, recorded)
}

fn event(name: &str, data: Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

/// A Claude stream: a (hidden) thinking block, then the text in `chunks`.
fn claude_stream(model: &str, chunks: &[&str], stop_reason: &str) -> String {
    let mut sse = event(
        "message_start",
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
            "role": "assistant", "model": model, "content": [],
            "usage": {"input_tokens": 100, "output_tokens": 1}}}),
    );
    sse += &event(
        "content_block_start",
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "thinking", "thinking": ""}}),
    );
    sse += &event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
    );
    sse += &event(
        "content_block_start",
        json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "text", "text": ""}}),
    );
    for chunk in chunks {
        sse += &event(
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1,
                "delta": {"type": "text_delta", "text": chunk}}),
        );
    }
    sse += &event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 1}),
    );
    sse += &event(
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": stop_reason},
            "usage": {"output_tokens": 42}}),
    );
    sse += &event("message_stop", json!({"type": "message_stop"}));
    sse
}

fn generator(url: String, fallback: Option<&str>) -> LlmGenerator {
    let resolver = ServiceTargetResolver::from_resolver_fn(
        move |target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            Ok(ServiceTarget {
                endpoint: Endpoint::from_owned(url.clone()),
                auth: AuthData::from_single("test-key"),
                model: ModelIden::new(AdapterKind::Anthropic, target.model.model_name),
            })
        },
    );
    let client = Client::builder()
        .with_service_target_resolver(resolver)
        .build();
    LlmGenerator::with_client(
        client,
        LlmConfig {
            fallback_model: fallback.map(str::to_owned),
            ..LlmConfig::default()
        },
    )
}

fn prompt() -> Prompt {
    Prompt {
        system: "Tu es un conteur.".into(),
        user: "<set numero=\"3666\"></set>\nÉcris une histoire.".into(),
    }
}

#[tokio::test]
async fn streams_story_and_sends_claude_settings() {
    let (url, recorded) = mock_server(vec![claude_stream(
        "claude-opus-5-5",
        &["Il était ", "une fois…"],
        "end_turn",
    )])
    .await;

    let mut streamed = String::new();
    let generation = generator(url, None)
        .generate(&prompt(), |t| streamed.push_str(t))
        .await
        .unwrap();

    assert_eq!(generation.text, "Il était une fois…");
    assert_eq!(streamed, generation.text);
    assert_eq!(generation.model, "claude-opus-5-5");
    // genai adds message_start's output_tokens (1) to message_delta's
    // cumulative count (42): the total is approximate by a token or so.
    assert_eq!(generation.output_tokens, Some(43));
    assert!(!generation.truncated);

    let requests = recorded.lock().unwrap();
    let request = &requests[0];
    assert!(request.head.starts_with("post /v1/messages"));
    assert!(request.head.contains("x-api-key: test-key"));
    assert!(request.head.contains("anthropic-version:"));
    let body = &request.body;
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 64_000);
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    assert_eq!(body["output_config"]["effort"], "medium");
    assert!(body["system"].to_string().contains("Tu es un conteur."));
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body["messages"][0].to_string().contains("numero"));
}

#[tokio::test]
async fn retries_once_on_fallback_model_after_refusal() {
    let (url, recorded) = mock_server(vec![
        claude_stream("claude-opus-5-5", &["Partiel"], "refusal"),
        claude_stream("claude-opus-4-8", &["Histoire complète."], "end_turn"),
    ])
    .await;

    let generation = generator(url, Some("claude-opus-4-8"))
        .generate(&prompt(), |_| {})
        .await
        .unwrap();

    assert_eq!(generation.text, "Histoire complète.");
    assert_eq!(generation.model, "claude-opus-4-8");
    let requests = recorded.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].body["model"], "claude-opus-4-8");
}

#[tokio::test]
async fn refusal_without_fallback_is_an_error() {
    let (url, _) = mock_server(vec![claude_stream(
        "claude-opus-5-5",
        &["Partiel"],
        "refusal",
    )])
    .await;

    let err = generator(url, None)
        .generate(&prompt(), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, RagError::LlmRefusal { model } if model == "claude-opus-5-5"));
}

#[tokio::test]
async fn max_tokens_marks_story_truncated() {
    let (url, _) = mock_server(vec![claude_stream(
        "claude-opus-5-5",
        &["Il était"],
        "max_tokens",
    )])
    .await;

    let generation = generator(url, None)
        .generate(&prompt(), |_| {})
        .await
        .unwrap();
    assert!(generation.truncated);
}
