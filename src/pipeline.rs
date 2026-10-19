//! Retrieve → Augment → Generate, wired together.

use crate::augment::StoryOptions;
use crate::error::Result;
use crate::generator::StoryGenerator;
use crate::model::{Collection, SetNumber};
use crate::retriever::InventorySource;
use crate::writer::{Observer, WriterOptions, Written, write};

pub struct Story {
    /// The boxes the story is played with.
    pub collection: Collection,
    pub written: Written,
}

pub struct RagPipeline<S, G> {
    source: S,
    generator: G,
}

impl<S: InventorySource, G: StoryGenerator> RagPipeline<S, G> {
    pub fn new(source: S, generator: G) -> Self {
        Self { source, generator }
    }

    /// One story mixing every set of `sets` (duplicates ignored). Sets are
    /// fetched one after the other, to go easy on PlaymoDB.
    pub async fn run<O: Observer>(
        &self,
        sets: &[SetNumber],
        options: &StoryOptions,
        writer: WriterOptions,
        observer: &mut O,
    ) -> Result<Story> {
        let mut inventories = Vec::with_capacity(sets.len());
        for (i, set) in sets.iter().enumerate() {
            if !sets[..i].contains(set) {
                inventories.push(self.source.fetch(set).await?);
            }
        }
        let collection = Collection::new(inventories);
        let written = write(&self.generator, &collection, options, writer, observer).await?;
        Ok(Story {
            collection,
            written,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::augment::Prompt;
    use crate::generator::Generation;
    use crate::model::{Part, SetInventory};
    use crate::writer::{Step, WritingMethod};

    /// 3666 is a castle, any other set the pirate ship. Counts fetches.
    #[derive(Default)]
    struct FakeSource {
        fetched: Mutex<Vec<String>>,
    }
    impl InventorySource for FakeSource {
        async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
            self.fetched.lock().unwrap().push(set.to_string());
            let part = |number: &str, name: &str, category: &str| Part {
                number: number.into(),
                name: name.into(),
                color: None,
                quantity: Some(1),
                category: category.into(),
                components: vec![],
                image: None,
            };
            if set.as_str() == "3666" {
                return Ok(SetInventory {
                    set_number: set.clone(),
                    name: "Kings Large Castle".into(),
                    theme: Some("Knights".into()),
                    focus: None,
                    release_date: None,
                    parts: vec![
                        part("30 00 003", "Knight", "Klicky"),
                        part("30 00 004", "Horse", "Animal"),
                    ],
                    source_url: "test".into(),
                });
            }
            Ok(SetInventory {
                set_number: set.clone(),
                name: "Pirate Ship".into(),
                theme: Some("Pirates".into()),
                focus: None,
                release_date: None,
                parts: vec![
                    part("30 00 001", "Pirate captain", "Klicky"),
                    part("30 00 002", "Treasure chest", "Furniture"),
                ],
                source_url: "test".into(),
            })
        }
    }

    /// Answers with scripted texts, in order, and records every prompt.
    /// An empty script echoes the user prompt back.
    #[derive(Default)]
    struct ScriptedGenerator {
        script: Mutex<VecDeque<String>>,
        prompts: Mutex<Vec<(&'static str, Prompt)>>,
    }

    impl ScriptedGenerator {
        fn new(script: &[String]) -> Self {
            Self {
                script: Mutex::new(script.iter().cloned().collect()),
                ..Self::default()
            }
        }

        fn answer(&self, kind: &'static str, prompt: &Prompt) -> Generation {
            self.prompts.lock().unwrap().push((kind, prompt.clone()));
            let text = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| prompt.user.clone());
            Generation {
                text,
                model: "scripted".into(),
                truncated: false,
                output_tokens: Some(10),
            }
        }
    }

    impl StoryGenerator for ScriptedGenerator {
        async fn generate<F>(&self, prompt: &Prompt, mut on_text: F) -> Result<Generation>
        where
            F: FnMut(&str) + Send,
        {
            let generation = self.answer("text", prompt);
            on_text(&generation.text);
            Ok(generation)
        }

        async fn generate_json(&self, prompt: &Prompt, _: &str, _: &Value) -> Result<Generation> {
            Ok(self.answer("json", prompt))
        }
    }

    #[derive(Default)]
    struct Recorder {
        text: String,
        steps: Vec<String>,
    }

    impl Observer for Recorder {
        fn text(&mut self, chunk: &str) {
            self.text.push_str(chunk);
        }
        fn step(&mut self, step: Step<'_>) {
            self.steps.push(match step {
                Step::Planning => "plan".into(),
                Step::PlanRejected(_) => "plan-rejected".into(),
                Step::PlanRepaired(_) => "plan-repaired".into(),
                Step::PlanAbandoned(_) => "plan-abandoned".into(),
                Step::PlanReady(_) => "plan-ready".into(),
                Step::Drafting { part, total } => format!("draft {part}/{total}"),
                Step::Reviewing => "review".into(),
                Step::ReviewUnreadable => "review-unreadable".into(),
                Step::Revising(_) => "revise".into(),
                Step::SceneSetupDropped => "setup-dropped".into(),
            });
        }
    }

    fn plan_json(figure: &str) -> String {
        json!({
            "title": "Le trésor de Barbe-Rousse",
            "premise": "Un capitaine cherche son coffre.",
            "setting": "Le pont du navire.",
            "characters": [{"name": "Barbe-Rousse", "figure": figure, "portrait": "capitaine"}],
            "outline": [
                {"event": "Le coffre a disparu.", "characters": ["Barbe-Rousse"], "objects": []},
                {"event": "Il le retrouve.", "characters": ["Barbe-Rousse"], "objects": ["30 00 002"]},
            ],
        })
        .to_string()
    }

    async fn run(generator: &ScriptedGenerator, writer: WriterOptions) -> (Story, Recorder) {
        run_sets(generator, writer, &["5135"]).await
    }

    async fn run_sets(
        generator: &ScriptedGenerator,
        writer: WriterOptions,
        sets: &[&str],
    ) -> (Story, Recorder) {
        let pipeline = RagPipeline::new(FakeSource::default(), generator);
        let sets: Vec<SetNumber> = sets.iter().map(|s| SetNumber::parse(s).unwrap()).collect();
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(&sets, &StoryOptions::default(), writer, &mut recorder)
            .await
            .unwrap();
        (story, recorder)
    }

    #[tokio::test]
    async fn direct_method_streams_one_call_with_the_inventory() {
        let generator = ScriptedGenerator::default();
        let writer = WriterOptions {
            method: WritingMethod::Direct,
            ..WriterOptions::default()
        };
        let (story, recorder) = run(&generator, writer).await;

        let text = &story.written.generation.text;
        assert!(text.contains("1× Pirate captain [réf. 30 00 001]"));
        assert!(text.contains("set Playmobil 5135 « Pirate Ship »"));
        assert_eq!(recorder.text, *text);
        assert_eq!(story.written.calls, 1);
        assert!(recorder.steps.is_empty());
    }

    #[tokio::test]
    async fn planned_method_plans_drafts_and_reviews() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Barbe-Rousse cherche partout.".into(),
            "Il ouvre enfin le coffre.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(
            recorder.steps,
            ["plan", "plan-ready", "draft 1/2", "draft 2/2", "review"]
        );
        let text = &story.written.generation.text;
        assert!(text.starts_with("# Le trésor de Barbe-Rousse\n\n"));
        assert!(text.contains("Barbe-Rousse cherche partout.\n\nIl ouvre enfin le coffre."));
        assert!(text.contains("## Dans ta boîte"));
        assert!(text.contains("- Barbe-Rousse — Pirate captain"));
        assert!(text.contains("- 1× Treasure chest"));
        assert_eq!(
            recorder.text, *text,
            "planned text is emitted once, at the end"
        );
        assert_eq!(story.written.calls, 4);
        assert_eq!(story.written.generation.output_tokens, Some(40));

        // The 2nd draft sees the plan, the story so far and its own objects.
        let prompts = generator.prompts.lock().unwrap();
        let draft2 = &prompts[2].1.user;
        assert!(draft2.contains("<plan>"));
        assert!(draft2.contains("Barbe-Rousse cherche partout."));
        assert!(draft2.contains("Objets de la boîte à utiliser : Treasure chest"));
        assert!(draft2.contains("conclus l'histoire"));
    }

