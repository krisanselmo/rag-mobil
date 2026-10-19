//! Interactive story ("dont tu es le héros"): the plan is a graph of
//! paragraphs, each ending with choices that lead further on, down to
//! several endings. Like the linear plan, every character and object is a
//! reference checked against the boxes; the graph itself (links, endings,
//! no loop, nothing unreachable) is checked by code too.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::augment::StoryOptions;
use crate::error::Result;
use crate::generator::{Generation, StoryGenerator};
use crate::model::Collection;
use crate::writer::plan::{
    Labels, PlannedCharacter, PlotPoint, Review, StoryPlan, in_your_box, review_schema, sanitize,
    scene_setup, scene_setup_markdown, validate,
};
use crate::writer::planned::{Tally, fall_back_to_direct, parse_json};
use crate::writer::{Observer, Step, WriterOptions, Written, prompts};

/// Most choices a paragraph may offer.
const MAX_CHOICES: usize = 3;
/// Fewest paragraphs for a story with real choices.
const MIN_PARAGRAPHS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePlan {
    pub title: String,
    pub premise: String,
    pub setting: String,
    pub characters: Vec<PlannedCharacter>,
    /// The first one is the beginning.
    pub paragraphs: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub event: String,
    pub characters: Vec<String>,
    pub objects: Vec<String>,
    /// None: an ending.
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// What the reader decides, e.g. "Pépin explore la grotte".
    pub label: String,
    /// `id` of the paragraph it leads to.
    pub goto: String,
}

/// A paragraph of the written story, numbered from 1 in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    /// Each choice and the number of the paragraph it leads to; none for an
    /// ending.
    pub choices: Vec<(String, usize)>,
}

impl GamePlan {
    /// The plan as a linear one (one step per paragraph), for everything
    /// shared with the planned method: checks, scene setups, "In your box".
    pub fn story_plan(&self) -> StoryPlan {
        StoryPlan {
            title: self.title.clone(),
            premise: self.premise.clone(),
            setting: self.setting.clone(),
            characters: self.characters.clone(),
            outline: self
                .paragraphs
                .iter()
                .map(|n| PlotPoint {
                    event: n.event.clone(),
                    characters: n.characters.clone(),
                    objects: n.objects.clone(),
                })
                .collect(),
        }
    }

    /// Takes back the characters and references of a repaired linear plan.
    fn with_story_plan(mut self, plan: StoryPlan) -> Self {
        self.characters = plan.characters;
        for (node, step) in self.paragraphs.iter_mut().zip(plan.outline) {
            node.characters = step.characters;
            node.objects = step.objects;
        }
        self
    }

