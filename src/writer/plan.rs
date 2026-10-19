//! The story plan (Re3's "Plan" module: premise, setting, characters,
//! outline), grounded in the inventory: every character points to a figure
//! and every object to a part, by reference. The check is done by code, not
//! by a model, which replaces Re3's learned rerankers.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{Part, SetInventory};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryPlan {
    pub title: String,
    pub premise: String,
    pub setting: String,
    pub characters: Vec<PlannedCharacter>,
    pub outline: Vec<PlotPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedCharacter {
    pub name: String,
    /// Part reference of the figure, e.g. `30 00 496`.
    pub figure: String,
    pub portrait: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlotPoint {
    pub event: String,
    /// Names of the characters present.
    pub characters: Vec<String>,
    /// Part references of the objects used.
    pub objects: Vec<String>,
}

/// A fidelity problem found by the review (Re3's "Edit" module).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub excerpt: String,
    pub problem: String,
    pub fix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Review {
    pub issues: Vec<Issue>,
}

pub(crate) fn plan_schema() -> Value {
    let string = |description: &str| json!({"type": "string", "description": description});
    let strings = |description: &str| json!({"type": "array", "items": {"type": "string"}, "description": description});
    json!({
        "type": "object",
        "properties": {
            "title": string("Titre de l'histoire"),
            "premise": string("Prémisse, en une ou deux phrases"),
            "setting": string("Décor, construit avec les pièces de la boîte"),
            "characters": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": string("Prénom ou surnom"),
                        "figure": string("Référence [réf. …] de la figurine dans <personnages>"),
                        "portrait": string("Caractère et apparence, fidèle à la figurine"),
                    },
                    "required": ["name", "figure", "portrait"],
                    "additionalProperties": false,
                },
            },
            "outline": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "event": string("Ce qui se passe dans cette étape"),
                        "characters": strings("Noms des personnages présents"),
                        "objects": strings("Références [réf. …] des pièces utilisées"),
                    },
                    "required": ["event", "characters", "objects"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["title", "premise", "setting", "characters", "outline"],
        "additionalProperties": false,
    })
}

pub(crate) fn review_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "issues": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "excerpt": {"type": "string", "description": "Passage concerné, cité"},
                        "problem": {"type": "string", "description": "Ce qui n'est pas fidèle à la boîte"},
                        "fix": {"type": "string", "description": "Correction proposée, avec ce qui est dans la boîte"},
                    },
                    "required": ["excerpt", "problem", "fix"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["issues"],
        "additionalProperties": false,
    })
}

fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// Finds a part by reference, ignoring spacing and any `réf.` prefix.
pub fn find_part<'a>(inventory: &'a SetInventory, reference: &str) -> Option<&'a Part> {
    let wanted = digits(reference);
    if wanted.is_empty() {
        return None;
    }
    inventory.parts.iter().find(|p| digits(&p.number) == wanted)
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Every way the plan contradicts the inventory, as messages the model can
/// act on when asked to fix its plan. Empty = valid.
pub fn validate(plan: &StoryPlan, inventory: &SetInventory) -> Vec<String> {
    let mut errors = Vec::new();
    if plan.outline.is_empty() {
        errors.push("le plan ne contient aucune étape".to_owned());
    }
    if plan.characters.is_empty() && inventory.figures().next().is_some() {
        errors.push("aucun personnage : choisis-les parmi les figurines de <personnages>".into());
    }

    let mut names = HashSet::new();
    let mut uses: HashMap<&str, u32> = HashMap::new();
    for character in &plan.characters {
        if !names.insert(key(&character.name)) {
            errors.push(format!(
                "le nom « {} » est donné à deux personnages",
                character.name
            ));
        }
        match find_part(inventory, &character.figure) {
            None => errors.push(format!(
                "personnage « {} » : la référence « {} » n'existe pas dans la boîte",
                character.name, character.figure
            )),
            Some(part) if !part.is_figure() => errors.push(format!(
                "personnage « {} » : « {} » ({}) n'est pas une figurine",
                character.name, character.figure, part.name
            )),
            Some(part) => {
                let count = uses.entry(&part.number).or_default();
                *count += 1;
                if part.quantity.is_some_and(|q| *count == q + 1) {
                    errors.push(format!(
                        "la figurine {} ({}) n'existe qu'en {} exemplaire(s) mais sert à \
                         plusieurs personnages",
                        part.number,
                        part.name,
                        part.quantity.unwrap_or_default()
                    ));
                }
            }
        }
    }

    for (i, step) in plan.outline.iter().enumerate() {
        for name in &step.characters {
            if !names.contains(&key(name)) {
                errors.push(format!(
                    "étape {} : « {name} » ne fait pas partie des personnages",
                    i + 1
                ));
            }
        }
        for reference in &step.objects {
            if find_part(inventory, reference).is_none() {
                errors.push(format!(
                    "étape {} : la référence d'objet « {reference} » n'existe pas dans la boîte",
                    i + 1
                ));
            }
        }
    }
    errors
}

