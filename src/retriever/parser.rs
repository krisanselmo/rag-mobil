//! Pure HTML parsing for PlaymoDB pages. No I/O here, so it is fully unit-testable.
//!
//! Two pages are combined:
//! - `showinv.pl` gives the set metadata (name, theme, release date) and the
//!   *category* of each part (parts are grouped under `<h4>` headings);
//! - `listinv.pl` gives a tab-separated `<pre>` table with *quantities*
//!   and sub-components (wigs, hats... attached to figures).

use std::collections::HashMap;

use scraper::{ElementRef, Html, Selector};

use crate::model::{CATEGORY_UNKNOWN, Part};

/// What `showinv.pl` tells us.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShowInvPage {
    pub found: bool,
    pub name: Option<String>,
    pub theme: Option<String>,
    pub focus: Option<String>,
    pub release_date: Option<String>,
    /// Parts in page order, with category but without quantity.
    pub parts: Vec<Part>,
}

/// One row of the `listinv.pl` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInvRow {
    pub number: String,
    pub quantity: Option<u32>,
    pub name: String,
    pub color: Option<String>,
    pub components: Vec<String>,
}

const NOT_FOUND_MARKER: &str = "does not appear in the database";
const PART_LINK_PREFIX: &str = "showpart.pl?partnum=";

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("static CSS selector is valid")
}

/// Collapses whitespace (incl. `&nbsp;`) and trims.
fn clean(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c == '\u{a0}')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn element_text(el: ElementRef<'_>) -> String {
    clean(&el.text().collect::<String>())
}

/// Splits `"Horse (old-style) (brown)"` into `("Horse (old-style)", Some("brown"))`.
fn split_trailing_color(label: &str) -> (String, Option<String>) {
    let label = label.trim();
    if let Some(body) = label.strip_suffix(')')
        && let Some(open) = body.rfind(" (")
    {
        let color = body[open + 2..].trim();
        if !color.is_empty() && color != "n/a" {
            return (body[..open].trim().to_owned(), Some(color.to_owned()));
        }
        return (body[..open].trim().to_owned(), None);
    }
    (label.to_owned(), None)
}

/// Splits `"Window frame - brown"` into `("Window frame", Some("brown"))`.
fn split_dash_color(label: &str) -> (String, Option<String>) {
    match label.rsplit_once(" - ") {
        Some((name, color)) if !color.trim().is_empty() && color.trim() != "n/a" => {
            (name.trim().to_owned(), Some(color.trim().to_owned()))
        }
        Some((name, _)) => (name.trim().to_owned(), None),
        None => (label.trim().to_owned(), None),
    }
}

pub fn parse_showinv(html: &str) -> ShowInvPage {
    let doc = Html::parse_document(html);

    if doc
        .root_element()
        .text()
        .any(|t| t.contains(NOT_FOUND_MARKER))
    {
        return ShowInvPage::default();
    }

    let mut page = ShowInvPage {
        found: true,
        ..Default::default()
    };

    // The first <h2> is "PlaymoDB Set Inventory - N", the second is the set name.
    page.name = doc
        .select(&selector("h2"))
        .map(element_text)
        .find(|t| !t.is_empty() && !t.starts_with("PlaymoDB"));

    for link in doc.select(&selector("a[href^='sets.pl?theme=']")) {
        let href = link.value().attr("href").unwrap_or_default();
        let text = element_text(link);
        if href.contains("focus=") {
            page.focus.get_or_insert(text);
        } else {
            page.theme.get_or_insert(text);
        }
    }

    page.release_date = doc
        .select(&selector("div.setdate"))
        .map(element_text)
        .find_map(|t| {
            t.strip_prefix("First released:")
                .map(|d| d.trim().to_owned())
        });

    // Walk the document in order: an <h4> opens a category, then parts follow
    // in one of two layouts:
    // - old sets: `<a href="showpart.pl?...">NUM</a> label (color)<br/>`
    // - newer sets: `<div class="lineup"><div class="listpartnumber">NUM</div>
    //   <div class="listpartname">label</div>...<small>color</small></div>`
    let mut category = CATEGORY_UNKNOWN.to_owned();
    for node in doc.root_element().descendants() {
        let Some(el) = ElementRef::wrap(node) else {
            continue;
        };
        let class = el.value().attr("class").unwrap_or_default();
        match el.value().name() {
            // Spare/alternate parts, explicitly "not included in inventory".
            "div" if class == "extra_parts" => break,
            "h4" => {
                if let Some(first) = el.text().map(clean).find(|t| !t.is_empty()) {
                    category = first;
                }
            }
            "div" if class == "lineup" => {
                let first_text = |css: &str| el.select(&selector(css)).next().map(element_text);
                let Some(number) = first_text(".listpartnumber") else {
                    continue;
                };
                page.parts.push(Part {
                    number: normalize_part_number(&number),
                    name: first_text(".listpartname").unwrap_or_default(),
                    color: first_text("small").filter(|c| !c.is_empty() && c != "n/a"),
                    quantity: None,
                    category: category.clone(),
                    components: Vec::new(),
                });
            }
            "a" if el
                .value()
                .attr("href")
                .is_some_and(|h| h.starts_with(PART_LINK_PREFIX)) =>
            {
                let label = node
                    .next_sibling()
                    .and_then(|n| n.value().as_text().map(|t| clean(t)))
                    .unwrap_or_default();
                let (name, color) = split_trailing_color(&label);
                page.parts.push(Part {
                    number: normalize_part_number(&element_text(el)),
                    name,
                    color,
                    quantity: None,
                    category: category.clone(),
                    components: Vec::new(),
                });
            }
            _ => {}
        }
    }

    page
}