    fn index(&self) -> HashMap<&str, usize> {
        self.paragraphs
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.trim(), i))
            .collect()
    }

    /// Paragraphs in reading order (breadth first from the beginning), each
    /// with the paragraph it is first reached from. Unreachable ones are
    /// left out.
    fn reading_order(&self) -> Vec<(usize, Option<usize>)> {
        let index = self.index();
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        if !self.paragraphs.is_empty() {
            queue.push_back((0, None));
            seen.insert(0);
        }
        while let Some((i, parent)) = queue.pop_front() {
            order.push((i, parent));
            for choice in &self.paragraphs[i].choices {
                if let Some(&next) = index.get(choice.goto.trim())
                    && seen.insert(next)
                {
                    queue.push_back((next, Some(i)));
                }
            }
        }
        order
    }

    /// The paragraphs in reading order, and how they link to each other.
    fn ordered(self) -> (Self, Vec<Paragraph>, Vec<Option<usize>>) {
        let order = self.reading_order();
        let number: HashMap<&str, usize> = order
            .iter()
            .enumerate()
            .map(|(n, (i, _))| (self.paragraphs[*i].id.trim(), n + 1))
            .collect();
        let position: HashMap<usize, usize> = order
            .iter()
            .enumerate()
            .map(|(n, (i, _))| (*i, n))
            .collect();
        let paragraphs = order
            .iter()
            .map(|(i, _)| Paragraph {
                choices: self.paragraphs[*i]
                    .choices
                    .iter()
                    .filter_map(|c| Some((c.label.trim().to_owned(), *number.get(c.goto.trim())?)))
                    .collect(),
            })
            .collect();
        let parents = order
            .iter()
            .map(|(_, parent)| parent.map(|p| position[&p]))
            .collect();
        let nodes = order
            .iter()
            .map(|(i, _)| self.paragraphs[*i].clone())
            .collect();
        (
            Self {
                paragraphs: nodes,
                ..self
            },
            paragraphs,
            parents,
        )
    }

    /// Longest way through the story, in paragraphs.
    fn longest_path(&self) -> usize {
        let index = self.index();
        let mut memo: HashMap<usize, usize> = HashMap::new();
        fn depth(
            plan: &GamePlan,
            index: &HashMap<&str, usize>,
            i: usize,
            memo: &mut HashMap<usize, usize>,
            visiting: &mut HashSet<usize>,
        ) -> usize {
            if let Some(&d) = memo.get(&i) {
                return d;
            }
            if !visiting.insert(i) {
                return 0; // a loop; reported by the validation
            }
            let d = 1 + plan.paragraphs[i]
                .choices
                .iter()
                .filter_map(|c| index.get(c.goto.trim()))
                .map(|&next| depth(plan, index, next, memo, visiting))
                .max()
                .unwrap_or(0);
            visiting.remove(&i);
            memo.insert(i, d);
            d
        }
        if self.paragraphs.is_empty() {
            return 0;
        }
        depth(self, &index, 0, &mut memo, &mut HashSet::new())
    }
}

/// Every way the graph is broken, as messages the model can act on.
pub fn validate_graph(plan: &GamePlan) -> Vec<String> {
    let mut errors = Vec::new();
    if plan.paragraphs.len() < MIN_PARAGRAPHS {
        errors.push(format!(
            "il faut au moins {MIN_PARAGRAPHS} paragraphes (il y en a {})",
            plan.paragraphs.len()
        ));
    }
    let mut ids = HashSet::new();
    for node in &plan.paragraphs {
        if node.id.trim().is_empty() {
            errors.push(format!(
                "le paragraphe « {} » n'a pas d'identifiant",
                node.event
            ));
        } else if !ids.insert(node.id.trim()) {
            errors.push(format!(
                "l'identifiant « {} » est donné à deux paragraphes",
                node.id
            ));
        }
    }
    let index = plan.index();
    for node in &plan.paragraphs {
        let id = node.id.trim();
        if node.choices.len() == 1 {
            errors.push(format!(
                "le paragraphe « {id} » n'a qu'un choix : il en faut 2 ou {MAX_CHOICES}, ou \
                 aucun pour une fin"
            ));
        }
        if node.choices.len() > MAX_CHOICES {
            errors.push(format!(
                "le paragraphe « {id} » propose {} choix : {MAX_CHOICES} au plus",
                node.choices.len()
            ));
        }
        for choice in &node.choices {
            let goto = choice.goto.trim();
            if choice.label.trim().is_empty() {
                errors.push(format!("un choix du paragraphe « {id} » n'a pas de texte"));
            }
            if goto == id {
                errors.push(format!("le paragraphe « {id} » mène à lui-même"));
            } else if !index.contains_key(goto) {
                errors.push(format!(
                    "le paragraphe « {id} » mène à « {goto} », qui n'existe pas"
                ));
            }
        }
    }
    let endings = plan
        .paragraphs
        .iter()
        .filter(|n| n.choices.is_empty())
        .count();
    if endings < 2 {
        errors.push(format!(
            "il faut au moins 2 fins (paragraphes sans choix), il y en a {endings}"
        ));
    }
    if plan
        .paragraphs
        .first()
        .is_some_and(|n| n.choices.is_empty())
    {
        errors.push("le premier paragraphe doit proposer des choix".into());
    }
    let reachable: HashSet<usize> = plan.reading_order().iter().map(|(i, _)| *i).collect();
    for (i, node) in plan.paragraphs.iter().enumerate() {
        if !reachable.contains(&i) {
            errors.push(format!(
                "aucun choix ne mène au paragraphe « {} »",
                node.id.trim()
            ));
        }
    }
    if let Some(id) = find_loop(plan) {
        errors.push(format!(
            "les choix forment une boucle qui repasse par « {id} » : un choix doit toujours \
             mener plus loin"
        ));
    }
    errors
}

