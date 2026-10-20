use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;

/// Inventories saved with `--inventory-json` (one set, or an array for
/// several): offline runs and tests, without hitting PlaymoDB.
pub struct JsonFile(pub PathBuf);

#[derive(Deserialize)]
#[serde(untagged)]
enum Saved {
    One(SetInventory),
    Many(Vec<SetInventory>),
}

impl InventorySource for JsonFile {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        let json = std::fs::read_to_string(&self.0).map_err(|source| RagError::Io {
            path: self.0.clone(),
            source,
        })?;
        let inventories = match serde_json::from_str(&json)? {
            Saved::One(inventory) => vec![inventory],
            Saved::Many(inventories) => inventories,
        };
        let available: Vec<String> = inventories
            .iter()
            .map(|i| i.set_number.to_string())
            .collect();
        inventories
            .into_iter()
            .find(|i| &i.set_number == set)
            .ok_or_else(|| {
                RagError::SetNotFound(format!(
                    "{set} (the file {} contains: {})",
                    self.0.display(),
                    available.join(", ")
                ))
            })
    }
}
