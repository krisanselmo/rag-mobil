//! **R** of RAG: retrieve the ground-truth inventory of a set.

pub mod cache;
mod json_file;
pub mod parser;
pub mod playmodb;

use std::future::Future;

use crate::error::Result;
use crate::model::{SetInventory, SetNumber};

pub use cache::Cached;
pub use json_file::JsonFile;
pub use playmodb::PlaymoDb;

/// A source of set inventories: PlaymoDB (optionally behind the on-disk
/// [`Cached`]), or a saved JSON file. Another database can be plugged in
/// behind this trait.
pub trait InventorySource {
    fn fetch(&self, set: &SetNumber) -> impl Future<Output = Result<SetInventory>> + Send;
}
