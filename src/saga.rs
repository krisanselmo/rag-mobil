//! Sagas: stories that follow one another. The episodes told so far are kept
//! in a JSON file (`--saga`); the next plan sees them, keeps the names of the
//! recurring characters (checked by code, figure by figure) and doesn't
//! retell the same plot.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{RagError, Result};
use crate::model::{Collection, reference_key};
use crate::writer::plan::{PlannedCharacter, StoryPlan, find_part, part_label};

/// The episodes told so far, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saga {
    pub episodes: Vec<Episode>,
}

/// What the next episodes need to know about one story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    /// Set numbers the episode was played with.
    pub sets: Vec<String>,
    pub title: String,
    pub premise: String,
    pub characters: Vec<PlannedCharacter>,
    /// What happened, step by step (empty for an interactive story, whose
    /// course depends on the reader).
    #[serde(default)]
    pub events: Vec<String>,
}

impl Episode {
    pub fn new(collection: &Collection, plan: &StoryPlan, events: bool) -> Self {
        Self {
            sets: collection
                .sets
                .iter()
                .map(|s| s.set_number.to_string())
                .collect(),
            title: plan.title.trim().to_owned(),
            premise: plan.premise.trim().to_owned(),
            characters: plan.characters.clone(),
            events: if events {
                plan.outline.iter().map(|s| s.event.clone()).collect()
            } else {
                Vec::new()
            },
        }
    }
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

impl Saga {
    /// The saga in `path`, or a new one if the file doesn't exist yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(RagError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json + "\n").map_err(|source| RagError::Io {
            path: path.to_owned(),
            source,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.episodes.is_empty()
    }

    /// Number of the next episode.
    pub fn next_number(&self) -> usize {
        self.episodes.len() + 1
    }

    /// Every recurring character, latest episode first: (name, figure
    /// reference, portrait), each name once.
    fn recurring(&self) -> Vec<&PlannedCharacter> {
        let mut seen = HashSet::new();
        self.episodes
            .iter()
            .rev()
            .flat_map(|e| &e.characters)
            .filter(|c| seen.insert(key(&c.name)))
            .collect()
    }

    /// The names each figure (by reference key) had in the saga.
    fn names_by_figure(&self) -> HashMap<String, Vec<&str>> {
        let mut names: HashMap<String, Vec<&str>> = HashMap::new();
        for character in self.recurring() {
            names
                .entry(reference_key(&character.figure))
                .or_default()
                .push(character.name.trim());
        }
        names
    }