/// A paragraph on a loop, if any (depth-first search).
fn find_loop(plan: &GamePlan) -> Option<String> {
    let index = plan.index();
    // 0 = not seen, 1 = on the current path, 2 = done.
    let mut state = vec![0u8; plan.paragraphs.len()];
    fn visit(
        plan: &GamePlan,
        index: &HashMap<&str, usize>,
        i: usize,
        state: &mut [u8],
    ) -> Option<usize> {
        state[i] = 1;
        for choice in &plan.paragraphs[i].choices {
            let Some(&next) = index.get(choice.goto.trim()) else {
                continue;
            };
            match state[next] {
                1 => return Some(next),
                0 => {
                    if let Some(found) = visit(plan, index, next, state) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        state[i] = 2;
        None
    }
    (0..plan.paragraphs.len())
        .find_map(|i| {
            (state[i] == 0)
                .then(|| visit(plan, &index, i, &mut state))
                .flatten()
        })
        .map(|i| plan.paragraphs[i].id.trim().to_owned())
}

pub(crate) fn game_schema() -> Value {
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
            "paragraphs": {
                "type": "array",
                "description": "Le premier paragraphe est le début",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": string("Identifiant court et unique, ex. p1"),
                        "event": string("Ce qui se passe dans ce paragraphe"),
                        "characters": strings("Noms des personnages présents"),
                        "objects": strings("Références [réf. …] des pièces utilisées"),
                        "choices": {
                            "type": "array",
                            "description": "2 ou 3 choix ; aucun pour une fin",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": string("Ce que le lecteur décide"),
                                    "goto": string("Identifiant du paragraphe où mène ce choix"),
                                },
                                "required": ["label", "goto"],
                                "additionalProperties": false,
                            },
                        },
                    },
                    "required": ["id", "event", "characters", "objects", "choices"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["title", "premise", "setting", "characters", "paragraphs"],
        "additionalProperties": false,
    })
}

async fn make_plan<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    observer: &mut O,
    tally: &mut Tally,
) -> Result<std::result::Result<GamePlan, String>> {
    observer.step(Step::Planning);
    let base = prompts::game_plan(boxes, options);
    let schema = game_schema();
    let check = |plan: &GamePlan| {
        let story = plan.story_plan();
        let mut errors = validate_graph(plan);
        errors.extend(validate(&story, boxes));
        errors.extend(options.saga.validate(&story, boxes));
        errors
    };

    let first = generator
        .generate_json(&base, "story_game", &schema)
        .await?;
    tally.add(&first);
    let errors = match parse_json::<GamePlan>(&first.text) {
        Ok(plan) => {
            let errors = check(&plan);
            if errors.is_empty() {
                return Ok(Ok(plan));
            }
            errors
        }
        Err(err) => vec![format!("ta réponse n'est pas un JSON valide ({err})")],
    };

    observer.step(Step::PlanRejected(&errors));
    let retry = prompts::plan_fix(&base, &first.text, &errors);
    let second = generator
        .generate_json(&retry, "story_game", &schema)
        .await?;
    tally.add(&second);
    let plan: GamePlan = match parse_json(&second.text) {
        Ok(plan) => plan,
        Err(err) => return Ok(Err(format!("plan corrigé illisible ({err})"))),
    };
    let graph = validate_graph(&plan);
    if !graph.is_empty() {
        // A broken graph can't be repaired by dropping references.
        return Ok(Err(format!("choix incohérents ({})", graph.join(" ; "))));
    }
    let errors = check(&plan);
    if errors.is_empty() {
        return Ok(Ok(plan));
    }
    observer.step(Step::PlanRepaired(&errors));
    let story = options
        .saga
        .restore_names(sanitize(plan.story_plan(), boxes), boxes);
    Ok(Ok(plan.with_story_plan(story)))
}