/// Last resort when the model can't produce a valid plan: drops every
/// reference that doesn't hold, so drafting only sees what is in the box.
pub fn sanitize(mut plan: StoryPlan, inventory: &SetInventory) -> StoryPlan {
    let mut uses: HashMap<String, u32> = HashMap::new();
    let mut names = HashSet::new();
    plan.characters
        .retain(|c| match find_part(inventory, &c.figure) {
            Some(part) if part.is_figure() => {
                let count = uses.entry(part.number.clone()).or_default();
                *count += 1;
                part.quantity.is_none_or(|q| *count <= q) && names.insert(key(&c.name))
            }
            _ => false,
        });
    for step in &mut plan.outline {
        step.characters.retain(|n| names.contains(&key(n)));
        step.objects.retain(|r| find_part(inventory, r).is_some());
    }
    plan
}

/// `Halberd (dark metal)`.
pub fn part_label(part: &Part) -> String {
    match &part.color {
        Some(color) => format!("{} ({color})", part.name),
        None => part.name.clone(),
    }
}

fn reference_label(inventory: &SetInventory, reference: &str) -> String {
    find_part(inventory, reference).map_or_else(|| reference.to_owned(), part_label)
}

/// The plan as text, for the drafting prompts.
pub fn render_plan(plan: &StoryPlan, inventory: &SetInventory) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Titre : {}", plan.title);
    let _ = writeln!(out, "Prémisse : {}", plan.premise);
    let _ = writeln!(out, "Décor : {}", plan.setting);
    out.push_str("Personnages :\n");
    for c in &plan.characters {
        let _ = writeln!(
            out,
            "- {} (figurine : {}) : {}",
            c.name,
            reference_label(inventory, &c.figure),
            c.portrait
        );
    }
    out.push_str("Étapes :\n");
    for (i, step) in plan.outline.iter().enumerate() {
        let _ = writeln!(out, "{}. {}", i + 1, step.event);
    }
    out
}

/// Objects of one step, as labels.
pub fn step_objects(step: &PlotPoint, inventory: &SetInventory) -> Vec<String> {
    step.objects
        .iter()
        .map(|r| reference_label(inventory, r))
        .collect()
}

