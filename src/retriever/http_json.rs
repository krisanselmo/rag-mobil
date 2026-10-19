use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::ACCEPT;

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::{InventorySource, USER_AGENT};

/// Any HTTP service answering `GET <template with {set} replaced>` with one
/// inventory in format v1 (`docs/format-inventaire.md`), and 404 for a set
/// it doesn't know.
pub struct HttpJson {
    http: reqwest::Client,
    template: String,
}

const PLACEHOLDER: &str = "{set}";

impl HttpJson {
    /// `template` is an http(s) URL containing `{set}`, e.g.
    /// `http://localhost:8096/api/inventories/{set}`.
    pub fn new(template: impl Into<String>) -> Result<Self> {
        let template = template.into().trim().to_owned();
        let invalid = || RagError::InvalidInventoryUrl(template.clone());
        if !template.contains(PLACEHOLDER) {
            return Err(invalid());
        }
        let probe =
            reqwest::Url::parse(&template.replace(PLACEHOLDER, "0")).map_err(|_| invalid())?;
        if !matches!(probe.scheme(), "http" | "https") || probe.host_str().is_none() {
            return Err(invalid());
        }
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, template })
    }

    pub fn url(&self, set: &SetNumber) -> String {
        self.template.replace(PLACEHOLDER, set.as_str())
    }

    /// `host[:port]` of the service, to tell the user where a set came from.
    pub fn host(&self) -> String {
        let url = reqwest::Url::parse(&self.url(&SetNumber::parse("0").expect("valid")))
            .expect("checked in new()");
        match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            (None, _) => self.template.clone(),
        }
    }
}

impl InventorySource for HttpJson {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        let url = self.url(set);
        let response = self
            .http
            .get(&url)
            .header(ACCEPT, "application/json")
            .send()
            .await?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(RagError::SetNotFound(format!("{set} (absent de {url})")));
        }
        if !status.is_success() {
            return Err(RagError::SourceStatus {
                url,
                status: status.as_u16(),
            });
        }
        let body = response.text().await?;
        let inventory: SetInventory =
            serde_json::from_str(&body).map_err(|err| RagError::InvalidInventory {
                url: url.clone(),
                reason: format!("JSON au format v1 attendu ({err})"),
            })?;
        if &inventory.set_number != set {
            return Err(RagError::InvalidInventory {
                url,
                reason: format!("set {set} demandé, set {} reçu", inventory.set_number),
            });
        }
        if inventory.parts.is_empty() {
            return Err(RagError::EmptyInventory(format!("{set} ({url})")));
        }
        Ok(inventory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_must_be_an_http_url_with_a_placeholder() {
        for bad in [
            "http://localhost:8096/api/inventories/",
            "http://localhost:8096/api/inventories/{SET}",
            "localhost:8096/{set}",
            "ftp://example.org/{set}",
            "file:///tmp/{set}.json",
            "",
        ] {
            assert!(
                matches!(HttpJson::new(bad), Err(RagError::InvalidInventoryUrl(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn placeholder_is_replaced_everywhere() {
        let source = HttpJson::new(" https://example.org/sets/{set}?id={set} ").unwrap();
        let set = SetNumber::parse("3666").unwrap();
        assert_eq!(source.url(&set), "https://example.org/sets/3666?id=3666");
        assert_eq!(source.host(), "example.org");
        let local = HttpJson::new("http://localhost:8096/api/inventories/{set}").unwrap();
        assert_eq!(local.host(), "localhost:8096");
    }
}
