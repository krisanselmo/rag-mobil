use std::time::Duration;

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::parser::{merge, parse_listinv, parse_showinv};
use crate::retriever::{InventorySource, USER_AGENT};

pub const DEFAULT_BASE_URL: &str = "https://playmodb.org/cgi-bin";

/// Scrapes PlaymoDB (two pages per set, fetched concurrently).
pub struct PlaymoDb {
    http: reqwest::Client,
    base_url: String,
    /// Wait before each retry of a transient failure.
    retry_delays: Vec<Duration>,
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
            retry_delays: [1, 3, 6].map(Duration::from_secs).to_vec(),
        })
    }

    /// Overrides the waits between retries (an empty list disables retries).
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    fn url(&self, script: &str, set: &SetNumber) -> String {
        format!("{}/{script}?setnum={set}", self.base_url)
    }

    /// GET with retries: PlaymoDB is a volunteer-run site on shared hosting
    /// and regularly answers 508 (resource limit) or other transient 5xx.
    async fn get(&self, url: &str) -> Result<String> {
        let mut attempt = 0;
        loop {
            let response = self.http.get(url).send().await?;
            let status = response.status();
            if status.is_success() {
                return Ok(response.text().await?);
            }
            let transient = status.is_server_error() || status.as_u16() == 429;
            if !transient || attempt >= self.retry_delays.len() {
                return Err(RagError::ScraperStatus {
                    url: url.to_owned(),
                    status: status.as_u16(),
                });
            }
            tokio::time::sleep(self.retry_delays[attempt]).await;
            attempt += 1;
        }
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
        // The same page with a picture of each part.
        let pics_url = format!("{show_url}&pics=on");
        let list_url = self.url("listinv.pl", set);
        let (show_html, list_html) = tokio::try_join!(self.get(&pics_url), self.get(&list_url))?;

        let show = parse_showinv(&show_html);
        if !show.found {
            return Err(RagError::SetNotFound(format!("{set} (absent de PlaymoDB)")));
        }
        let mut parts = merge(&show, parse_listinv(&list_html));
        let base = reqwest::Url::parse(&pics_url).ok();
        for part in &mut parts {
            part.image = part
                .image
                .take()
                .and_then(|src| Some(base.as_ref()?.join(&src).ok()?.to_string()));
        }
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
