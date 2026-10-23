//! Retrieve → Augment → Generate, wired together.

use crate::augment::StoryOptions;
use crate::error::Result;
use crate::generator::StoryGenerator;
use crate::illustrate::Illustration;
use crate::model::{Collection, SetNumber};
use crate::retriever::InventorySource;
use crate::writer::{Observer, WriterOptions, Written, write};

pub struct Story {
    /// The boxes the story is played with.
    pub collection: Collection,
    pub written: Written,
    /// `--illustrate`: the cover picture, drawn after the story.
    pub illustration: Option<Illustration>,
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
            illustration: None,
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
    use crate::generator::{Generation, TokenUsage};
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
                usage: TokenUsage {
                    input: Some(100),
                    output: Some(10),
                    ..TokenUsage::default()
                },
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
                Step::CitationsDropped(refs) => format!("citations-dropped {}", refs.join(", ")),
            });
        }
    }

    fn plan_json(figure: &str) -> String {
        json!({
            "title": "Redbeard's treasure",
            "premise": "A captain looks for his chest.",
            "setting": "The deck of the ship.",
            "characters": [{"name": "Redbeard", "figure": figure, "portrait": "captain"}],
            "outline": [
                {"event": "The chest has disappeared.", "characters": ["Redbeard"], "objects": []},
                {"event": "He finds it again.", "characters": ["Redbeard"], "objects": ["30 00 002"]},
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
        run_options(generator, writer, sets, &StoryOptions::default()).await
    }

    async fn run_options(
        generator: &ScriptedGenerator,
        writer: WriterOptions,
        sets: &[&str],
        options: &StoryOptions,
    ) -> (Story, Recorder) {
        let pipeline = RagPipeline::new(FakeSource::default(), generator);
        let sets: Vec<SetNumber> = sets.iter().map(|s| SetNumber::parse(s).unwrap()).collect();
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(&sets, options, writer, &mut recorder)
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
        assert!(text.contains("1× Pirate captain [ref. 30 00 001]"));
        assert!(text.contains("Playmobil set 5135 \"Pirate Ship\""));
        assert_eq!(recorder.text, *text);
        assert_eq!(story.written.calls, 1);
        assert!(recorder.steps.is_empty());
    }

    #[tokio::test]
    async fn planned_method_plans_drafts_and_reviews() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Redbeard searches everywhere.".into(),
            "At last he opens the chest.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(
            recorder.steps,
            ["plan", "plan-ready", "draft 1/2", "draft 2/2", "review"]
        );
        let text = &story.written.generation.text;
        assert!(text.starts_with("# Redbeard's treasure\n\n"));
        assert!(text.contains("Redbeard searches everywhere.\n\nAt last he opens the chest."));
        assert!(text.contains("## In your box"));
        assert!(text.contains("- Redbeard — Pirate captain"));
        assert!(text.contains("- 1× Treasure chest"));
        assert_eq!(
            recorder.text, *text,
            "planned text is emitted once, at the end"
        );
        assert_eq!(story.written.calls, 4);
        assert_eq!(story.written.generation.usage.output, Some(40));
        assert_eq!(story.written.generation.usage.input, Some(400));

        // The 2nd draft sees the plan, the story so far and its own objects.
        let prompts = generator.prompts.lock().unwrap();
        let draft2 = &prompts[2].1.user;
        assert!(draft2.contains("<plan>"));
        assert!(draft2.contains("Redbeard searches everywhere."));
        assert!(draft2.contains("Objects from the box to use: Treasure chest"));
        assert!(draft2.contains("bring the story to a close"));
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
            "Redbeard searches everywhere.".into(),
            "At last he opens the chest.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, _) = run(&generator, staged()).await;

        let text = &story.written.generation.text;
        assert!(text.contains(
            "# Redbeard's treasure\n\n\
             > 🧱 **Set the scene**: Redbeard (Pirate captain)\n\n\
             Redbeard searches everywhere.\n\n\
             > 🧱 **Set the scene**: Redbeard (Pirate captain); Treasure chest\n\n\
             At last he opens the chest.\n\n## In your box"
        ));
        assert!(story.written.staged);
        assert_eq!(story.written.scenes.len(), 2);
    }

    fn one_issue() -> String {
        json!({"issues": [{"excerpt": "a horse", "problem": "no horse", "fix": "on foot"}]})
            .to_string()
    }

    #[tokio::test]
    async fn revision_keeps_the_scenes_apart() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "He leaves on a horse.".into(),
            "He finds the chest.".into(),
            one_issue(),
            "He leaves on foot.\n\n* * *\n\nHe finds the chest.".into(),
        ]);
        let (story, recorder) = run(&generator, staged()).await;

        assert_eq!(recorder.steps.last().unwrap(), "revise");
        assert_eq!(
            story.written.scenes,
            ["He leaves on foot.", "He finds the chest."]
        );
        let text = &story.written.generation.text;
        assert!(!text.contains("* * *"));
        assert_eq!(text.matches("Set the scene").count(), 2);
        let prompts = generator.prompts.lock().unwrap();
        let revise = &prompts[4].1.user;
        assert!(revise.contains("He leaves on a horse.\n\n* * *\n\nHe finds the chest."));
        assert!(revise.contains("in 2 parts separated by \"* * *\" lines"));
    }

    #[tokio::test]
    async fn revision_that_merges_the_scenes_drops_the_setups() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "He leaves on a horse.".into(),
            "He finds the chest.".into(),
            one_issue(),
            "He leaves on foot.\n\nHe finds the chest.".into(),
        ]);
        let (story, recorder) = run(&generator, staged()).await;

        assert_eq!(recorder.steps.last().unwrap(), "setup-dropped");
        assert!(!story.written.staged);
        let text = &story.written.generation.text;
        assert!(!text.contains("Set the scene"));
        assert!(text.contains("He leaves on foot.\n\nHe finds the chest."));
    }

    /// p1 → (p2, p3); p2 → (p4, p3); p3, p4 endings.
    fn game_json(figure: &str, goto: &str) -> String {
        json!({
            "title": "The lost chest",
            "premise": "A captain looks for his chest.",
            "setting": "The deck of the ship.",
            "characters": [{"name": "Redbeard", "figure": figure, "portrait": "captain"}],
            "paragraphs": [
                {"id": "p1", "event": "The chest has disappeared.", "characters": ["Redbeard"], "objects": [],
                 "choices": [{"label": "Search the deck", "goto": "p2"}, {"label": "Search the hold", "goto": goto}]},
                {"id": "p2", "event": "Nothing on the deck.", "characters": ["Redbeard"], "objects": [],
                 "choices": [{"label": "Climb the mast", "goto": "p4"}, {"label": "Go down", "goto": "p3"}]},
                {"id": "p3", "event": "The chest is in the hold.", "characters": ["Redbeard"], "objects": ["30 00 002"], "choices": []},
                {"id": "p4", "event": "He sees the chest from up there.", "characters": ["Redbeard"], "objects": ["30 00 002"], "choices": []},
            ],
        })
        .to_string()
    }

    fn interactive() -> WriterOptions {
        WriterOptions {
            method: WritingMethod::Interactive,
            ..WriterOptions::default()
        }
    }

    #[tokio::test]
    async fn interactive_story_is_a_gamebook() {
        let generator = ScriptedGenerator::new(&[
            game_json("30 00 001", "p3"),
            "The chest is gone!".into(),
            "The deck is empty.".into(),
            "The chest was in the hold.".into(),
            "From the top of the mast, he sees it.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, recorder) = run(&generator, interactive()).await;

        assert_eq!(
            recorder.steps,
            [
                "plan",
                "plan-ready",
                "draft 1/4",
                "draft 2/4",
                "draft 3/4",
                "draft 4/4",
                "review"
            ]
        );
        let written = &story.written;
        assert_eq!(written.calls, 6);
        // Reading order: p1, p2, p3 (2nd choice of p1), p4.
        assert_eq!(
            written.paragraphs[0].choices,
            [
                ("Search the deck".to_owned(), 2),
                ("Search the hold".to_owned(), 3)
            ]
        );
        assert_eq!(
            written.paragraphs[1].choices[0],
            ("Climb the mast".to_owned(), 4)
        );
        let text = &written.generation.text;
        assert!(text.starts_with("# The lost chest\n\n> 🎲 You choose"));
        assert!(text.contains(
            "## 1\n\nThe chest is gone!\n\n**What do you choose?**\n\n\
             - Search the deck → **go to 2**\n- Search the hold → **go to 3**\n"
        ));
        assert!(text.contains("## 3\n\nThe chest was in the hold.\n\n**— The End —**\n"));
        assert!(text.contains("## In your box"));

        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[0]
                .1
                .user
                .contains("interactive story in 6 to 9 paragraphs")
        );
        // p4 is reached through p1 then p2: both are in its context.
        let p4 = &prompts[4].1.user;
        assert!(p4.contains("<story_so_far>\nThe chest is gone!\n\nThe deck is empty.\n"));
        assert!(p4.contains("one of the endings of the story"));
        let p1 = &prompts[1].1.user;
        assert!(p1.contains("choose between: Search the deck; Search the hold"));
        // The review sees numbered paragraphs.
        assert!(prompts[5].1.user.contains("§1\nThe chest is gone!\n\n§2\n"));
    }

    #[tokio::test]
    async fn interactive_review_rewrites_only_the_quoted_paragraph() {
        let issues = json!({"issues": [{
            "excerpt": "with   his KEY", "problem": "no key", "fix": "without a key",
        }]});
        let generator = ScriptedGenerator::new(&[
            game_json("30 00 001", "p3"),
            "The chest is gone!".into(),
            "The deck is empty.".into(),
            "He opens the chest with his key.".into(),
            "From the top of the mast, he sees it.".into(),
            issues.to_string(),
            "He opens the chest by pulling hard.".into(),
        ]);
        let (story, recorder) = run(&generator, interactive()).await;

        assert_eq!(recorder.steps.last().unwrap(), "revise");
        assert_eq!(story.written.calls, 7);
        assert_eq!(
            story.written.scenes[2],
            "He opens the chest by pulling hard."
        );
        assert_eq!(story.written.scenes[1], "The deck is empty.");
    }

    #[tokio::test]
    async fn broken_graph_is_asked_again_then_abandoned() {
        let generator = ScriptedGenerator::new(&[
            game_json("30 00 001", "p9"),
            game_json("30 00 001", "p1"), // a loop
            "# A story\n\nOnce upon a time…".into(),
        ]);
        let (story, recorder) = run(&generator, interactive()).await;

        assert_eq!(recorder.steps, ["plan", "plan-rejected", "plan-abandoned"]);
        let prompts = generator.prompts.lock().unwrap();
        assert!(prompts[1].1.user.contains("\"p9\", which does not exist"));
        assert!(story.written.paragraphs.is_empty());
        assert!(story.written.plan.is_none());
    }

    #[tokio::test]
    async fn saga_episode_keeps_the_names() {
        use crate::saga::{Episode, Saga};
        let boxes: crate::model::Collection = FakeSource::default()
            .fetch(&SetNumber::parse("5135").unwrap())
            .await
            .unwrap()
            .into();
        let first: crate::writer::StoryPlan =
            serde_json::from_str(&plan_json("30 00 001")).unwrap();
        let options = StoryOptions {
            saga: Saga {
                episodes: vec![Episode::new(&boxes, &first, true)],
            },
            ..StoryOptions::default()
        };
        // The captain renamed: rejected, then named back.
        let renamed = plan_json("30 00 001").replace("Redbeard", "Captain Hook");
        let generator = ScriptedGenerator::new(&[
            renamed.clone(),
            renamed,
            "Part 1.".into(),
            "Part 2.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let pipeline = RagPipeline::new(FakeSource::default(), &generator);
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(
                &[SetNumber::parse("5135").unwrap()],
                &options,
                WriterOptions::default(),
                &mut recorder,
            )
            .await
            .unwrap();

        assert_eq!(
            recorder.steps[..4],
            ["plan", "plan-rejected", "plan-repaired", "plan-ready"]
        );
        let plan = story.written.plan.unwrap();
        assert_eq!(plan.characters[0].name, "Redbeard");
        assert_eq!(plan.outline[0].characters, ["Redbeard"]);
        let prompts = generator.prompts.lock().unwrap();
        let plan_prompt = &prompts[0].1.user;
        assert!(plan_prompt.contains(
            "<previous_episodes>\nEpisode 1 \"Redbeard's treasure\": A captain \
             looks for his chest.\n- The chest has disappeared.\n- He finds it again.\n"
        ));
        assert!(plan_prompt.contains("episode 2 of a saga"));
        assert!(prompts[1].1.user.contains("was called \"Redbeard\""));
    }

    #[tokio::test]
    async fn favorite_part_is_at_the_heart_of_the_story() {
        let options = StoryOptions {
            stars: vec!["3000002".into()],
            ..StoryOptions::default()
        };
        // The chest serves in one step out of two: rejected, then put back
        // in the first step by code.
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            plan_json("30 00 001"),
            "Part 1.".into(),
            "Part 2.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let pipeline = RagPipeline::new(FakeSource::default(), &generator);
        let mut recorder = Recorder::default();
        let story = pipeline
            .run(
                &[SetNumber::parse("5135").unwrap()],
                &options,
                WriterOptions::default(),
                &mut recorder,
            )
            .await
            .unwrap();

        assert_eq!(
            recorder.steps[..4],
            ["plan", "plan-rejected", "plan-repaired", "plan-ready"]
        );
        let plan = story.written.plan.unwrap();
        assert_eq!(plan.outline[0].objects, ["30 00 002"]);
        let prompts = generator.prompts.lock().unwrap();
        assert!(prompts[0].1.user.contains(
            "The child's favorite piece: Treasure chest [ref. 30 00 002]; give it a \
             central role"
        ));
        assert!(
            prompts[1]
                .1
                .user
                .contains("at least 2 steps out of 2, not 1")
        );
        assert!(
            prompts[2]
                .1
                .user
                .contains("Objects from the box to use: Treasure chest")
        );
    }

    #[tokio::test]
    async fn unfaithful_plan_is_asked_again_with_its_errors() {
        let generator = ScriptedGenerator::new(&[
            plan_json("99 99 999"),
            plan_json("30 00 001"),
            "Part 1.".into(),
            "Part 2.".into(),
            r#"{"issues": []}"#.into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps[..3], ["plan", "plan-rejected", "plan-ready"]);
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[1]
                .1
                .user
                .contains("\"99 99 999\" does not exist in the box")
        );
        assert_eq!(story.written.calls, 5);
    }

    #[tokio::test]
    async fn several_sets_make_one_story() {
        let plan = json!({
            "title": "The knight and the pirate",
            "premise": "A knight meets a pirate.",
            "setting": "The castle by the sea.",
            "characters": [
                {"name": "Arthur", "figure": "30 00 003", "portrait": "knight"},
                {"name": "Redbeard", "figure": "30 00 001", "portrait": "pirate"},
            ],
            "outline": [
                {"event": "They meet.", "characters": ["Arthur", "Redbeard"], "objects": ["30 00 004"]},
                {"event": "They share the chest.", "characters": ["Arthur", "Redbeard"], "objects": ["30 00 002"]},
            ],
        });
        let generator = ScriptedGenerator::new(&[
            plan.to_string(),
            "Part 1.".into(),
            "Part 2.".into(),
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
        assert_eq!(plan_prompt.matches("<set number=").count(), 2);
        assert!(plan_prompt.contains(
            "the 2 Playmobil sets described above, together: 3666 \"Kings Large Castle\", \
             5135 \"Pirate Ship\""
        ));

        let text = &story.written.generation.text;
        assert!(text.contains("- Arthur — Knight (box 3666)\n"));
        assert!(text.contains("- Redbeard — Pirate captain (box 5135)\n"));
        assert!(text.contains("- 1× Horse (box 3666)\n- 1× Treasure chest (box 5135)\n"));
    }

    #[tokio::test]
    async fn unusable_plan_falls_back_to_one_call() {
        // What qwen2.5:3b did on set 3799: an unfaithful plan, then a
        // "corrected" plan that isn't valid (no title).
        let generator = ScriptedGenerator::new(&[
            plan_json("99 99 999"),
            r#"{"characters": [], "outline": []}"#.into(),
            "# A story\n\nOnce upon a time…".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps, ["plan", "plan-rejected", "plan-abandoned"]);
        assert_eq!(
            story.written.generation.text,
            "# A story\n\nOnce upon a time…"
        );
        assert_eq!(recorder.text, story.written.generation.text);
        assert!(story.written.plan.is_none());
        assert_eq!(story.written.calls, 3);
        let prompts = generator.prompts.lock().unwrap();
        assert!(prompts[2].1.system.contains("In your box"), "direct prompt");
    }

    #[tokio::test]
    async fn unreadable_review_keeps_the_draft() {
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Part 1.".into(),
            "Part 2.".into(),
            "Everything is faithful, well done!".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps.last().unwrap(), "review-unreadable");
        assert!(story.written.generation.text.contains("Part 1.\n\nPart 2."));
        assert!(story.written.issues.is_empty());
    }

    #[tokio::test]
    async fn review_issues_trigger_one_revision() {
        let issues = json!({"issues": [{
            "excerpt": "three horses",
            "problem": "the box holds no horse",
            "fix": "remove the horses",
        }]});
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "He leaves with three horses.".into(),
            "He finds the chest.".into(),
            issues.to_string(),
            "He leaves on foot.\n\nHe finds the chest.".into(),
        ]);
        let (story, recorder) = run(&generator, WriterOptions::default()).await;

        assert_eq!(recorder.steps.last().unwrap(), "revise");
        assert_eq!(story.written.issues.len(), 1);
        let text = &story.written.generation.text;
        assert!(text.contains("He leaves on foot."));
        assert!(!text.contains("horses"));
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[4]
                .1
                .user
                .contains("\"three horses\": the box holds no horse")
        );
    }

    #[tokio::test]
    async fn later_review_rounds_check_the_revision() {
        let horses = json!({"issues": [{
            "excerpt": "three horses",
            "problem": "the box holds no horse",
            "fix": "remove the horses",
        }]});
        let cart = json!({"issues": [{
            "excerpt": "a cart",
            "problem": "the box holds no cart",
            "fix": "walk",
        }]});
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "He leaves with three horses.".into(),
            "He finds the chest.".into(),
            horses.to_string(),
            "He leaves in a cart.\n\nHe finds the chest.".into(),
            cart.to_string(),
            "He leaves on foot.\n\nHe finds the chest.".into(),
            json!({"issues": []}).to_string(),
        ]);
        let writer = WriterOptions {
            reviews: 3,
            ..WriterOptions::default()
        };
        let (story, recorder) = run(&generator, writer).await;

        // The third review finds nothing: no third rewrite.
        assert_eq!(
            recorder.steps[recorder.steps.len() - 5..],
            ["review", "revise", "review", "revise", "review"]
        );
        assert_eq!(story.written.calls, 1 + 2 + 3 * 2 - 1);
        assert_eq!(story.written.issues.len(), 2);
        assert!(story.written.generation.text.contains("He leaves on foot."));
        let prompts = generator.prompts.lock().unwrap();
        assert!(
            prompts[5].1.user.contains("He leaves in a cart."),
            "2nd review"
        );
    }

    #[tokio::test]
    async fn no_review_round_writes_the_draft_as_is() {
        let generator =
            ScriptedGenerator::new(&[plan_json("30 00 001"), "Part 1.".into(), "Part 2.".into()]);
        let writer = WriterOptions {
            reviews: 0,
            ..WriterOptions::default()
        };
        let (story, recorder) = run(&generator, writer).await;

        assert_eq!(recorder.steps.last().unwrap(), "draft 2/2");
        assert_eq!(story.written.calls, 3);
    }

    #[tokio::test]
    async fn invented_citations_are_revised_then_removed() {
        let options = StoryOptions {
            citations: true,
            ..StoryOptions::default()
        };
        let generator = ScriptedGenerator::new(&[
            plan_json("30 00 001"),
            "Redbeard [ref. 30 00 001] opens the old map [ref. 30 99 999].".into(),
            "He finds the chest [ref. 30 00 002, 30 88 888].".into(),
            json!({"issues": []}).to_string(),
            // The revision fixes the map, not the chest.
            "Redbeard [ref. 30 00 001] looks around.\n\n\
             He finds the chest [ref. 30 00 002, 30 88 888]."
                .into(),
        ]);
        let (story, recorder) =
            run_options(&generator, WriterOptions::default(), &["5135"], &options).await;

        // The reviewer found nothing: the issues come from the code.
        assert_eq!(story.written.issues.len(), 2);
        assert_eq!(
            story.written.issues[0].excerpt,
            "opens the old map [ref. 30 99 999]"
        );
        let prompts = generator.prompts.lock().unwrap();
        let revise = &prompts[4].1.user;
        assert!(revise.contains(
            "- \"opens the old map [ref. 30 99 999]\": cites ref. 30 99 999, not in the box"
        ));
        assert!(revise.contains(
            "- \"He finds the chest [ref. 30 00 002, 30 88 888]\": cites ref. 30 88 888"
        ));
        assert_eq!(
            recorder.steps.last().unwrap(),
            "citations-dropped 30 88 888"
        );
        let text = &story.written.generation.text;
        assert!(text.contains("He finds the chest [ref. 30 00 002]."));
        assert!(!text.contains("30 88 888"));
    }
}