/// Normalized for matching an excerpt quoted by the reviewer.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) async fn write<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    writer: WriterOptions,
    observer: &mut O,
) -> Result<Written> {
    let mut tally = Tally::default();

    // 1. Plan: the graph of paragraphs
    let plan = match make_plan(generator, boxes, options, observer, &mut tally).await? {
        Ok(plan) => plan,
        Err(reason) => {
            observer.step(Step::PlanAbandoned(&reason));
            return fall_back_to_direct(generator, boxes, options, observer, tally).await;
        }
    };
    let words = options.target_words as usize / plan.longest_path().max(1);
    let (plan, paragraphs, parents) = plan.ordered();
    let story_plan = plan.story_plan();
    observer.step(Step::PlanReady(&story_plan));

    // 2. Draft each paragraph, knowing the way that leads to it
    let total = paragraphs.len();
    let mut texts: Vec<String> = Vec::with_capacity(total);
    for index in 0..total {
        observer.step(Step::Drafting {
            part: index + 1,
            total,
        });
        let mut path = Vec::new();
        let mut at = parents[index];
        while let Some(p) = at {
            path.push(p);
            at = parents[p];
        }
        let so_far: Vec<&str> = path.iter().rev().map(|&p| texts[p].as_str()).collect();
        let choices: Vec<&str> = paragraphs[index]
            .choices
            .iter()
            .map(|(label, _)| label.as_str())
            .collect();
        let prompt = prompts::paragraph(
            boxes,
            options,
            &story_plan,
            &so_far.join("\n\n"),
            index,
            &choices,
            words.max(50),
        );
        let part = generator.generate(&prompt, |_| {}).await?;
        tally.add(&part);
        texts.push(part.text.trim().to_owned());
    }

    // 3. Edit: one review of every paragraph, then a rewrite of each
    //    paragraph an issue quotes
    let mut issues = Vec::new();
    if writer.review {
        observer.step(Step::Reviewing);
        let all: Vec<String> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| format!("§{}\n{t}", i + 1))
            .collect();
        let prompt = prompts::review(boxes, options, &all.join("\n\n"));
        let report = generator
            .generate_json(&prompt, "story_review", &review_schema())
            .await?;
        tally.add(&report);
        issues = match parse_json::<Review>(&report.text) {
            Ok(review) => review.issues,
            Err(_) => {
                observer.step(Step::ReviewUnreadable);
                Vec::new()
            }
        };
        if !issues.is_empty() {
            observer.step(Step::Revising(&issues));
            for text in &mut texts {
                let haystack = normalized(text);
                let own: Vec<_> = issues
                    .iter()
                    .filter(|i| {
                        let excerpt = normalized(&i.excerpt);
                        !excerpt.is_empty() && haystack.contains(&excerpt)
                    })
                    .cloned()
                    .collect();
                if own.is_empty() {
                    continue;
                }
                let prompt = prompts::revise(boxes, options, text, &own, None);
                let revised = generator.generate(&prompt, |_| {}).await?;
                tally.add(&revised);
                *text = revised.text.trim().to_owned();
            }
        }
    }

    let text = markdown(&story_plan, &texts, &paragraphs, boxes, options, writer);
    observer.text(&text);
    Ok(Written {
        generation: Generation {
            text,
            model: tally.model,
            truncated: tally.truncated,
            output_tokens: tally.output_tokens,
        },
        plan: Some(story_plan),
        scenes: texts,
        staged: writer.scene_setup,
        paragraphs,
        issues,
        calls: tally.calls,
    })
}

