use serde::{Deserialize, Serialize};

use crate::error::{RagError, Result};

/// A validated Playmobil set number (e.g. `3666`, `70145`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SetNumber(String);

impl SetNumber {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        let valid = !trimmed.is_empty()
            && trimmed.len() <= 8
            && trimmed.chars().all(|c| c.is_ascii_digit());
        if valid {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(RagError::InvalidSetNumber(raw.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SetNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One line of a set inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    /// PlaymoDB part number, e.g. `30 00 4950`.
    pub number: String,
    pub name: String,
    pub color: Option<String>,
    /// `None` when PlaymoDB lists the part without a count.
    pub quantity: Option<u32>,
    /// PlaymoDB category (`Klicky`, `Animal`, `Furniture`, ...).
    pub category: String,
    /// Sub-components listed under the part (e.g. a figure's wig).
    pub components: Vec<String>,
}

impl Part {
    pub fn is_figure(&self) -> bool {
        self.category.eq_ignore_ascii_case(CATEGORY_FIGURES)
    }
}

/// PlaymoDB's category for the figures ("Klickys").
pub const CATEGORY_FIGURES: &str = "Klicky";
pub const CATEGORY_UNKNOWN: &str = "Other";

/// The retrieved document: everything we know about a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetInventory {
    pub set_number: SetNumber,
    pub name: String,
    pub theme: Option<String>,
    pub focus: Option<String>,
    pub release_date: Option<String>,
    pub parts: Vec<Part>,
    pub source_url: String,
}

impl SetInventory {
    pub fn figures(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(|p| p.is_figure())
    }
}
