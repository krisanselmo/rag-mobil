use serde::{Deserialize, Serialize};

use crate::error::{RagError, Result};

/// A validated Playmobil set number as PlaymoDB spells it: `3666`, `70145`,
/// `3258a` (a revised edition), `3110x`, `2016ah`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SetNumber(String);

impl SetNumber {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim().to_ascii_lowercase();
        let digits = trimmed.trim_end_matches(|c: char| c.is_ascii_lowercase());
        let valid = (1..=8).contains(&digits.len())
            && digits.chars().all(|c| c.is_ascii_digit())
            && trimmed.len() - digits.len() <= 2;
        if valid {
            Ok(Self(trimmed))
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
    /// Absolute URL of a picture of the part; not read by the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The name the child gave this figure or piece (\"Coco\" for a parrot):
    /// every story calls it that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
}

impl Part {
    /// A figure (a character): category `Klicky` (PlaymoDB) or `figure`, any case.
    pub fn is_figure(&self) -> bool {
        [CATEGORY_FIGURES, CATEGORY_FIGURE]
            .iter()
            .any(|c| self.category.eq_ignore_ascii_case(c))
    }
}

/// A part reference reduced to its digits, so that `30 07 640`, `3007640`
/// and `ref. 30 07 640` compare equal.
pub fn reference_key(reference: &str) -> String {
    reference.chars().filter(char::is_ascii_digit).collect()
}

/// A part missing from the real box (lost, broken): every copy of a
/// reference (`30 07 640`), or some of them (`30 07 640:2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPart {
    pub reference: String,
    /// Copies missing; `None` = all of them.
    pub count: Option<u32>,
}

impl MissingPart {
    pub fn parse(raw: &str) -> Result<Self> {
        let invalid = || RagError::InvalidMissingPart(raw.to_owned());
        let (reference, count) = match raw.rsplit_once(':') {
            Some((reference, count)) => {
                let count: u32 = count.trim().parse().map_err(|_| invalid())?;
                if count == 0 {
                    return Err(invalid());
                }
                (reference, Some(count))
            }
            None => (raw, None),
        };
        Ok(Self {
            reference: parse_reference(reference).ok_or_else(invalid)?,
            count,
        })
    }
}

/// A reference as typed by hand: digits and spaces, `ref.` allowed in front.
fn parse_reference(raw: &str) -> Option<String> {
    let reference = raw.trim();
    let reference = reference
        .strip_prefix("ref.")
        .or_else(|| reference.strip_prefix("réf."))
        .unwrap_or(reference)
        .trim();
    let valid = !reference.is_empty()
        && reference
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace());
    valid.then(|| reference.to_owned())
}

/// The name the child gave a figure or piece, by its reference:
/// `30 06 368=Coco`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPart {
    pub reference: String,
    pub nickname: String,
}

impl NamedPart {
    pub fn parse(raw: &str) -> Result<Self> {
        let invalid = || RagError::InvalidNickname(raw.to_owned());
        let (reference, nickname) = raw.split_once('=').ok_or_else(invalid)?;
        let nickname = nickname.trim();
        // Quoted in the prompt (`named "Coco"`): a quote or a line break would
        // break out of it.
        if nickname.is_empty() || nickname.contains(['"', '\n', '\r']) {
            return Err(invalid());
        }
        Ok(Self {
            reference: parse_reference(reference).ok_or_else(invalid)?,
            nickname: nickname.to_owned(),
        })
    }
}

/// PlaymoDB's category for the figures ("Klickys").
pub const CATEGORY_FIGURES: &str = "Klicky";
/// Source-neutral category for the figures.
pub const CATEGORY_FIGURE: &str = "figure";
pub const CATEGORY_UNKNOWN: &str = "Other";

