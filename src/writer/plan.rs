//! The story plan (Re3's "Plan" module: premise, setting, characters,
//! outline), grounded in the inventory: every character points to a figure
//! and every object to a part, by reference. The check is done by code, not
//! by a model, which replaces Re3's learned rerankers.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::augment::StoryOptions;
use crate::model::{Collection, Part, SetInventory, reference_key};

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

/// Every box holding a reference (spacing and any `réf.` prefix ignored),
/// with the matching part.
pub fn holdings<'a>(
    collection: &'a Collection,
    reference: &str,
) -> Vec<(&'a SetInventory, &'a Part)> {
    let wanted = reference_key(reference);
    if wanted.is_empty() {
        return Vec::new();
    }
    collection
        .parts()
        .filter(|(_, p)| reference_key(&p.number) == wanted)
        .collect()
}

/// Finds a part by reference in any of the boxes.
pub fn find_part<'a>(collection: &'a Collection, reference: &str) -> Option<&'a Part> {
    holdings(collection, reference).first().map(|(_, p)| *p)
}

/// Copies of a reference across all boxes; `None` if a box doesn't know.
pub fn total_quantity(collection: &Collection, reference: &str) -> Option<u32> {
    holdings(collection, reference)
        .iter()
        .map(|(_, p)| p.quantity)
        .sum()
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Every way the plan contradicts the boxes, as messages the model can act
/// on when asked to fix its plan. Empty = valid.
pub fn validate(plan: &StoryPlan, collection: &Collection) -> Vec<String> {
    let boxes = if collection.is_single() {
        "la boîte"
    } else {
        "les boîtes"
    };
    let mut errors = Vec::new();
    if plan.outline.is_empty() {
        errors.push("le plan ne contient aucune étape".to_owned());
    }
    if plan.characters.is_empty() && collection.figures().next().is_some() {
        errors.push("aucun personnage : choisis-les parmi les figurines de <personnages>".into());
    }

    let mut names = HashSet::new();
    let mut uses: HashMap<String, u32> = HashMap::new();
    for character in &plan.characters {
        if !names.insert(key(&character.name)) {
            errors.push(format!(
                "le nom « {} » est donné à deux personnages",
                character.name
            ));
        }
        match find_part(collection, &character.figure) {
            None => errors.push(format!(
                "personnage « {} » : la référence « {} » n'existe pas dans {boxes}",
                character.name, character.figure
            )),
            Some(part) if !part.is_figure() => errors.push(format!(
                "personnage « {} » : « {} » ({}) n'est pas une figurine",
                character.name, character.figure, part.name
            )),
            Some(part) => {
                let count = uses.entry(part.number.clone()).or_default();
                *count += 1;
                let available = total_quantity(collection, &part.number);
                if available.is_some_and(|q| *count == q + 1) {
                    errors.push(format!(
                        "la figurine {} ({}) n'existe qu'en {} exemplaire(s) mais sert à \
                         plusieurs personnages",
                        part.number,
                        part.name,
                        available.unwrap_or_default()
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
            if find_part(collection, reference).is_none() {
                errors.push(format!(
                    "étape {} : la référence d'objet « {reference} » n'existe pas dans {boxes}",
                    i + 1
                ));
            }
        }
    }
    errors
}

/// Last resort when the model can't produce a valid plan: drops every
/// reference that doesn't hold, so drafting only sees what is in the boxes.
pub fn sanitize(mut plan: StoryPlan, collection: &Collection) -> StoryPlan {
    let mut uses: HashMap<String, u32> = HashMap::new();
    let mut names = HashSet::new();
    plan.characters
        .retain(|c| match find_part(collection, &c.figure) {
            Some(part) if part.is_figure() => {
                let count = uses.entry(part.number.clone()).or_default();
                *count += 1;
                total_quantity(collection, &part.number).is_none_or(|q| *count <= q)
                    && names.insert(key(&c.name))
            }
            _ => false,
        });
    for step in &mut plan.outline {
        step.characters.retain(|n| names.contains(&key(n)));
        step.objects.retain(|r| find_part(collection, r).is_some());
    }
    plan
}

/// Steps a favorite part (`--star`) must appear in: a figure is the main
/// character, in half of them (the first one included); an object serves
/// in two.
fn star_minimum(part: &Part, steps: usize) -> usize {
    if part.is_figure() {
        steps.div_ceil(2)
    } else {
        steps.min(2)
    }
}

/// Names of the characters played by the figure `part`.
fn played_by(plan: &StoryPlan, part: &Part) -> HashSet<String> {
    let wanted = reference_key(&part.number);
    plan.characters
        .iter()
        .filter(|c| reference_key(&c.figure) == wanted)
        .map(|c| key(&c.name))
        .collect()
}

/// Whether step `step` features the favorite part.
fn features(step: &PlotPoint, part: &Part, names: &HashSet<String>) -> bool {
    if part.is_figure() {
        step.characters.iter().any(|n| names.contains(&key(n)))
    } else {
        let wanted = reference_key(&part.number);
        step.objects.iter().any(|r| reference_key(r) == wanted)
    }
}

/// Every way the plan sidelines the child's favorite parts (`--star`), as
/// messages the model can act on. References missing from the boxes are
/// skipped (checked before any call). Empty = valid.
pub fn validate_stars(plan: &StoryPlan, collection: &Collection, stars: &[String]) -> Vec<String> {
    let steps = plan.outline.len();
    let mut errors = Vec::new();
    for star in stars {
        let Some(part) = find_part(collection, star).filter(|_| steps > 0) else {
            continue;
        };
        let names = played_by(plan, part);
        let label = format!("{} ({})", part.number, part_label(part));
        if part.is_figure() && names.is_empty() {
            errors.push(format!(
                "la figurine préférée de l'enfant, {label}, doit jouer le personnage principal"
            ));
            continue;
        }
        if part.is_figure() && !features(&plan.outline[0], part, &names) {
            errors.push(format!(
                "la figurine préférée de l'enfant, {label}, doit être présente dès l'étape 1"
            ));
        }
        let count = plan
            .outline
            .iter()
            .filter(|s| features(s, part, &names))
            .count();
        let minimum = star_minimum(part, steps);
        if count < minimum {
            let what = if part.is_figure() {
                "la figurine"
            } else {
                "l'objet"
            };
            errors.push(format!(
                "{what} préféré(e) de l'enfant, {label}, doit être au cœur de l'histoire : \
                 présent(e) dans au moins {minimum} étapes sur {steps}, et non {count}"
            ));
        }
    }
    errors
}

/// Last resort after [`sanitize`]: puts the favorite parts in the steps
/// that lack them, from the first one, until each appears in enough steps.
/// A favorite figure that plays no character is left out (no name to give).
pub fn feature_stars(mut plan: StoryPlan, collection: &Collection, stars: &[String]) -> StoryPlan {
    let steps = plan.outline.len();
    for star in stars {
        let Some(part) = find_part(collection, star) else {
            continue;
        };
        let names = played_by(&plan, part);
        let entry = if part.is_figure() {
            match plan
                .characters
                .iter()
                .find(|c| names.contains(&key(&c.name)))
            {
                Some(character) => character.name.clone(),
                None => continue,
            }
        } else {
            part.number.clone()
        };
        let mut count = plan
            .outline
            .iter()
            .filter(|s| features(s, part, &names))
            .count();
        let minimum = star_minimum(part, steps);
        for (i, step) in plan.outline.iter_mut().enumerate() {
            let needed = count < minimum || (i == 0 && part.is_figure());
            if !needed || features(step, part, &names) {
                continue;
            }
            if part.is_figure() {
                step.characters.push(entry.clone());
            } else {
                step.objects.push(entry.clone());
            }
            count += 1;
        }
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

/// The label, followed by the citation when the story cites its parts.
pub fn cited_label(part: &Part, citations: bool) -> String {
    if citations {
        format!("{} [réf. {}]", part_label(part), part.number)
    } else {
        part_label(part)
    }
}

fn reference_label(collection: &Collection, reference: &str) -> String {
    find_part(collection, reference).map_or_else(|| reference.to_owned(), part_label)
}

/// The plan as text, for the drafting prompts.
pub fn render_plan(plan: &StoryPlan, collection: &Collection) -> String {
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
            reference_label(collection, &c.figure),
            c.portrait
        );
    }
    out.push_str("Étapes :\n");
    for (i, step) in plan.outline.iter().enumerate() {
        let _ = writeln!(out, "{}. {}", i + 1, step.event);
    }
    out
}

/// Objects of one step, as labels, with their companion pieces:
/// `Parrot body (light blue) + Parrot wings (yellow)`.
/// A piece already shown with an earlier object of the step isn't repeated.
pub fn step_objects(step: &PlotPoint, collection: &Collection, citations: bool) -> Vec<String> {
    step_items(step, collection)
        .into_iter()
        .map(|item| match item {
            Ok(pieces) => pieces
                .into_iter()
                .map(|p| cited_label(p, citations))
                .collect::<Vec<_>>()
                .join(" + "),
            Err(reference) => reference.clone(),
        })
        .collect()
}

/// Objects of one step, each with its companion pieces; an unknown
/// reference is kept as is (`Err`).
fn step_items<'a>(
    step: &'a PlotPoint,
    collection: &'a Collection,
) -> Vec<std::result::Result<Vec<&'a Part>, &'a String>> {
    let mut shown = HashSet::new();
    step.objects
        .iter()
        .filter_map(|r| match holdings(collection, r).first() {
            Some((set, part)) => {
                if !shown.insert(part.number.clone()) {
                    return None;
                }
                let companions = companions(part, set);
                shown.extend(companions.iter().map(|p| p.number.clone()));
                Some(Ok(std::iter::once(*part).chain(companions).collect()))
            }
            None => Some(Err(r)),
        })
        .collect()
}

/// What to put on the table before a part of the story is played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSetup<'a> {
    /// Each character present, with its figure.
    pub characters: Vec<(&'a str, &'a Part)>,
    /// Each object used, with its companion pieces.
    pub objects: Vec<Vec<&'a Part>>,
}

impl SceneSetup<'_> {
    pub fn is_empty(&self) -> bool {
        self.characters.is_empty() && self.objects.is_empty()
    }
}

/// The scene setup of a plot point, built from the validated plan: only
/// parts that are in the boxes.
pub fn scene_setup<'a>(
    step: &'a PlotPoint,
    plan: &'a StoryPlan,
    collection: &'a Collection,
) -> SceneSetup<'a> {
    let characters = step
        .characters
        .iter()
        .filter_map(|name| {
            let character = plan.characters.iter().find(|c| key(&c.name) == key(name))?;
            let figure = find_part(collection, &character.figure)?;
            Some((character.name.as_str(), figure))
        })
        .collect();
    let objects = step_items(step, collection)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect();
    SceneSetup {
        characters,
        objects,
    }
}

