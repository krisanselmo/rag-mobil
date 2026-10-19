//! Turns a [`SetInventory`] into the grounding context given to the LLM.
//!
//! Figures come first (they become the characters), then the other parts
//! grouped by category. Categories with no narrative value (paper
//! instructions) are dropped unless `include_all` is set.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::model::{Collection, Part, SetInventory};

/// Categories that never matter for a story.
const NON_NARRATIVE_CATEGORIES: &[&str] = &["Paper"];

#[derive(Debug, Clone, Default)]
pub struct ContextOptions {
    /// Keep every category, including paper instructions.
    pub include_all: bool,
}

fn quantity_prefix(part: &Part) -> String {
    part.quantity
        .map_or_else(|| "?× ".to_owned(), |q| format!("{q}× "))
}

fn render_part(out: &mut String, part: &Part) {
    let _ = write!(out, "- {}{}", quantity_prefix(part), part.name);
    if let Some(color) = &part.color {
        let _ = write!(out, " ({color})");
    }
    let _ = write!(out, " [réf. {}]", part.number);
    if !part.components.is_empty() {
        let _ = write!(out, " — avec : {}", part.components.join(" ; "));
    }
    out.push('\n');
}

/// Escapes characters that would break the pseudo-XML attribute syntax.
fn attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// The grounding context: one `<set>` block per box.
pub fn build_context(collection: &Collection, options: &ContextOptions) -> String {
    collection
        .sets
        .iter()
        .map(|set| set_context(set, options))
        .collect::<Vec<_>>()
        .join("\n")
}

fn set_context(inventory: &SetInventory, options: &ContextOptions) -> String {
    let keep = |p: &&Part| {
        options.include_all
            || !NON_NARRATIVE_CATEGORIES
                .iter()
                .any(|c| p.category.eq_ignore_ascii_case(c))
    };

    let mut out = String::new();
    let _ = write!(
        out,
        "<set numero=\"{}\" nom=\"{}\"",
        inventory.set_number,
        attr(&inventory.name)
    );
    if let Some(theme) = &inventory.theme {
        let _ = write!(out, " theme=\"{}\"", attr(theme));
    }
    if let Some(focus) = &inventory.focus {
        let _ = write!(out, " sous_theme=\"{}\"", attr(focus));
    }
    if let Some(date) = &inventory.release_date {
        let _ = write!(out, " sortie=\"{}\"", attr(date));
    }
    let _ = writeln!(out, " source=\"{}\">", attr(&inventory.source_url));

    out.push_str("<personnages>\n");
    let mut figures = inventory.figures().filter(keep).peekable();
    if figures.peek().is_none() {
        out.push_str("(aucune figurine listée dans l'inventaire)\n");
    }
    for part in figures {
        render_part(&mut out, part);
    }
    out.push_str("</personnages>\n");

    // BTreeMap: stable, alphabetical category order => deterministic prompt.
    let mut by_category: BTreeMap<&str, Vec<&Part>> = BTreeMap::new();
    for part in inventory
        .parts
        .iter()
        .filter(|p| !p.is_figure())
        .filter(keep)
    {
        by_category.entry(&part.category).or_default().push(part);
    }
    for (category, parts) in by_category {
        let _ = writeln!(out, "<pieces categorie=\"{}\">", attr(category));
        for part in parts {
            render_part(&mut out, part);
        }
        out.push_str("</pieces>\n");
    }
    out.push_str("</set>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SetNumber;

    fn part(name: &str, category: &str, quantity: Option<u32>) -> Part {
        Part {
            number: "30 00 000".into(),
            name: name.into(),
            color: Some("red".into()),
            quantity,
            category: category.into(),
            components: vec![],
            image: None,
        }
    }

    fn inventory() -> SetInventory {
        SetInventory {
            set_number: SetNumber::parse("3666").unwrap(),
            name: "Kings Large Castle".into(),
            theme: Some("Knights".into()),
            focus: None,
            release_date: Some("1993/09".into()),
            parts: vec![
                part("Halberd", "Tool", Some(4)),
                part("Castle Trumpeter", "Klicky", Some(1)),
                part("Instructions", "Paper", Some(1)),
                part("Throne", "Furniture", None),
            ],
            source_url: "https://playmodb.org/cgi-bin/showinv.pl?setnum=3666".into(),
        }
    }

    #[test]
    fn figures_come_first_and_paper_is_dropped() {
        let ctx = build_context(&inventory().into(), &ContextOptions::default());
        let figures = ctx.find("<personnages>").unwrap();
        let trumpeter = ctx.find("Castle Trumpeter").unwrap();
        let halberd = ctx.find("Halberd").unwrap();
        assert!(figures < trumpeter && trumpeter < halberd);
        assert!(ctx.contains("4× Halberd (red) [réf. 30 00 000]"));
        assert!(ctx.contains("?× Throne"));
        assert!(!ctx.contains("Instructions"));
    }

    #[test]
    fn include_all_keeps_paper() {
        let ctx = build_context(&inventory().into(), &ContextOptions { include_all: true });
        assert!(ctx.contains("Instructions"));
    }

    #[test]
    fn one_block_per_set() {
        let mut other = inventory();
        other.set_number = SetNumber::parse("5135").unwrap();
        other.name = "Pirates Ship".into();
        let ctx = build_context(
            &Collection::new(vec![inventory(), other]),
            &ContextOptions::default(),
        );
        assert_eq!(ctx.matches("<set numero=").count(), 2);
        assert!(ctx.contains("</set>\n<set numero=\"5135\" nom=\"Pirates Ship\""));
    }
}
