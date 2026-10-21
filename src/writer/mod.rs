//! Writing: how the story text is produced from the augmented context.
//!
//! - [`WritingMethod::Direct`]: one LLM call writes the whole story.
//! - [`WritingMethod::Planned`]: Plan → Draft → Edit, adapted from the
//!   open-source Re3 and DOC story generators (see `writer/planned.rs`).

pub mod citations;
pub mod interactive;
pub mod plan;
mod planned;
mod prompts;

use crate::augment::{Prompt, StoryOptions, build_prompt};
use crate::error::Result;
use crate::generator::{Generation, StoryGenerator};
use crate::model::Collection;

pub use interactive::{GamePlan, Paragraph};
pub use plan::{Issue, PlannedCharacter, PlotPoint, SceneSetup, StoryPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMethod {
    /// One call; the story streams as it is written.
    Direct,
    /// Plan, then draft each plot point, then review (several calls).
    #[default]
    Planned,
    /// Like `Planned`, but the plan is a graph of paragraphs ending with
    /// choices for the reader ("choose your own adventure").
    Interactive,
}

#[derive(Debug, Clone, Copy)]
pub struct WriterOptions {
    pub method: WritingMethod,
    /// Planned and interactive methods: rounds of review, each followed by a
    /// rewrite fixing the fidelity issues found (0 = no review). The rounds
    /// stop at the first review that finds nothing.
    pub reviews: u8,
    /// Planned method only: before each part of the story, a "Mise en place"
    /// listing the characters and objects to put on the table, from the plan.
    pub scene_setup: bool,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            method: WritingMethod::Planned,
            reviews: 1,
            scene_setup: false,
        }
    }
}

/// Progress of the planned method.
#[derive(Debug)]
pub enum Step<'a> {
    Planning,
    /// The plan contradicted the inventory and is asked again.
    PlanRejected(&'a [String]),
    /// Still invalid after the retry: invalid references were dropped.
    PlanRepaired(&'a [String]),
    /// No usable plan even after the retry: the story is written in one
    /// call instead (direct method), with this reason.
    PlanAbandoned(&'a str),
    PlanReady(&'a StoryPlan),
    Drafting {
        part: usize,
        total: usize,
    },
    Reviewing,
    /// The review answer wasn't valid JSON: the draft is kept as is.
    ReviewUnreadable,
    Revising(&'a [Issue]),
    /// The revision didn't keep the parts of the story apart: the scene
    /// setups are left out.
    SceneSetupDropped,
    /// `--citations`: references still not in the boxes after the review,
    /// removed from the story.
    CitationsDropped(&'a [String]),
}

/// Receives the story text and the progress of the writing.
pub trait Observer: Send {
    /// Story text: streamed chunks (direct) or the final story (planned).
    fn text(&mut self, _chunk: &str) {}
    fn step(&mut self, _step: Step<'_>) {}
}

/// Ignores everything.
impl Observer for () {}

/// The written story.
#[derive(Debug, Clone)]
pub struct Written {
    /// Final text, with stats aggregated over every call.
    pub generation: Generation,
    pub plan: Option<StoryPlan>,
    /// Planned method: the story itself (no title, no "In your box"), one
    /// entry per plot point, or a single one when the revision merged them.
    /// Empty with the direct method.
    pub scenes: Vec<String>,
    /// Each scene is preceded by its setup (one scene per plot point).
    pub staged: bool,
    /// Interactive method: one per scene, in the same order (paragraph `n`
    /// is `scenes[n - 1]`). Empty otherwise.
    pub paragraphs: Vec<Paragraph>,
    /// Issues found by the review (fixed in the final text).
    pub issues: Vec<Issue>,
    pub calls: u32,
}

/// First prompt sent by `method`, for `--dry-run`.
pub fn first_prompt(boxes: &Collection, options: &StoryOptions, method: WritingMethod) -> Prompt {
    match method {
        WritingMethod::Direct => build_prompt(boxes, options),
        WritingMethod::Planned => prompts::plan(boxes, options),
        WritingMethod::Interactive => prompts::game_plan(boxes, options),
    }
}

pub async fn write<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    writer: WriterOptions,
    observer: &mut O,
) -> Result<Written> {
    match writer.method {
        WritingMethod::Direct => {
            let prompt = build_prompt(boxes, options);
            let generation = generator.generate(&prompt, |t| observer.text(t)).await?;
            Ok(Written {
                generation,
                plan: None,
                scenes: Vec::new(),
                staged: false,
                paragraphs: Vec::new(),
                issues: Vec::new(),
                calls: 1,
            })
        }
        WritingMethod::Planned => planned::write(generator, boxes, options, writer, observer).await,
        WritingMethod::Interactive => {
            interactive::write(generator, boxes, options, writer, observer).await
        }
    }
}
