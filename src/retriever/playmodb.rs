use std::time::Duration;

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;
use crate::retriever::parser::{merge, parse_listinv, parse_showinv};

pub const DEFAULT_BASE_URL: &str = "https://playmodb.org/cgi-bin";

const USER_AGENT: &str = concat!(
    "rag-mobil/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/krisanselmo/rag-mobil)"
);

/// Scrapes PlaymoDB (two pages per set, fetched concurrently).
pub struct PlaymoDb {
    http: reqwest::Client,
    base_url: String,
}

impl PlaymoDb {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        })
    }

    fn url(&self, script: &str, set: &SetNumber) -> String {
        format!("{}/{script}?setnum={set}", self.base_url)
    }

    async fn get(&self, url: &str) -> Result<String> {
        let response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(RagError::ScraperStatus {
                url: url.to_owned(),
                status: status.as_u16(),
            });
        }
        Ok(response.text().await?)
    }
}

impl Default for PlaymoDb {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL).expect("default HTTP client builds")
    }
}

impl InventorySource for PlaymoDb {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        let show_url = self.url("showinv.pl", set);
        let list_url = self.url("listinv.pl", set);
        let (show_html, list_html) = tokio::try_join!(self.get(&show_url), self.get(&list_url))?;

        let show = parse_showinv(&show_html);
        if !show.found {
            return Err(RagError::SetNotFound(set.to_string()));
        }
        let parts = merge(&show, parse_listinv(&list_html));
        if parts.is_empty() {
            return Err(RagError::EmptyInventory(set.to_string()));
        }

        Ok(SetInventory {
            set_number: set.clone(),
            name: show.name.unwrap_or_else(|| format!("Set {set}")),
            theme: show.theme,
            focus: show.focus,
            release_date: show.release_date,
            parts,
            source_url: show_url,
        })
    }
}