    fn staged() -> WriterOptions {
        WriterOptions {
            scene_setup: true,
            ..WriterOptions::default()
        }
    }

    #[tokio::test]
    async fn scene_setup_precedes_each_part() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Barbe-Rousse cherche partout.".into(),
            "Il ouvre enfin le coffre.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, _) = run(&generator, staged()).await;

        let text = &story.written.generation.text;
        assert!(text.contains(
            "# Le trésor de Barbe-Rousse\n\n\
             > 🧱 **Mise en place** : Barbe-Rousse (Pirate captain)\n\n\
             Barbe-Rousse cherche partout.\n\n\
             > 🧱 **Mise en place** : Barbe-Rousse (Pirate captain) ; Treasure chest\n\n\
             Il ouvre enfin le coffre.\n\n## Dans ta boîte"
        ));
        assert!(story.written.staged);
        assert_eq!(story.written.scenes.len(), 2);
    }

    fn one_issue() -> String {
        json!({"issues": [{"excerpt": "un cheval", "problem": "pas de cheval", "fix": "à pied"}]})
            .to_string()
    }

    #[tokio::test]
    async fn revision_keeps_the_scenes_apart() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Il part avec un cheval.".into(),
            "Il trouve le coffre.".into(),
            one_issue(),
            "Il part à pied.\n\n* * *\n\nIl trouve le coffre.".into(),
        ]);
        let (story, recorder) = run(&generator, staged()).await;

        assert_eq!(recorder.steps.last().unwrap(), "revise");
        assert_eq!(
            story.written.scenes,
            ["Il part à pied.", "Il trouve le coffre."]
        );
        let text = &story.written.generation.text;
        assert!(!text.contains("* * *"));
        assert_eq!(text.matches("Mise en place").count(), 2);
        let prompts = generator.prompts.lock().unwrap();
        let revise = &prompts[4].1.user;
        assert!(revise.contains("Il part avec un cheval.\n\n* * *\n\nIl trouve le coffre."));
        assert!(revise.contains("en 2 parties séparées par des lignes « * * * »"));
    }

    #[tokio::test]
    async fn revision_that_merges_the_scenes_drops_the_setups() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Il part avec un cheval.".into(),
            "Il trouve le coffre.".into(),
            one_issue(),
            "Il part à pied.\n\nIl trouve le coffre.".into(),
        ]);
        let (story, recorder) = run(&generator, staged()).await;

        assert_eq!(recorder.steps.last().unwrap(), "setup-dropped");
        assert!(!story.written.staged);
        let text = &story.written.generation.text;
        assert!(!text.contains("Mise en place"));
        assert!(text.contains("Il part à pied.\n\nIl trouve le coffre."));
    }

    #[tokio::test]
    async fn unfaithful_plan_is_asked_again_with_its_errors() {
        let generator = ScriptedGenerator::new(&[
            plan_json("99 99 999"),
            plan_json("30 00 001"),
            "Partie 1.".into(),
            "Partie 2.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps[..3], ["plan", "plan-rejected", "plan-ready"]);
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[1]
                .1
                .user
                .contains("« 99 99 999 » n'existe pas dans la boîte")
        );
        assert_eq!(story.written.calls, 5);
    }

    #[tokio::test]
    async fn several_sets_make_one_story() {
        let plan = json!({
            "title": "Le chevalier et le pirate",
            "premise": "Un chevalier rencontre un pirate.",
            "setting": "Le château au bord de la mer.",
            "characters": [
                {"name": "Arthur", "figure": "30 00 003", "portrait": "chevalier"},
                {"name": "Barbe-Rousse", "figure": "30 00 001", "portrait": "pirate"},
            ],
            "outline": [
                {"event": "Ils se rencontrent.", "characters": ["Arthur", "Barbe-Rousse"], "objects": ["30 00 004"]},
                {"event": "Ils partagent le coffre.", "characters": ["Arthur", "Barbe-Rousse"], "objects": ["30 00 002"]},
            ],
        });
        let generator = ScriptedGenerator::new(&[
            plan.to_string(),
            "Partie 1.".into(),
            "Partie 2.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let pipeline = RagPipeline::new(FakeSource::default(), &generator);
        let sets: Vec<SetNumber> = ["3666", "5135", "3666"]
            .iter()
            .map(|s| SetNumber::parse(s).unwrap())
            .collect();
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(
                &sets,
                &StoryOptions::default(),
                WriterOptions::default(),
                &mut recorder,
            )
            .await
            .unwrap();

        // Duplicates fetched once, in order.
        assert_eq!(*pipeline.source.fetched.lock().unwrap(), ["3666", "5135"]);
        assert_eq!(story.collection.sets.len(), 2);
        assert_eq!(
            recorder.steps[1], "plan-ready",
            "figures of both boxes accepted"
        );

        let prompts = generator.prompts.lock().unwrap();
        let plan_prompt = &prompts[0].1.user;
        assert_eq!(plan_prompt.matches("<set numero=").count(), 2);
        assert!(plan_prompt.contains(
            "les 2 sets Playmobil décrits ci-dessus, réunis : 3666 « Kings Large Castle », \
             5135 « Pirate Ship »"
        ));

        let text = &story.written.generation.text;
        assert!(text.contains("- Arthur — Knight (boîte 3666)\n"));
        assert!(text.contains("- Barbe-Rousse — Pirate captain (boîte 5135)\n"));
        assert!(text.contains("- 1× Horse (boîte 3666)\n- 1× Treasure chest (boîte 5135)\n"));
    }

    #[tokio::test]
    async fn unusable_plan_falls_back_to_one_call() {
        // What qwen2.5:3b did on set 3799: an unfaithful plan, then a
        // "corrected" plan that isn't valid (no title).
        let generator = ScriptedGenerator::new(&[
            plan_json("99 99 999"),
            r#"{"characters": [], "outline": []}"#.into(),
            "# Une histoire\n\nIl était une fois…".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps, ["plan", "plan-rejected", "plan-abandoned"]);
        assert_eq!(
            story.written.generation.text,
            "# Une histoire\n\nIl était une fois…"
        );
        assert_eq!(recorder.text, story.written.generation.text);
        assert!(story.written.plan.is_none());
        assert_eq!(story.written.calls, 3);
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[2].1.system.contains("Dans ta boîte"),
            "direct prompt"
        );
    }

    #[tokio::test]
    async fn unreadable_review_keeps_the_draft() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Partie 1.".into(),
            "Partie 2.".into(),
            "Tout est fidèle, bravo !".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps.last().unwrap(), "review-unreadable");
        assert!(
            story
                .written
                .generation
                .text
                .contains("Partie 1.\n\nPartie 2.")
        );
        assert!(story.written.issues.is_empty());
    }

    #[tokio::test]
    async fn review_issues_trigger_one_revision() {
        let issues = json!({"issues": [{
            "excerpt": "trois chevaux",
            "problem": "la boîte ne contient pas de cheval",
            "fix": "retirer les chevaux",
        }]});
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Il part avec trois chevaux.".into(),
            "Il trouve le coffre.".into(),
            issues.to_string(),
            "Il part à pied.\n\nIl trouve le coffre.".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps.last().unwrap(), "revise");
        assert_eq!(story.written.issues.len(), 1);
        let text = &story.written.generation.text;
        assert!(text.contains("Il part à pied."));
        assert!(!text.contains("chevaux"));
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[4]
                .1
                .user
                .contains("« trois chevaux » : la boîte ne contient pas de cheval")
        );
    }
}