/// The gamebook: numbered paragraphs, each followed by its choices.
fn markdown(
    plan: &StoryPlan,
    texts: &[String],
    paragraphs: &[Paragraph],
    boxes: &Collection,
    options: &StoryOptions,
    writer: WriterOptions,
) -> String {
    let labels = Labels::new(options);
    let mut out = format!("# {}\n\n> 🎲 {}\n", plan.title.trim(), labels.interactive);
    for (i, ((text, paragraph), step)) in
        texts.iter().zip(paragraphs).zip(&plan.outline).enumerate()
    {
        out.push_str(&format!("\n## {}\n\n", i + 1));
        if writer.scene_setup {
            let setup = scene_setup(step, plan, boxes);
            if !setup.is_empty() {
                out.push_str(&scene_setup_markdown(&setup, options));
                out.push_str("\n\n");
            }
        }
        out.push_str(text);
        out.push_str("\n\n");
        if paragraph.choices.is_empty() {
            out.push_str(&format!("**— {} —**\n", labels.the_end));
        } else {
            out.push_str(&format!("**{}**\n\n", labels.choose));
            for (label, number) in &paragraph.choices {
                out.push_str(&format!("- {label} → **{} {number}**\n", labels.go_to));
            }
        }
    }
    out.push('\n');
    out.push_str(&in_your_box(plan, boxes, options));
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn node(id: &str, choices: &[&str]) -> Node {
        Node {
            id: id.into(),
            event: format!("Paragraphe {id}"),
            characters: vec!["Léon".into()],
            objects: vec![],
            choices: choices
                .iter()
                .map(|g| Choice {
                    label: format!("Aller en {g}"),
                    goto: (*g).into(),
                })
                .collect(),
        }
    }

    /// p1 → (p2, p3); p2 → (p4, p5); p3 → (p5, p4); p4, p5 endings.
    pub(crate) fn game() -> GamePlan {
        let base = crate::writer::plan::tests::valid_plan();
        GamePlan {
            title: base.title,
            premise: base.premise,
            setting: base.setting,
            characters: base.characters,
            paragraphs: vec![
                node("p1", &["p3", "p2"]),
                node("p2", &["p4", "p5"]),
                node("p3", &["p5", "p4"]),
                node("p4", &[]),
                node("p5", &[]),
            ],
        }
    }

    #[test]
    fn valid_graph_has_no_errors() {
        assert_eq!(validate_graph(&game()), Vec::<String>::new());
        assert_eq!(game().longest_path(), 3);
    }

    #[test]
    fn detects_broken_graphs() {
        let mut plan = game();
        plan.paragraphs[1].choices.truncate(1); // p2: one choice
        plan.paragraphs[2].choices[0].goto = "p9".into(); // unknown
        plan.paragraphs[3].choices = node("x", &["p1", "p5"]).choices; // p4 → p1: a loop
        plan.paragraphs.push(node("p6", &[])); // unreachable
        let errors = validate_graph(&plan);
        for expected in [
            "qu'un choix",
            "« p9 », qui n'existe pas",
            "boucle qui repasse par « p1 »",
            "aucun choix ne mène au paragraphe « p6 »",
        ] {
            assert!(
                errors.iter().any(|e| e.contains(expected)),
                "{expected}: {errors:#?}"
            );
        }

        let mut flat = game();
        flat.paragraphs.truncate(2);
        flat.paragraphs[0].choices.clear();
        let errors = validate_graph(&flat);
        assert!(errors.iter().any(|e| e.contains("au moins 4 paragraphes")));
        assert!(errors.iter().any(|e| e.contains("premier paragraphe")));
    }

    #[test]
    fn paragraphs_are_numbered_in_reading_order() {
        let (plan, paragraphs, parents) = game().ordered();
        let ids: Vec<&str> = plan.paragraphs.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["p1", "p3", "p2", "p5", "p4"]);
        assert_eq!(
            paragraphs[0].choices,
            [("Aller en p3".to_owned(), 2), ("Aller en p2".to_owned(), 3)]
        );
        assert!(paragraphs[4].choices.is_empty());
        // p5 is first reached from p3 (2nd paragraph), p4 too.
        assert_eq!(parents, [None, Some(0), Some(0), Some(1), Some(1)]);
    }
}
