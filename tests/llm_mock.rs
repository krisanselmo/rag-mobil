//! Drives `LlmGenerator` against a local mock of the Claude Messages API, to
//! check the HTTP request genai actually sends and the refusal fallback.

use std::sync::{Arc, Mutex};

use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use rag_mobil::RagError;
use rag_mobil::augment::Prompt;
use rag_mobil::generator::{LlmConfig, LlmGenerator, StoryGenerator, openai_compatible_client};
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
    let usage = json!({"input_tokens": 100, "output_tokens": 1});
    claude_stream_usage(model, chunks, stop_reason, usage)
}

/// The same, with the input `usage` of `message_start`.
fn claude_stream_usage(model: &str, chunks: &[&str], stop_reason: &str, usage: Value) -> String {
    let mut sse = event(
        "message_start",
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
            "role": "assistant", "model": model, "content": [], "usage": usage}}),
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
        system: "You are a storyteller.".into(),
        user: "<set number=\"3666\"></set>\nWrite a story.".into(),
        cached: 0,
    }
}

#[tokio::test]
async fn streams_story_and_sends_claude_settings() {
    let (url, recorded) = mock_server(vec![claude_stream(
        "claude-opus-5-5",
        &["Once upon ", "a time…"],
        "end_turn",
    )])
    .await;

    let mut streamed = String::new();
    let generation = generator(url, None)
        .generate(&prompt(), |t| streamed.push_str(t))
        .await
        .unwrap();

    assert_eq!(generation.text, "Once upon a time…");
    assert_eq!(streamed, generation.text);
    assert_eq!(generation.model, "claude-opus-5-5");
    // genai adds message_start's output_tokens (1) to message_delta's
    // cumulative count (42): the total is approximate by a token or so.
    assert_eq!(generation.usage.output, Some(43));
    assert_eq!(generation.usage.input, Some(100));
    assert_eq!(generation.usage.cache_read, None);
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
    assert!(
        body["system"]
            .to_string()
            .contains("You are a storyteller.")
    );
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body["messages"][0].to_string().contains("number"));
}

#[tokio::test]
async fn shared_prefix_is_cached_and_counted() {
    let usage = json!({"input_tokens": 20, "output_tokens": 1,
        "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 0});
    let (url, recorded) = mock_server(vec![claude_stream_usage(
        "claude-opus-5-5",
        &["Part 2."],
        "end_turn",
        usage,
    )])
    .await;
    let prompt = Prompt {
        cached: "<set number=\"3666\"></set>\n".len(),
        ..prompt()
    };

    let generation = generator(url, None)
        .generate(&prompt, |_| {})
        .await
        .unwrap();

    assert_eq!(generation.usage.input, Some(3020));
    assert_eq!(generation.usage.cache_read, Some(3000));
    let requests = recorded.lock().unwrap();
    let messages = &requests[0].body["messages"];
    assert_eq!(
        messages[0],
        json!({"role": "user", "content": [{"type": "text",
            "text": "<set number=\"3666\"></set>\n",
            "cache_control": {"type": "ephemeral"}}]})
    );
    assert_eq!(
        messages[1],
        json!({"role": "user", "content": "Write a story."})
    );
}

#[tokio::test]
async fn retries_once_on_fallback_model_after_refusal() {
    let (url, recorded) = mock_server(vec![
        claude_stream("claude-opus-5-5", &["Partiel"], "refusal"),
        claude_stream("claude-opus-4-8", &["Complete story."], "end_turn"),
    ])
    .await;

    let generation = generator(url, Some("claude-opus-4-8"))
        .generate(&prompt(), |_| {})
        .await
        .unwrap();

    assert_eq!(generation.text, "Complete story.");
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
        &["Once upon"],
        "max_tokens",
    )])
    .await;

    let generation = generator(url, None)
        .generate(&prompt(), |_| {})
        .await
        .unwrap();
    assert!(generation.truncated);
}

#[tokio::test]
async fn structured_output_sends_json_schema() {
    let (url, recorded) = mock_server(vec![claude_stream(
        "claude-opus-5-5",
        &["{\"issues\": ", "[]}"],
        "end_turn",
    )])
    .await;

    let schema = json!({
        "type": "object",
        "properties": {"issues": {"type": "array", "items": {"type": "string"}}},
        "required": ["issues"],
    });
    let generation = generator(url, None)
        .generate_json(&prompt(), "story_review", &schema)
        .await
        .unwrap();
    assert_eq!(generation.text, "{\"issues\": []}");

    let requests = recorded.lock().unwrap();
    let output_config = &requests[0].body["output_config"];
    assert_eq!(output_config["effort"], "medium");
    assert_eq!(output_config["format"]["type"], "json_schema");
    assert_eq!(
        output_config["format"]["schema"]["properties"]["issues"]["type"],
        "array"
    );
    assert_eq!(
        output_config["format"]["schema"]["additionalProperties"],
        false
    );
}

/// An OpenAI chat-completions stream, as llama.cpp or vLLM send it.
fn openai_stream(model: &str, chunks: &[&str]) -> String {
    let mut sse = String::new();
    for chunk in chunks {
        let data = json!({"id": "c1", "object": "chat.completion.chunk", "model": model,
            "choices": [{"index": 0, "delta": {"content": chunk}, "finish_reason": null}]});
        sse.push_str(&format!("data: {data}\n\n"));
    }
    let end = json!({"id": "c1", "object": "chat.completion.chunk", "model": model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 7, "total_tokens": 107}});
    sse.push_str(&format!("data: {end}\n\ndata: [DONE]\n\n"));
    sse
}

fn local_generator(url: &str) -> LlmGenerator {
    LlmGenerator::with_client(
        openai_compatible_client(url, false),
        LlmConfig {
            model: "openai::ministral-3-3b".into(),
            fallback_model: None,
            ..LlmConfig::default()
        },
    )
}

#[tokio::test]
async fn openai_compatible_server_gets_the_story_request() {
    let (url, recorded) = mock_server(vec![openai_stream(
        "ministral-3-3b",
        &["Once upon ", "a time…"],
    )])
    .await;

    let generation = local_generator(&url)
        .generate(&prompt(), |_| {})
        .await
        .unwrap();
    assert_eq!(generation.text, "Once upon a time…");

    let requests = recorded.lock().unwrap();
    let request = &requests[0];
    assert!(request.head.starts_with("post /v1/chat/completions"));
    assert!(request.head.contains("authorization: bearer none"));
    assert_eq!(request.body["model"], "ministral-3-3b");
    assert_eq!(request.body["stream"], true);
    assert!(
        request.body["messages"]
            .to_string()
            .contains("You are a storyteller.")
    );
}

#[tokio::test]
async fn openai_compatible_server_gets_the_json_schema() {
    let (url, recorded) = mock_server(vec![openai_stream(
        "ministral-3-3b",
        &["{\"issues\": ", "[]}"],
    )])
    .await;

    let schema = json!({
        "type": "object",
        "properties": {"issues": {"type": "array", "items": {"type": "string"}}},
        "required": ["issues"],
    });
    let generation = local_generator(&url)
        .generate_json(&prompt(), "story_review", &schema)
        .await
        .unwrap();
    assert_eq!(generation.text, "{\"issues\": []}");

    let format = &recorded.lock().unwrap()[0].body["response_format"];
    assert_eq!(format["type"], "json_schema");
    assert_eq!(
        format["json_schema"]["schema"]["properties"]["issues"]["type"],
        "array"
    );
}
