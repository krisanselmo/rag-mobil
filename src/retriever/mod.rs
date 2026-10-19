//! **R** of RAG: retrieve the ground-truth inventory of a set.

pub mod cache;
mod fallback;
mod http_json;
mod json_file;
pub mod parser;
pub mod playmodb;

use std::future::Future;

use crate::error::Result;
use crate::model::{SetInventory, SetNumber};

pub use cache::Cached;
pub use fallback::{Answered, Fallback};
pub use http_json::HttpJson;
pub use json_file::JsonFile;
pub use playmodb::PlaymoDb;

pub(crate) const USER_AGENT: &str = concat!(
    "rag-mobil/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/krisanselmo/rag-mobil)"
);

/// A source of set inventories: PlaymoDB (optionally behind the on-disk
/// [`Cached`]), a saved JSON file, or any HTTP service speaking the
/// inventory format v1 ([`HttpJson`]); [`Fallback`] chains two of them.
///
/// A source that doesn't know a set must answer
/// [`RagError::SetNotFound`](crate::RagError::SetNotFound), so that the
/// next source of a [`Fallback`] gets asked.
pub trait InventorySource {
    fn fetch(&self, set: &SetNumber) -> impl Future<Output = Result<SetInventory>> + Send;
}