    /// The previous episodes, for the plan prompt (empty for a first one).
    pub fn context(&self, collection: &Collection) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("<episodes_precedents>\n");
        for (i, episode) in self.episodes.iter().enumerate() {
            let _ = writeln!(
                out,
                "Épisode {} « {} » : {}",
                i + 1,
                episode.title,
                episode.premise
            );
            for event in &episode.events {
                let _ = writeln!(out, "- {event}");
            }
        }
        let (present, absent): (Vec<_>, Vec<_>) = self
            .recurring()
            .into_iter()
            .partition(|c| find_part(collection, &c.figure).is_some());
        if !present.is_empty() {
            out.push_str("Personnages déjà connus, dont la figurine est dans la boîte :\n");
            for c in present {
                let figure = find_part(collection, &c.figure).expect("partitioned");
                let _ = writeln!(
                    out,
                    "- {} : figurine {} [réf. {}] ; {}",
                    c.name.trim(),
                    part_label(figure),
                    figure.number,
                    c.portrait
                );
            }
        }
        if !absent.is_empty() {
            let names: Vec<&str> = absent.iter().map(|c| c.name.trim()).collect();
            let _ = writeln!(
                out,
                "Personnages déjà connus dont la figurine n'est pas dans la boîte (ils ne \
                 peuvent être qu'évoqués) : {}",
                names.join(", ")
            );
        }
        out.push_str("</episodes_precedents>");
        out
    }

    /// The request that goes with [`Saga::context`].
    pub fn request(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        format!(
            "Cette histoire est l'épisode {} d'une saga : elle fait suite aux épisodes \
             décrits dans <episodes_precedents>, sans en répéter l'intrigue. Un personnage \
             déjà connu garde son nom et son caractère, et reste joué par la même \
             figurine ; une autre figurine reçoit un nouveau nom.",
            self.next_number()
        )
    }

    /// Recurring characters renamed, or names given to another figure, as
    /// messages the model can act on. Empty = consistent with the saga.
    pub fn validate(&self, plan: &StoryPlan, collection: &Collection) -> Vec<String> {
        let mut errors = Vec::new();
        let by_figure = self.names_by_figure();
        let by_name: HashMap<String, String> = self
            .recurring()
            .into_iter()
            .map(|c| (key(&c.name), reference_key(&c.figure)))
            .collect();
        for c in &plan.characters {
            let Some(part) = find_part(collection, &c.figure) else {
                continue; // reported by the plan validation
            };
            let figure = reference_key(&part.number);
            if let Some(former) = by_name.get(&key(&c.name))
                && *former != figure
            {
                errors.push(format!(
                    "« {} » était joué par une autre figurine dans les épisodes précédents : \
                     donne un autre nom à la figurine {} ({})",
                    c.name, part.number, part.name
                ));
            }
        }
        for (figure, names) in &by_figure {
            let players: Vec<&PlannedCharacter> = plan
                .characters
                .iter()
                .filter(|c| {
                    find_part(collection, &c.figure)
                        .is_some_and(|p| reference_key(&p.number) == *figure)
                })
                .collect();
            let unused: Vec<&&str> = names
                .iter()
                .filter(|n| !players.iter().any(|c| key(&c.name) == key(n)))
                .collect();
            for (c, name) in players
                .iter()
                .filter(|c| !names.iter().any(|n| key(n) == key(&c.name)))
                .zip(unused)
            {
                errors.push(format!(
                    "la figurine {} s'appelait « {name} » dans les épisodes précédents : \
                     garde ce nom au lieu de « {} »",
                    c.figure, c.name
                ));
            }
        }
        errors
    }

    /// Last resort: gives recurring figures back their names (in the
    /// characters and the steps), when the name is free.
    pub fn restore_names(&self, mut plan: StoryPlan, collection: &Collection) -> StoryPlan {
        let by_figure = self.names_by_figure();
        let mut renames: Vec<(String, String)> = Vec::new();
        for i in 0..plan.characters.len() {
            let Some(part) = find_part(collection, &plan.characters[i].figure) else {
                continue;
            };
            let Some(names) = by_figure.get(&reference_key(&part.number)) else {
                continue;
            };
            let current = key(&plan.characters[i].name);
            if names.iter().any(|n| key(n) == current) {
                continue;
            }
            let free = names
                .iter()
                .find(|n| !plan.characters.iter().any(|c| key(&c.name) == key(n)));
            if let Some(name) = free {
                renames.push((current, (*name).to_owned()));
                plan.characters[i].name = (*name).to_owned();
            }
        }
        for step in &mut plan.outline {
            for name in &mut step.characters {
                if let Some((_, new)) = renames.iter().find(|(old, _)| *old == key(name)) {
                    name.clone_from(new);
                }
            }
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::plan::tests::{inventory, valid_plan};

    fn saga() -> Saga {
        let boxes: Collection = inventory().into();
        let mut first = valid_plan();
        first.characters[0].name = "Gaspard".into(); // the trumpeter
        Saga {
            episodes: vec![Episode::new(&boxes, &first, true)],
        }
    }

    #[test]
    fn episodes_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("rag-mobil-saga-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saga.json");
        assert!(Saga::load(&path).unwrap().is_empty(), "no file: a new saga");
        let saga = saga();
        saga.save(&path).unwrap();
        assert_eq!(Saga::load(&path).unwrap(), saga);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_recalls_episodes_and_characters() {
        let mut boxes = inventory();
        boxes.parts.remove(1); // the lady in red is gone
        let saga = saga();
        let context = saga.context(&boxes.into());
        assert!(context.starts_with(
            "<episodes_precedents>\nÉpisode 1 « Le cheval perdu » : Le cheval du château a \
             disparu.\n- Léon sonne l'alerte.\n"
        ));
        assert!(context.contains(
            "- Gaspard : figurine Castle Trumpeter [réf. 30 00 496] ; trompettiste joyeux\n"
        ));
        assert!(context.contains("ne peuvent être qu'évoqués) : Rose\n"));
        assert!(saga.request().contains("l'épisode 2 d'une saga"));
        assert!(Saga::default().context(&inventory().into()).is_empty());
    }

    #[test]
    fn recurring_figures_keep_their_names() {
        let boxes: Collection = inventory().into();
        let saga = saga();
        let mut plan = valid_plan(); // the trumpeter is "Léon" again
        let errors = saga.validate(&plan, &boxes);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        assert!(errors[0].contains("s'appelait « Gaspard »"));

        plan.characters[0].name = "Gaspard".into();
        for step in &mut plan.outline {
            for name in &mut step.characters {
                if name.eq_ignore_ascii_case("léon") {
                    *name = "Gaspard".into();
                }
            }
        }
        assert!(saga.validate(&plan, &boxes).is_empty());

        // "Rose" given to the trumpeter: a name taken from another figure.
        plan.characters[0].name = "Rose".into();
        plan.characters[1].name = "Lili".into();
        let errors = saga.validate(&plan, &boxes);
        assert!(errors.iter().any(|e| e.contains("« Rose » était joué")));
    }

    #[test]
    fn restore_names_repairs_the_plan() {
        let boxes: Collection = inventory().into();
        let saga = saga();
        let plan = saga.restore_names(valid_plan(), &boxes);
        assert_eq!(plan.characters[0].name, "Gaspard");
        assert_eq!(plan.outline[0].characters, ["Gaspard"]);
        assert_eq!(plan.outline[1].characters, ["Rose", "Gaspard"]);
        assert!(saga.validate(&plan, &boxes).is_empty());
    }
}
