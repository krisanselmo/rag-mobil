//! The scraper retries PlaymoDB's transient errors (508 resource limit...).

use std::time::Duration;

use rag_mobil::retriever::{InventorySource, PlaymoDb};
use rag_mobil::{RagError, SetNumber};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SHOWINV: &str = include_str!("fixtures/showinv_3666.html");
const LISTINV: &str = include_str!("fixtures/listinv_3666.html");

/// Answers each request with the next scripted status; the body depends on
/// the requested page.
async fn server(statuses: Vec<u16>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/cgi-bin", listener.local_addr().unwrap());
    tokio::spawn(async move {
        for status in statuses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let body = match status {
                200 if request.contains("listinv.pl") => LISTINV,
                200 => SHOWINV,
                _ => "Resource Limit Is Reached",
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: text/html\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    url
}

fn scraper(url: String, retries: usize) -> PlaymoDb {
    PlaymoDb::new(url)
        .unwrap()
        .with_retry_delays(vec![Duration::from_millis(10); retries])
}

#[tokio::test]
async fn retries_transient_errors() {
    // The two pages are fetched concurrently: 4 answers cover 2 failures + 2 successes.
    let url = server(vec![508, 508, 200, 200]).await;
    let inventory = scraper(url, 2)
        .fetch(&SetNumber::parse("3666").unwrap())
        .await
        .unwrap();
    assert_eq!(inventory.name, "Kings Large Castle");
}

#[tokio::test]
async fn gives_up_after_the_last_retry() {
    let url = server(vec![508; 6]).await;
    let err = scraper(url, 1)
        .fetch(&SetNumber::parse("3666").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, RagError::ScraperStatus { status: 508, .. }));
}
