//! **R** of RAG: retrieve the ground-truth inventory of a set.

pub mod parser;
pub mod playmodb;

use std::future::Future;

use crate::error::Result;
use crate::model::{SetInventory, SetNumber};

pub use playmodb::PlaymoDb;

/// A source of set inventories. PlaymoDB is the only implementation today;
/// a local cache or another database can be plugged in behind this trait.
pub trait InventorySource {
    fn fetch(&self, set: &SetNumber) -> impl Future<Output = Result<SetInventory>> + Send;
}
