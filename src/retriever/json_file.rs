use std::path::PathBuf;

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;

/// An inventory saved with `--inventory-json`: offline runs and tests,
/// without hitting PlaymoDB.
pub struct JsonFile(pub PathBuf);

impl InventorySource for JsonFile {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        let json = std::fs::read_to_string(&self.0).map_err(|source| RagError::Io {
            path: self.0.clone(),
            source,
        })?;
        let inventory: SetInventory = serde_json::from_str(&json)?;
        if &inventory.set_number != set {
            return Err(RagError::SetNotFound(format!(
                "{set} (le fichier {} contient le set {})",
                self.0.display(),
                inventory.set_number
            )));
        }
        Ok(inventory)
    }
}