/// The closing "In your box" section, built from the plan by code so that it
/// only ever lists parts that really are in the box.
pub fn in_your_box(plan: &StoryPlan, inventory: &SetInventory, language: &str) -> String {
    let french = language.to_lowercase().starts_with("fr");
    let (heading, figures_label, objects_label) = if french {
        ("Dans ta boîte", "Personnages", "Objets")
    } else {
        ("In your box", "Characters", "Objects")
    };

    let mut out = format!("## {heading}\n");
    if !plan.characters.is_empty() {
        let _ = write!(out, "\n**{figures_label}**\n");
        for c in &plan.characters {
            let _ = writeln!(
                out,
                "- {} — {}",
                c.name,
                reference_label(inventory, &c.figure)
            );
        }
    }

    let mut seen = HashSet::new();
    let objects: Vec<&Part> = plan
        .outline
        .iter()
        .flat_map(|s| &s.objects)
        .filter_map(|r| find_part(inventory, r))
        .filter(|p| !p.is_figure() && seen.insert(p.number.clone()))
        .collect();
    if !objects.is_empty() {
        let _ = write!(out, "\n**{objects_label}**\n");
        for part in objects {
            let quantity = part.quantity.map(|q| format!("{q}× ")).unwrap_or_default();
            let _ = writeln!(out, "- {quantity}{}", part_label(part));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::SetNumber;

    pub(crate) fn part(number: &str, name: &str, category: &str, quantity: Option<u32>) -> Part {
        Part {
            number: number.into(),
            name: name.into(),
            color: None,
            quantity,
            category: category.into(),
            components: vec![],
        }
    }

    pub(crate) fn inventory() -> SetInventory {
        SetInventory {
            set_number: SetNumber::parse("3666").unwrap(),
            name: "Kings Large Castle".into(),
            theme: None,
            focus: None,
            release_date: None,
            parts: vec![
                part("30 00 496", "Castle Trumpeter", "Klicky", Some(1)),
                part("30 14 044", "Lady in red dress", "Klicky", Some(1)),
                part("30 65 622", "Horse (old-style)", "Animal", Some(1)),
                part("30 07 640", "Halberd", "Tool", Some(4)),
            ],
            source_url: "test".into(),
        }
    }

    pub(crate) fn valid_plan() -> StoryPlan {
        StoryPlan {
            title: "Le cheval perdu".into(),
            premise: "Le cheval du château a disparu.".into(),
            setting: "Le grand château.".into(),
            characters: vec![
                PlannedCharacter {
                    name: "Léon".into(),
                    figure: "30 00 496".into(),
                    portrait: "trompettiste joyeux".into(),
                },
                PlannedCharacter {
                    name: "Rose".into(),
                    figure: "réf. 30 14 044".into(),
                    portrait: "dame en robe rouge".into(),
                },
            ],
            outline: vec![
                PlotPoint {
                    event: "Léon sonne l'alerte.".into(),
                    characters: vec!["Léon".into()],
                    objects: vec![],
                },
                PlotPoint {
                    event: "Rose retrouve le cheval.".into(),
                    characters: vec!["Rose".into(), "léon".into()],
                    objects: vec!["30 65 622".into(), "30 07 640".into()],
                },
            ],
        }
    }

    #[test]
    fn valid_plan_has_no_errors() {
        assert_eq!(validate(&valid_plan(), &inventory()), Vec::<String>::new());
    }

    #[test]
    fn detects_every_kind_of_infidelity() {
        let mut plan = valid_plan();
        plan.characters[1].figure = "30 00 496".into(); // only 1 trumpeter
        plan.characters.push(PlannedCharacter {
            name: "Dragon".into(),
            figure: "99 99 999".into(),
            portrait: "".into(),
        });
        plan.characters.push(PlannedCharacter {
            name: "Cheval".into(),
            figure: "30 65 622".into(), // an animal, not a figure
            portrait: "".into(),
        });
        plan.outline[0].characters.push("Merlin".into());
        plan.outline[1].objects.push("12 34 567".into());

        let errors = validate(&plan, &inventory());
        assert_eq!(errors.len(), 5, "{errors:#?}");
        assert!(errors.iter().any(|e| e.contains("qu'en 1 exemplaire")));
        assert!(errors.iter().any(|e| e.contains("99 99 999")));
        assert!(errors.iter().any(|e| e.contains("n'est pas une figurine")));
        assert!(errors.iter().any(|e| e.contains("Merlin")));
        assert!(errors.iter().any(|e| e.contains("12 34 567")));
    }

    #[test]
    fn sanitize_keeps_only_what_is_in_the_box() {
        let mut plan = valid_plan();
        plan.characters.push(PlannedCharacter {
            name: "Dragon".into(),
            figure: "99 99 999".into(),
            portrait: "".into(),
        });
        plan.outline[1].characters.push("Dragon".into());
        plan.outline[1].objects.push("12 34 567".into());

        let clean = sanitize(plan, &inventory());
        assert!(validate(&clean, &inventory()).is_empty());
        assert_eq!(clean.characters.len(), 2);
        assert_eq!(clean.outline[1].objects.len(), 2);
    }

    #[test]
    fn in_your_box_lists_plan_parts_once() {
        let mut plan = valid_plan();
        plan.outline[0].objects.push("30 07 640".into());
        let section = in_your_box(&plan, &inventory(), "français");
        assert!(section.starts_with("## Dans ta boîte"));
        assert!(section.contains("- Léon — Castle Trumpeter"));
        assert_eq!(section.matches("4× Halberd").count(), 1);
        assert!(section.contains("1× Horse (old-style)"));
        assert!(in_your_box(&plan, &inventory(), "English").starts_with("## In your box"));
    }
}