/// The retrieved document: everything we know about a set. Its JSON form is
/// the inventory format v1 (`docs/inventory-format.md`,
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

    /// Takes out the parts missing from the real boxes, so that the story
    /// never relies on them. Copies are taken from the boxes in order. Returns
    /// one line per missing part, saying what was done.
    pub fn remove_missing(&mut self, missing: &[MissingPart]) -> Vec<String> {
        let mut report = Vec::with_capacity(missing.len());
        for wanted in missing {
            let key = reference_key(&wanted.reference);
            let held = |p: &Part| reference_key(&p.number) == key;
            let Some(part) = self.sets.iter().flat_map(|s| &s.parts).find(|p| held(p)) else {
                report.push(format!("{}: not in the boxes, ignored", wanted.reference));
                continue;
            };
            let name = format!("{} ({})", part.number, part.name);
            let Some(mut left_to_remove) = wanted.count else {
                for set in &mut self.sets {
                    set.parts.retain(|p| !held(p));
                }
                report.push(format!("{name}: removed"));
                continue;
            };

            let mut unknown = false;
            let mut removed = 0;
            for set in &mut self.sets {
                for part in set.parts.iter_mut().filter(|p| held(p)) {
                    match part.quantity {
                        None => unknown = true,
                        Some(q) => {
                            let taken = q.min(left_to_remove);
                            part.quantity = Some(q - taken);
                            left_to_remove -= taken;
                            removed += taken;
                        }
                    }
                }
                set.parts.retain(|p| !(held(p) && p.quantity == Some(0)));
            }
            let left: u32 = self
                .sets
                .iter()
                .flat_map(|s| &s.parts)
                .filter(|p| held(p))
                .filter_map(|p| p.quantity)
                .sum();
            report.push(match (removed, unknown) {
                (0, true) => {
                    format!("{name}: unknown quantity, kept (without :N to remove it)")
                }
                _ if left == 0 && !unknown => format!("{name}: removed"),
                _ => format!("{name}: {removed} fewer, {left} left"),
            });
        }
        report
    }

    /// Gives the child's names to their pieces, every copy in every box,
    /// over any name the inventory had. Returns one line per name, saying
    /// what was done.
    pub fn name_parts(&mut self, names: &[NamedPart]) -> Vec<String> {
        let mut report = Vec::with_capacity(names.len());
        for named in names {
            let key = reference_key(&named.reference);
            let mut parts = self
                .sets
                .iter_mut()
                .flat_map(|s| &mut s.parts)
                .filter(|p| reference_key(&p.number) == key)
                .peekable();
            let Some(first) = parts.peek() else {
                report.push(format!("{}: not in the boxes, ignored", named.reference));
                continue;
            };
            let mut line = format!("{} ({}): \"{}\"", first.number, first.name, named.nickname);
            if let Some(old) = first.nickname.as_ref().filter(|n| **n != named.nickname) {
                line.push_str(&format!(", instead of \"{old}\""));
            }
            for part in parts {
                part.nickname = Some(named.nickname.clone());
            }
            report.push(line);
        }
        report
    }

    /// `3666 "Kings Large Castle", 5135 "Pirates Ship"`.
    pub fn label(&self) -> String {
        self.sets
            .iter()
            .map(|s| format!("{} \"{}\"", s.set_number, s.name))
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
            image: None,
            nickname: None,
        }
    }

    #[test]
    fn parses_set_numbers() {
        let parse = |raw| SetNumber::parse(raw).unwrap();
        assert_eq!(parse(" 3666 ").as_str(), "3666");
        assert_eq!(parse("3258a").as_str(), "3258a");
        assert_eq!(parse("3258A").as_str(), "3258a");
        assert_eq!(parse("2016ah").as_str(), "2016ah");
        for raw in ["", "a", "abc", "3258abc", "32a58", "123456789"] {
            assert!(SetNumber::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn parses_missing_parts() {
        let parse = |raw| MissingPart::parse(raw).unwrap();
        assert_eq!(
            parse(" 30 07 640 "),
            MissingPart {
                reference: "30 07 640".into(),
                count: None
            }
        );
        assert_eq!(parse("30 07 640:2").count, Some(2));
        assert_eq!(parse("ref. 30 07 640 : 2").reference, "30 07 640");
        assert_eq!(parse("réf. 30 07 640").reference, "30 07 640");
        for raw in ["", "abc", "30 07 640:0", "30 07 640:x", ":2"] {
            assert!(MissingPart::parse(raw).is_err(), "{raw}");
        }
    }

    fn numbered(number: &str, name: &str, quantity: Option<u32>) -> Part {
        Part {
            number: number.into(),
            name: name.into(),
            quantity,
            ..part("Tool")
        }
    }

    fn boxes() -> Collection {
        let set = |number: &str, parts| SetInventory {
            set_number: SetNumber::parse(number).unwrap(),
            name: "x".into(),
            theme: None,
            focus: None,
            release_date: None,
            parts,
            source_url: "test".into(),
        };
        Collection::new(vec![
            set(
                "3666",
                vec![
                    numbered("30 07 640", "Halberd", Some(4)),
                    numbered("30 06 368", "Parrot", Some(1)),
                    numbered("30 08 000", "Flower", None),
                ],
            ),
            set("5135", vec![numbered("30 07 640", "Halberd", Some(2))]),
        ])
    }

    fn quantities(collection: &Collection, number: &str) -> Vec<Option<u32>> {
        collection
            .parts()
            .filter(|(_, p)| p.number == number)
            .map(|(_, p)| p.quantity)
            .collect()
    }

    #[test]
    fn parses_nicknames() {
        let parse = |raw| NamedPart::parse(raw).unwrap();
        assert_eq!(
            parse(" 30 06 368 = Coco "),
            NamedPart {
                reference: "30 06 368".into(),
                nickname: "Coco".into()
            }
        );
        assert_eq!(parse("ref. 3006368=Captain Hook").nickname, "Captain Hook");
        // Only the first = separates: the name may hold one.
        assert_eq!(parse("30 06 368=A=B").nickname, "A=B");
        for raw in [
            "",
            "30 06 368",
            "30 06 368=",
            "=Coco",
            "abc=Coco",
            "1=Co\"co",
        ] {
            assert!(NamedPart::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn nicknames_go_to_every_copy() {
        let mut boxes = boxes();
        boxes.sets[0].parts[1].nickname = Some("Polly".into());
        let named = |reference: &str, nickname: &str| NamedPart {
            reference: reference.into(),
            nickname: nickname.into(),
        };
        let report = boxes.name_parts(&[
            named("3007640", "Pike"),
            named("30 06 368", "Coco"),
            named("30 99 999", "Ghost"),
        ]);
        assert_eq!(
            report,
            [
                "30 07 640 (Halberd): \"Pike\"",
                "30 06 368 (Parrot): \"Coco\", instead of \"Polly\"",
                "30 99 999: not in the boxes, ignored",
            ]
        );
        let nicknames: Vec<_> = boxes.parts().map(|(_, p)| p.nickname.as_deref()).collect();
        assert_eq!(nicknames, [Some("Pike"), Some("Coco"), None, Some("Pike")]);
    }

    #[test]
    fn missing_parts_leave_the_boxes() {
        let mut boxes = boxes();
        let missing: Vec<MissingPart> = ["3006368", "30 07 640:5", "30 08 000:1", "99 99 999"]
            .into_iter()
            .map(|raw| MissingPart::parse(raw).unwrap())
            .collect();
        let report = boxes.remove_missing(&missing);
        assert_eq!(
            report,
            [
                "30 06 368 (Parrot): removed",
                "30 07 640 (Halberd): 5 fewer, 1 left",
                "30 08 000 (Flower): unknown quantity, kept (without :N to remove it)",
                "99 99 999: not in the boxes, ignored",
            ]
        );
        assert!(quantities(&boxes, "30 06 368").is_empty());
        // Taken from the first box, then the second.
        assert_eq!(quantities(&boxes, "30 07 640"), [Some(1)]);
        assert_eq!(
            boxes.sets[0].parts.len(),
            1,
            "4 of 3666 gone, 1 of 5135 left"
        );
        assert_eq!(quantities(&boxes, "30 08 000"), [None]);

        let mut boxes = self::boxes();
        let all = [MissingPart::parse("30 07 640:9").unwrap()];
        assert_eq!(boxes.remove_missing(&all), ["30 07 640 (Halberd): removed"]);
        assert!(quantities(&boxes, "30 07 640").is_empty());
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