pub fn parse_listinv(html: &str) -> Vec<ListInvRow> {
    let doc = Html::parse_document(html);
    let Some(pre) = doc.select(&selector("pre")).next() else {
        return Vec::new();
    };
    let text: String = pre.text().collect();

    let mut rows: Vec<ListInvRow> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with("Part number") || line.starts_with("---") {
            continue;
        }

        // Sub-component: "\t\t\t| 30 09 6200 - Wig (hair), classic male - grey"
        if line.starts_with('\t') {
            if let Some(component) = line.trim().strip_prefix('|')
                && let Some(last) = rows.last_mut()
            {
                // Drop the leading part number, keep the human-readable label.
                let component = component.trim();
                let label = component
                    .split_once(" - ")
                    .map_or(component, |(_, rest)| rest);
                last.components.push(clean(label));
            }
            continue;
        }

        let mut cols = line.splitn(3, '\t');
        let (Some(number), Some(count), Some(label)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let quantity = count.trim().trim_end_matches('x').parse::<u32>().ok();
        let (name, color) = split_dash_color(&clean(label));
        rows.push(ListInvRow {
            number: normalize_part_number(number),
            quantity,
            name,
            color,
            components: Vec::new(),
        });
    }
    rows
}

/// `"30 00 4950"` and `"30 00 495"` refer to the same part: PlaymoDB displays
/// 7 digits in link text but 8 (trailing 0) in URLs and the text list. We keep 7.
fn normalize_part_number(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = if digits.len() == 8 && digits.ends_with('0') {
        &digits[..7]
    } else {
        &digits[..]
    };
    match digits.len() {
        7 => format!("{} {} {}", &digits[..2], &digits[2..4], &digits[4..]),
        _ => clean(raw),
    }
}

