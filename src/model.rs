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
    /// Part reference, e.g. PlaymoDB's `30 00 4950`.
    pub number: String,
    pub name: String,
    pub color: Option<String>,
    /// `None` when the source lists the part without a count.
    pub quantity: Option<u32>,
    /// Free text (`Klicky`, `Animal`, `Furniture`, ...); see [`Part::is_figure`].
    pub category: String,
    /// Sub-components listed under the part (e.g. a figure's wig).
    pub components: Vec<String>,
}

impl Part {
    /// A figure (a character): category `Klicky` (PlaymoDB) or `figure`, any case.
    pub fn is_figure(&self) -> bool {
        [CATEGORY_FIGURES, CATEGORY_FIGURE]
            .iter()
            .any(|c| self.category.eq_ignore_ascii_case(c))
    }
}

/// PlaymoDB's category for the figures ("Klickys").
pub const CATEGORY_FIGURES: &str = "Klicky";
/// Source-neutral category for the figures.
pub const CATEGORY_FIGURE: &str = "figure";
pub const CATEGORY_UNKNOWN: &str = "Other";

/// The retrieved document: everything we know about a set. Its JSON form is
/// the inventory format v1 (`docs/format-inventaire.md`,
/// `schema/inventory-v1.schema.json`).
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

/// The boxes a story is played with: one or several sets, mixed together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub sets: Vec<SetInventory>,
}

impl Collection {
    pub fn new(sets: Vec<SetInventory>) -> Self {
        Self { sets }
    }

    pub fn is_single(&self) -> bool {
        self.sets.len() == 1
    }

    /// Every part, with the set it comes from.
    pub fn parts(&self) -> impl Iterator<Item = (&SetInventory, &Part)> {
        self.sets
            .iter()
            .flat_map(|set| set.parts.iter().map(move |part| (set, part)))
    }

    pub fn figures(&self) -> impl Iterator<Item = &Part> {
        self.sets.iter().flat_map(SetInventory::figures)
    }

    pub fn part_count(&self) -> usize {
        self.sets.iter().map(|s| s.parts.len()).sum()
    }

    /// `3666 « Kings Large Castle », 5135 « Pirates Ship »`.
    pub fn label(&self) -> String {
        self.sets
            .iter()
            .map(|s| format!("{} « {} »", s.set_number, s.name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl From<SetInventory> for Collection {
    fn from(set: SetInventory) -> Self {
        Self::new(vec![set])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(category: &str) -> Part {
        Part {
            number: "1".into(),
            name: "x".into(),
            color: None,
            quantity: None,
            category: category.into(),
            components: vec![],
        }
    }

    #[test]
    fn figures_are_klicky_or_figure() {
        for category in ["Klicky", "klicky", "KLICKY", "figure", "Figure", "FIGURE"] {
            assert!(part(category).is_figure(), "{category}");
        }
        for category in ["Animal", "Figures", "Klickys", "", "Other"] {
            assert!(!part(category).is_figure(), "{category}");
        }
    }
}
