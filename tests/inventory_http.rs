//! `HttpJson` (alone and in a `Fallback`) against a local inventory service.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rag_mobil::retriever::{Answered, Fallback, HttpJson, InventorySource, JsonFile};
use rag_mobil::{RagError, SetInventory, SetNumber};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const INVENTORY_3799: &str = include_str!("e2e/3799/inventory.json");

/// Serves `(path, status, body)` routes, 404 for any other path, and
/// records the request heads.
async fn server(routes: Vec<(&'static str, u16, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let heads = Arc::new(Mutex::new(Vec::new()));
    let log = heads.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 4096];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            let head = String::from_utf8_lossy(&raw).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
            log.lock().unwrap().push(head);
            let (status, body) = routes.iter().find(|(p, _, _)| *p == path).map_or(
                (404, "{\"error\":\"not found\"}".to_owned()),
                |(_, s, b)| (*s, b.clone()),
            );
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (base, heads)
}

fn set(n: &str) -> SetNumber {
    SetNumber::parse(n).unwrap()
}

fn source(base: &str) -> HttpJson {
    HttpJson::new(format!("{base}/api/inventories/{{set}}")).unwrap()
}

/// The 3799 inventory renumbered, so that a test can tell where it came from.
fn renumbered(number: &str) -> String {
    let mut inventory: SetInventory = serde_json::from_str(INVENTORY_3799).unwrap();
    inventory.set_number = set(number);
    inventory.name = format!("Set {number} du service");
    serde_json::to_string(&inventory).unwrap()
}

fn saved_3799() -> JsonFile {
    JsonFile(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/3799/inventory.json"))
}

#[tokio::test]
async fn found_inventory_is_returned_as_is() {
    let (base, heads) = server(vec![(
        "/api/inventories/3799",
        200,
        INVENTORY_3799.to_owned(),
    )])
    .await;
    let inventory = source(&base).fetch(&set("3799")).await.unwrap();
    let expected: SetInventory = serde_json::from_str(INVENTORY_3799).unwrap();
    assert_eq!(inventory, expected);

    let head = heads.lock().unwrap()[0].to_lowercase();
    assert!(
        head.starts_with("get /api/inventories/3799 http/1.1"),
        "{head}"
    );
    assert!(head.contains("accept: application/json"), "{head}");
    assert!(head.contains("user-agent: rag-mobil/"), "{head}");
}

#[tokio::test]
async fn unknown_set_is_set_not_found() {
    let (base, _) = server(vec![]).await;
    let err = source(&base).fetch(&set("3799")).await.unwrap_err();
    let RagError::SetNotFound(message) = &err else {
        panic!("{err:?}");
    };
    assert!(message.contains("/api/inventories/3799"), "{message}");
}

#[tokio::test]
async fn unknown_set_falls_back_to_the_second_source() {
    let (base, _) = server(vec![("/api/inventories/70145", 200, renumbered("70145"))]).await;
    let chain = Fallback::new(source(&base), saved_3799());

    let (inventory, answered) = chain.fetch_traced(&set("3799")).await.unwrap();
    assert_eq!(answered, Answered::Second);
    assert_eq!(inventory.name, "Pirates Secret Island");

    let (inventory, answered) = chain.fetch_traced(&set("70145")).await.unwrap();
    assert_eq!(answered, Answered::First);
    assert_eq!(inventory.name, "Set 70145 du service");
}

#[tokio::test]
async fn server_error_does_not_fall_back() {
    let (base, _) = server(vec![("/api/inventories/3799", 500, "boom".to_owned())]).await;
    let chain = Fallback::new(source(&base), saved_3799());
    let err = chain.fetch(&set("3799")).await.unwrap_err();
    let RagError::SourceStatus { url, status } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(*status, 500);
    assert_eq!(url, &format!("{base}/api/inventories/3799"));
    assert!(err.to_string().contains("HTTP 500"), "{err}");
}

#[tokio::test]
async fn malformed_json_is_an_error() {
    let (base, _) = server(vec![
        (
            "/api/inventories/1",
            200,
            "<html>not JSON</html>".to_owned(),
        ),
        // Valid JSON, but a required field (`parts`) is missing.
        (
            "/api/inventories/2",
            200,
            r#"{"set_number":"2","name":"x","source_url":"y"}"#.to_owned(),
        ),
    ])
    .await;
    for n in ["1", "2"] {
        let err = source(&base).fetch(&set(n)).await.unwrap_err();
        assert!(
            matches!(&err, RagError::InvalidInventory { url, .. } if url.ends_with(&format!("/{n}"))),
            "{err:?}"
        );
    }
}

#[tokio::test]
async fn another_set_in_the_body_is_an_error() {
    let (base, _) = server(vec![(
        "/api/inventories/3666",
        200,
        INVENTORY_3799.to_owned(),
    )])
    .await;
    let chain = Fallback::new(source(&base), saved_3799());
    let err = chain.fetch(&set("3666")).await.unwrap_err();
    let RagError::InvalidInventory { reason, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(reason, "set 3666 requested, set 3799 received");
}

#[tokio::test]
async fn empty_inventory_is_an_error() {
    let mut inventory: SetInventory = serde_json::from_str(INVENTORY_3799).unwrap();
    inventory.parts.clear();
    let body = serde_json::to_string(&inventory).unwrap();
    let (base, _) = server(vec![("/api/inventories/3799", 200, body)]).await;
    let err = source(&base).fetch(&set("3799")).await.unwrap_err();
    assert!(matches!(err, RagError::EmptyInventory(_)), "{err:?}");
}

#[test]
fn template_without_placeholder_is_rejected() {
    for bad in [
        "http://127.0.0.1:8096/api/inventories/",
        "not an address {set}",
    ] {
        assert!(matches!(
            HttpJson::new(bad),
            Err(RagError::InvalidInventoryUrl(_))
        ));
    }
    assert!(HttpJson::new("http://127.0.0.1:8096/api/inventories/{set}").is_ok());
}