/// Merges both pages: `listinv` (quantities, authoritative list) enriched
/// with the categories from `showinv`. Falls back to `showinv` alone if the
/// text list is empty.
///
/// Rows without a count are alternate part numbers of a part already counted
/// under another number (e.g. Throne `30 02 794` vs `30 08 698` ×2): they are
/// dropped so the LLM doesn't see the same object twice.
pub fn merge(show: &ShowInvPage, list: Vec<ListInvRow>) -> Vec<Part> {
    if list.is_empty() {
        return show.parts.clone();
    }
    let has_counts = list.iter().any(|r| r.quantity.is_some());
    let categories: HashMap<&str, &str> = show
        .parts
        .iter()
        .map(|p| (p.number.as_str(), p.category.as_str()))
        .collect();

    list.into_iter()
        .filter(|row| !has_counts || row.quantity.is_some())
        .map(|row| Part {
            category: categories
                .get(row.number.as_str())
                .copied()
                .unwrap_or(CATEGORY_UNKNOWN)
                .to_owned(),
            number: row.number,
            name: row.name,
            color: row.color,
            quantity: row.quantity,
            components: row.components,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWINV: &str = include_str!("../../tests/fixtures/showinv_3666.html");
    const LISTINV: &str = include_str!("../../tests/fixtures/listinv_3666.html");
    const SHOWINV_MISSING: &str = include_str!("../../tests/fixtures/showinv_missing.html");
    const SHOWINV_NEW_LAYOUT: &str = include_str!("../../tests/fixtures/showinv_70145.html");

    #[test]
    fn parses_set_metadata() {
        let page = parse_showinv(SHOWINV);
        assert!(page.found);
        assert_eq!(page.name.as_deref(), Some("Kings Large Castle"));
        assert_eq!(page.theme.as_deref(), Some("Knights"));
        assert_eq!(page.focus.as_deref(), Some("Building"));
        assert_eq!(page.release_date.as_deref(), Some("1993/09"));
    }

    #[test]
    fn parses_categories() {
        let page = parse_showinv(SHOWINV);
        let horse = page
            .parts
            .iter()
            .find(|p| p.name.starts_with("Horse"))
            .unwrap();
        assert_eq!(horse.category, "Animal");
        assert_eq!(horse.number, "30 65 622");
        assert_eq!(horse.name, "Horse (old-style)");
        assert_eq!(horse.color.as_deref(), Some("brown"));

        let trumpeter = page
            .parts
            .iter()
            .find(|p| p.name == "Castle Trumpeter")
            .unwrap();
        assert_eq!(trumpeter.category, "Klicky");
        assert_eq!(trumpeter.color, None);
    }

    #[test]
    fn skips_extra_parts_section() {
        let page = parse_showinv(SHOWINV);
        assert!(page.parts.iter().all(|p| p.name != "Throne"));
    }

    #[test]
    fn parses_lineup_layout() {
        let page = parse_showinv(SHOWINV_NEW_LAYOUT);
        assert_eq!(page.name.as_deref(), Some("Diving Scooter"));
        assert_eq!(page.parts.len(), 2);
        let fish = &page.parts[0];
        assert_eq!(fish.category, "Animal");
        assert_eq!(fish.number, "30 66 6973");
        assert_eq!(fish.name, "Fish, sword- (swordfish), small");
        assert_eq!(fish.color.as_deref(), Some("dark blue"));
        let figure = &page.parts[1];
        assert!(figure.is_figure());
        assert_eq!(figure.color, None);
    }

    #[test]
    fn detects_missing_set() {
        assert!(!parse_showinv(SHOWINV_MISSING).found);
    }

    #[test]
    fn parses_quantities_and_components() {
        let rows = parse_listinv(LISTINV);
        let trumpeter = rows.iter().find(|r| r.name == "Castle Trumpeter").unwrap();
        assert_eq!(trumpeter.quantity, Some(1));
        assert_eq!(
            trumpeter.components,
            vec!["Wig (hair), classic male - grey"]
        );

        let toast = rows.iter().find(|r| r.name.starts_with("Filler")).unwrap();
        assert_eq!(toast.quantity, Some(26));
        assert_eq!(toast.color.as_deref(), Some("brown"));

        let throne = rows.iter().find(|r| r.name == "Throne").unwrap();
        assert_eq!(throne.quantity, None);
    }

    #[test]
    fn merge_attaches_categories() {
        let parts = merge(&parse_showinv(SHOWINV), parse_listinv(LISTINV));
        let trumpeter = parts.iter().find(|p| p.name == "Castle Trumpeter").unwrap();
        assert_eq!(trumpeter.category, "Klicky");
        assert!(trumpeter.is_figure());
        // Count-less alternate number: dropped.
        assert!(parts.iter().all(|p| p.name != "Throne"));
        // Counted but absent from the category page.
        let keys = parts.iter().find(|p| p.name == "Keys on keyring").unwrap();
        assert_eq!(keys.category, "Tool");
    }

    #[test]
    fn normalizes_part_numbers() {
        assert_eq!(normalize_part_number("30 00 4950"), "30 00 495");
        assert_eq!(normalize_part_number("30 00 495"), "30 00 495");
    }
}
