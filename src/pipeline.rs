//! Retrieve → Augment → Generate, wired together.

use crate::augment::StoryOptions;
use crate::error::Result;
use crate::generator::StoryGenerator;
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;
use crate::writer::{Observer, WriterOptions, Written, write};

pub struct Story {
    pub inventory: SetInventory,
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

    pub async fn run<O: Observer>(
        &self,
        set: &SetNumber,
        options: &StoryOptions,
        writer: WriterOptions,
        observer: &mut O,
    ) -> Result<Story> {
        let inventory = self.source.fetch(set).await?;
        let written = write(&self.generator, &inventory, options, writer, observer).await?;
        Ok(Story { inventory, written })
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
    use crate::model::Part;
    use crate::writer::{Step, WritingMethod};

    struct FakeSource;
    impl InventorySource for FakeSource {
        async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
            let part = |number: &str, name: &str, category: &str| Part {
                number: number.into(),
                name: name.into(),
                color: None,
                quantity: Some(1),
                category: category.into(),
                components: vec![],
            };
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
        let pipeline = RagPipeline::new(FakeSource, generator);
        let set = SetNumber::parse("5135").unwrap();
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(&set, &StoryOptions::default(), writer, &mut recorder)
            .await
            .unwrap();
        (story, recorder)
    }

    #[tokio::test]
    async fn direct_method_streams_one_call_with_the_inventory() {
        let generator = ScriptedGenerator::default();
        let writer = WriterOptions {
            method: WritingMethod::Direct,
            review: true,
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