/// The scene setup as a Markdown quote, before the part it sets up:
/// `> 🧱 **Mise en place** : Léon (Castle Trumpeter) ; Horse, Halberd`.
pub fn scene_setup_markdown(setup: &SceneSetup<'_>, options: &StoryOptions) -> String {
    let label = |part: &Part| cited_label(part, options.citations);
    let characters: Vec<String> = setup
        .characters
        .iter()
        .map(|(name, figure)| format!("{name} ({})", label(figure)))
        .collect();
    let objects: Vec<String> = setup
        .objects
        .iter()
        .map(|pieces| {
            pieces
                .iter()
                .map(|p| label(p))
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .collect();
    let items: Vec<String> = [characters.join(", "), objects.join(", ")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    format!(
        "> 🧱 **{}** : {}",
        Labels::new(options).scene_setup,
        items.join(" ; ")
    )
}

/// Largest group of same-named pieces still treated as one object: beyond
/// that, a shared first word means a family (castle walls, rocks...), not
/// the pieces of a single object.
const MAX_OBJECT_PIECES: usize = 3;

fn first_word(name: &str) -> String {
    name.split(|c: char| c.is_whitespace() || ",;:(".contains(c))
        .find(|w| !w.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

/// Other lines of the same box that are pieces of the same object as
/// `part`. PlaymoDB lists some objects as several parts: "Parrot body" +
/// "Parrot wings", "Trunk; body and lid" + "Trunk; base and lock",
/// "Rowboat" + "Rowboat gear"... They share their first word and category,
/// in a small group.
pub fn companions<'a>(part: &Part, set: &'a SetInventory) -> Vec<&'a Part> {
    let word = first_word(&part.name);
    let group: Vec<&Part> = set
        .parts
        .iter()
        .filter(|p| p.category == part.category && first_word(&p.name) == word)
        .collect();
    if group.len() > MAX_OBJECT_PIECES {
        return Vec::new();
    }
    group
        .into_iter()
        .filter(|p| p.number != part.number)
        .collect()
}

/// Headings of the parts of the story written by code.
#[derive(Debug, Clone, Copy)]
pub struct Labels {
    pub in_your_box: &'static str,
    pub characters: &'static str,
    pub objects: &'static str,
    pub scene_setup: &'static str,
    /// `boîte`, as in `(boîte 3666)`.
    pub a_box: &'static str,
    /// How to read an interactive story.
    pub interactive: &'static str,
    pub choose: &'static str,
    /// `va au`, as in `va au 3`.
    pub go_to: &'static str,
    pub the_end: &'static str,
}

impl Labels {
    pub fn new(options: &StoryOptions) -> Self {
        if options.language.to_lowercase().starts_with("fr") {
            Self {
                in_your_box: "Dans ta boîte",
                characters: "Personnages",
                objects: "Objets",
                scene_setup: "Mise en place",
                a_box: "boîte",
                interactive: "C'est toi qui choisis la suite : commence au paragraphe 1, \
                              puis suis tes choix.",
                choose: "Que choisis-tu ?",
                go_to: "va au",
                the_end: "Fin",
            }
        } else {
            Self {
                in_your_box: "In your box",
                characters: "Characters",
                objects: "Objects",
                scene_setup: "Set the scene",
                a_box: "box",
                interactive: "You choose what happens: start at paragraph 1, then follow \
                              your choices.",
                choose: "What do you choose?",
                go_to: "go to",
                the_end: "The End",
            }
        }
    }
}

/// What the story uses from the boxes, from the plan.
#[derive(Debug, Clone)]
pub struct BoxContents<'a> {
    /// Each character, with its figure when it is in the boxes.
    pub characters: Vec<(&'a PlannedCharacter, Option<(&'a SetInventory, &'a Part)>)>,
    /// Each object (not a figure) once per box holding it, with its
    /// companion pieces.
    pub objects: Vec<(&'a SetInventory, &'a Part)>,
}

pub fn box_contents<'a>(plan: &'a StoryPlan, collection: &'a Collection) -> BoxContents<'a> {
    let characters = plan
        .characters
        .iter()
        .map(|c| (c, holdings(collection, &c.figure).first().copied()))
        .collect();

    // Each object comes with its companion pieces (parrot body → wings),
    // from every box that holds it.
    let mut seen = HashSet::new();
    let objects = plan
        .outline
        .iter()
        .flat_map(|s| &s.objects)
        .flat_map(|r| holdings(collection, r))
        .flat_map(|(set, part)| {
            std::iter::once(part)
                .chain(companions(part, set))
                .map(move |p| (set, p))
        })
        .filter(|(set, p)| {
            !p.is_figure() && seen.insert((set.set_number.clone(), p.number.clone()))
        })
        .collect();
    BoxContents {
        characters,
        objects,
    }
}

/// The closing "In your box" section, built from the plan by code so that it
/// only ever lists parts that really are in the boxes. With several boxes,
/// each line says which box to look in.
pub fn in_your_box(plan: &StoryPlan, collection: &Collection, options: &StoryOptions) -> String {
    let labels = Labels::new(options);
    let label = |part: &Part| cited_label(part, options.citations);
    let origin = |set: &SetInventory| {
        if collection.is_single() {
            String::new()
        } else {
            format!(" ({} {})", labels.a_box, set.set_number)
        }
    };
    let contents = box_contents(plan, collection);

    let mut out = format!("## {}\n", labels.in_your_box);
    if !contents.characters.is_empty() {
        let _ = write!(out, "\n**{}**\n", labels.characters);
        for (c, figure) in contents.characters {
            let label = match figure {
                Some((set, part)) => format!("{}{}", label(part), origin(set)),
                None => c.figure.clone(),
            };
            let _ = writeln!(out, "- {} — {label}", c.name);
        }
    }
    if !contents.objects.is_empty() {
        let _ = write!(out, "\n**{}**\n", labels.objects);
        for (set, part) in contents.objects {
            let quantity = part.quantity.map(|q| format!("{q}× ")).unwrap_or_default();
            let _ = writeln!(out, "- {quantity}{}{}", label(part), origin(set));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::SetNumber;

    fn english() -> StoryOptions {
        StoryOptions {
            language: "English".into(),
            ..StoryOptions::default()
        }
    }

    #[test]
    fn in_your_box_cites_references() {
        let mut plan = valid_plan();
        plan.outline[0].objects.push("30 07 640".into());
        let boxes: Collection = inventory().into();
        let cited = StoryOptions {
            citations: true,
            ..StoryOptions::default()
        };
        let section = in_your_box(&plan, &boxes, &cited);
        assert!(section.contains("- Léon — Castle Trumpeter [réf. "));
        assert!(section.contains("4× Halberd [réf. 30 07 640]\n"));
        assert!(!in_your_box(&plan, &boxes, &StoryOptions::default()).contains("[réf."));
    }

    pub(crate) fn part(number: &str, name: &str, category: &str, quantity: Option<u32>) -> Part {
        Part {
            number: number.into(),
            name: name.into(),
            color: None,
            quantity,
            category: category.into(),
            components: vec![],
            image: None,
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
    fn favorite_parts_must_be_at_the_heart_of_the_plan() {
        let boxes: Collection = inventory().into();
        let stars = |refs: &[&str]| refs.iter().map(|r| (*r).to_owned()).collect::<Vec<_>>();
        let plan = valid_plan();
        // Léon is in both steps; an unknown reference is checked elsewhere.
        assert!(validate_stars(&plan, &boxes, &stars(&["3000496", "99 99 999"])).is_empty());

        let errors = validate_stars(&plan, &boxes, &stars(&["30 14 044", "30 65 622"]));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("30 14 044 (Lady in red dress), doit être présente dès l'étape 1")
        );
        assert!(errors[1].contains("au moins 2 étapes sur 2, et non 1"));

        let mut unplayed = valid_plan();
        unplayed.characters.pop();
        for step in &mut unplayed.outline {
            step.characters.retain(|n| n != "Rose");
        }
        let errors = validate_stars(&unplayed, &boxes, &stars(&["30 14 044"]));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("doit jouer le personnage principal"));
    }

    #[test]
    fn favorite_parts_are_put_back_in_the_plan() {
        let boxes: Collection = inventory().into();
        let stars = vec!["30 14 044".to_owned(), "30 65 622".to_owned()];
        let plan = feature_stars(valid_plan(), &boxes, &stars);
        assert_eq!(plan.outline[0].characters, ["Léon", "Rose"]);
        assert_eq!(plan.outline[0].objects, ["30 65 622"]);
        assert_eq!(plan.outline[1], valid_plan().outline[1]);
        assert!(validate_stars(&plan, &boxes, &stars).is_empty());
        assert!(validate(&plan, &boxes).is_empty());
    }

    #[test]
    fn valid_plan_has_no_errors() {
        assert_eq!(
            validate(&valid_plan(), &inventory().into()),
            Vec::<String>::new()
        );
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

        let errors = validate(&plan, &inventory().into());
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

        let boxes: Collection = inventory().into();
        let clean = sanitize(plan, &boxes);
        assert!(validate(&clean, &boxes).is_empty());
        assert_eq!(clean.characters.len(), 2);
        assert_eq!(clean.outline[1].objects.len(), 2);
    }

    #[test]
    fn companions_complete_objects_split_in_pieces() {
        let mut inventory = inventory();
        inventory.parts.extend([
            part("30 06 198", "Parrot body", "Animal", Some(1)),
            part("30 06 199", "Parrot wings", "Animal", Some(1)),
            part("30 60 895", "Trunk; body and lid", "Furniture", Some(1)),
            part("30 60 896", "Trunk; base and lock", "Furniture", Some(1)),
            part("30 06 574", "Hut floor", "Building", Some(1)),
            part("30 06 575", "Hut Support (Stilt)", "Building", Some(5)),
            part("30 06 576", "Hut roof support", "Building", Some(1)),
            part("30 06 577", "Hut roof thatch", "Building", Some(1)),
        ]);
        let boxes: Collection = inventory.clone().into();
        let names = |number: &str| -> Vec<String> {
            let part = find_part(&boxes, number).unwrap();
            companions(part, &inventory)
                .iter()
                .map(|p| p.name.clone())
                .collect()
        };
        assert_eq!(names("30 06 198"), ["Parrot wings"]);
        assert_eq!(names("30 60 896"), ["Trunk; body and lid"]);
        // A family of 4 hut pieces is a building, not one object.
        assert!(names("30 06 574").is_empty());
        // A part alone in its kind has no companion.
        assert!(names("30 07 640").is_empty());

        let mut plan = valid_plan();
        plan.outline[0].objects.push("30 06 198".into());
        let section = in_your_box(&plan, &boxes, &StoryOptions::default());
        assert!(section.contains("- 1× Parrot body\n- 1× Parrot wings\n"));
    }

    #[test]
    fn in_your_box_lists_plan_parts_once() {
        let mut plan = valid_plan();
        plan.outline[0].objects.push("30 07 640".into());
        let boxes: Collection = inventory().into();
        let section = in_your_box(&plan, &boxes, &StoryOptions::default());
        assert!(section.starts_with("## Dans ta boîte"));
        assert!(section.contains("- Léon — Castle Trumpeter\n"));
        assert_eq!(section.matches("4× Halberd").count(), 1);
        assert!(section.contains("1× Horse (old-style)\n"));
        assert!(in_your_box(&plan, &boxes, &english()).starts_with("## In your box"));
    }

    /// A second box: a pirate figure and two more halberds.
    fn two_boxes() -> Collection {
        let mut ship = inventory();
        ship.set_number = SetNumber::parse("5135").unwrap();
        ship.name = "Pirates Ship".into();
        ship.parts = vec![
            part(
                "30 00 961",
                "Pirate, bald with yellow headband",
                "Klicky",
                Some(1),
            ),
            part("30 07 640", "Halberd", "Tool", Some(2)),
        ];
        Collection::new(vec![inventory(), ship])
    }

    #[test]
    fn scene_setup_lists_what_to_put_on_the_table() {
        let mut inventory = inventory();
        inventory.parts.extend([
            part("30 06 198", "Parrot body", "Animal", Some(1)),
            part("30 06 199", "Parrot wings", "Animal", Some(1)),
        ]);
        let boxes: Collection = inventory.into();
        let mut plan = valid_plan();
        plan.outline[1].objects.push("30 06 198".into());
        plan.outline[1].characters.push("Fantôme".into());

        let setup = scene_setup(&plan.outline[1], &plan, &boxes);
        let names: Vec<&str> = setup.characters.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["Rose", "Léon"], "unknown names left out");
        assert_eq!(setup.objects.len(), 3);
        assert_eq!(
            scene_setup_markdown(&setup, &StoryOptions::default()),
            "> 🧱 **Mise en place** : Rose (Lady in red dress), Léon (Castle Trumpeter) ; \
             Horse (old-style), Halberd, Parrot body + Parrot wings"
        );
        let cited = StoryOptions {
            citations: true,
            ..english()
        };
        let first = scene_setup(&plan.outline[0], &plan, &boxes);
        assert_eq!(
            scene_setup_markdown(&first, &cited),
            "> 🧱 **Set the scene** : Léon (Castle Trumpeter [réf. 30 00 496])"
        );
        let empty = PlotPoint {
            event: "".into(),
            characters: vec![],
            objects: vec![],
        };
        assert!(scene_setup(&empty, &plan, &boxes).is_empty());
    }

    #[test]
    fn step_objects_show_each_piece_once() {
        let mut inventory = inventory();
        inventory.parts.extend([
            part("30 05 806", "Rowboat, one-seater", "Vehicle", Some(1)),
            part("30 60 815", "Rowboat gear", "Vehicle", Some(1)),
        ]);
        let step = PlotPoint {
            event: "".into(),
            characters: vec![],
            objects: vec!["30 05 806".into(), "30 60 815".into(), "30 07 640".into()],
        };
        assert_eq!(
            step_objects(&step, &inventory.into(), false),
            ["Rowboat, one-seater + Rowboat gear", "Halberd"]
        );
    }

    #[test]
    fn quantities_add_up_across_boxes() {
        let boxes = two_boxes();
        assert_eq!(total_quantity(&boxes, "30 07 640"), Some(6));
        assert_eq!(holdings(&boxes, "30 07 640").len(), 2);
        assert_eq!(total_quantity(&boxes, "30 00 496"), Some(1));
        assert_eq!(total_quantity(&boxes, "99 99 999"), Some(0));
    }

    #[test]
    fn characters_can_come_from_any_box() {
        let boxes = two_boxes();
        let mut plan = valid_plan();
        plan.characters.push(PlannedCharacter {
            name: "Barbe-Jaune".into(),
            figure: "30 00 961".into(),
            portrait: "pirate".into(),
        });
        plan.outline[1].characters.push("Barbe-Jaune".into());
        assert!(validate(&plan, &boxes).is_empty());
        // The same pirate can't play two characters: one copy in all boxes.
        plan.characters.push(PlannedCharacter {
            name: "Jumeau".into(),
            figure: "30 00 961".into(),
            portrait: "".into(),
        });
        let errors = validate(&plan, &boxes);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert!(errors[0].contains("qu'en 1 exemplaire"));
    }

    #[test]
    fn in_your_box_says_which_box() {
        let boxes = two_boxes();
        let mut plan = valid_plan();
        plan.characters.push(PlannedCharacter {
            name: "Barbe-Jaune".into(),
            figure: "30 00 961".into(),
            portrait: "pirate".into(),
        });
        let section = in_your_box(&plan, &boxes, &StoryOptions::default());
        assert!(section.contains("- Léon — Castle Trumpeter (boîte 3666)\n"));
        assert!(
            section.contains("- Barbe-Jaune — Pirate, bald with yellow headband (boîte 5135)\n")
        );
        // Halberds are in both boxes: one line per box.
        assert!(section.contains("- 4× Halberd (boîte 3666)\n- 2× Halberd (boîte 5135)\n"));
    }
}
