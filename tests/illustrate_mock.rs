//! Drives `Illustrator` against local mocks of the OpenAI Images, Gemini and
//! Stable Diffusion web UI APIs, to check the requests sent and how the picture is read back.

use std::sync::{Arc, Mutex};

use rag_mobil::RagError;
use rag_mobil::illustrate::Illustrator;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One recorded HTTP request: the lowercased head and the JSON body.
type Recorded = Arc<Mutex<Option<(String, Value)>>>;

/// Answers a single request with `status` and the JSON `answer`.
async fn mock_server(status: u16, answer: Value) -> (String, Recorded) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let recorded: Recorded = Arc::default();
    let log = recorded.clone();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
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
        *log.lock().unwrap() = Some((head, serde_json::from_slice(&body).unwrap()));
        let answer = answer.to_string();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (url, recorded)
}

fn openai(base_url: String, model: &str) -> Illustrator {
    Illustrator::OpenAi {
        client: reqwest::Client::new(),
        base_url,
        key: Some("sk-test".into()),
        model: model.into(),
    }
}

#[tokio::test]
async fn openai_images_are_decoded_from_base64() {
    let (url, recorded) = mock_server(200, json!({"data": [{"b64_json": "iVBORw=="}]})).await;
    let picture = openai(url, "gpt-image-1").draw("a pirate").await.unwrap();
    assert_eq!(picture.bytes, b"\x89PNG");
    assert_eq!(picture.mime, "image/png");
    assert_eq!(picture.model, "gpt-image-1");

    let (head, body) = recorded.lock().unwrap().take().unwrap();
    assert!(head.starts_with("post /v1/images/generations "));
    assert!(head.contains("authorization: bearer sk-test"));
    assert_eq!(
        body,
        json!({"model": "gpt-image-1", "prompt": "a pirate", "n": 1, "size": "1536x1024"})
    );
}

#[tokio::test]
async fn dall_e_is_asked_for_base64() {
    let (url, recorded) = mock_server(200, json!({"data": [{"b64_json": "iVBORw=="}]})).await;
    openai(url, "dall-e-3").draw("a pirate").await.unwrap();
    let (_, body) = recorded.lock().unwrap().take().unwrap();
    assert_eq!(body["response_format"], "b64_json");
}

#[tokio::test]
async fn provider_errors_carry_their_message() {
    let (url, _) = mock_server(
        400,
        json!({"error": {"message": "Your request was rejected by the safety system."}}),
    )
    .await;
    let err = openai(url, "gpt-image-1")
        .draw("a pirate")
        .await
        .unwrap_err();
    match err {
        RagError::Image(message) => {
            assert!(message.contains("HTTP 400"), "{message}");
            assert!(message.contains("safety system"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn gemini_images_come_from_inline_data() {
    let answer = json!({"candidates": [{"content": {"parts": [
        {"text": "Here is your picture."},
        {"inlineData": {"mimeType": "image/jpeg", "data": "/9j/"}},
    ]}}]});
    let (url, recorded) = mock_server(200, answer).await;
    let gemini = Illustrator::Gemini {
        client: reqwest::Client::new(),
        base_url: url,
        key: "g-test".into(),
        model: "gemini-2.5-flash-image".into(),
    };
    let picture = gemini.draw("a pirate").await.unwrap();
    assert_eq!(picture.bytes, b"\xff\xd8\xff");
    assert_eq!(picture.mime, "image/jpeg");

    let (head, body) = recorded.lock().unwrap().take().unwrap();
    assert!(head.starts_with("post /v1beta/models/gemini-2.5-flash-image:generatecontent "));
    assert!(head.contains("x-goog-api-key: g-test"));
    assert_eq!(body["contents"][0]["parts"][0]["text"], "a pirate");
    assert_eq!(
        body["generationConfig"]["responseModalities"],
        json!(["IMAGE"])
    );
    assert_eq!(
        body["generationConfig"]["imageConfig"]["aspectRatio"],
        "16:9"
    );
}

#[tokio::test]
async fn openai_compatible_servers_need_no_key() {
    let (url, recorded) = mock_server(200, json!({"data": [{"b64_json": "iVBORw=="}]})).await;
    let local = Illustrator::OpenAi {
        client: reqwest::Client::new(),
        base_url: url,
        key: None,
        model: "flux.1-schnell".into(),
    };
    let picture = local.draw("a pirate").await.unwrap();
    assert_eq!(picture.bytes, b"\x89PNG");
    assert_eq!(picture.model, "flux.1-schnell");

    let (head, body) = recorded.lock().unwrap().take().unwrap();
    assert!(head.starts_with("post /v1/images/generations "));
    assert!(!head.contains("authorization:"));
    assert_eq!(body["response_format"], "b64_json");
    assert_eq!(body["size"], "1024x576");
}

fn stable_diffusion(base_url: String, checkpoint: Option<&str>) -> Illustrator {
    Illustrator::StableDiffusion {
        client: reqwest::Client::new(),
        base_url,
        checkpoint: checkpoint.map(str::to_owned),
    }
}

#[tokio::test]
async fn stable_diffusion_web_ui_draws_with_the_chosen_checkpoint() {
    let (url, recorded) = mock_server(200, json!({"images": ["iVBORw=="], "info": "{}"})).await;
    let picture = stable_diffusion(url, Some("sd_xl_base_1.0"))
        .draw("a pirate")
        .await
        .unwrap();
    assert_eq!(picture.bytes, b"\x89PNG");
    assert_eq!(picture.mime, "image/png");
    assert_eq!(picture.model, "sd_xl_base_1.0");

    let (head, body) = recorded.lock().unwrap().take().unwrap();
    assert!(head.starts_with("post /sdapi/v1/txt2img "));
    assert_eq!(body["prompt"], "a pirate");
    assert!(body["negative_prompt"].as_str().unwrap().contains("text"));
    assert_eq!(
        (body["width"].clone(), body["height"].clone()),
        (json!(1024), json!(576))
    );
    assert_eq!(
        body["override_settings"]["sd_model_checkpoint"],
        "sd_xl_base_1.0"
    );
}

#[tokio::test]
async fn stable_diffusion_web_ui_keeps_the_loaded_checkpoint() {
    let (url, recorded) =
        mock_server(200, json!({"images": ["data:image/png;base64,iVBORw=="]})).await;
    let picture = stable_diffusion(url, None).draw("a pirate").await.unwrap();
    assert_eq!(picture.bytes, b"\x89PNG");
    assert_eq!(picture.model, "sd");
    let (_, body) = recorded.lock().unwrap().take().unwrap();
    assert!(body.get("override_settings").is_none());
}

#[tokio::test]
async fn stable_diffusion_errors_carry_their_detail() {
    let (url, _) = mock_server(404, json!({"detail": "Not Found"})).await;
    match stable_diffusion(url, None)
        .draw("a pirate")
        .await
        .unwrap_err()
    {
        RagError::Image(message) => {
            assert!(message.contains("HTTP 404: Not Found"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}
