//! Rédaction: how the story text is produced from the augmented context.
//!
//! - [`WritingMethod::Direct`]: one LLM call writes the whole story.
//! - [`WritingMethod::Planned`]: Plan → Draft → Edit, adapted from the
//!   open-source Re3 and DOC story generators (see `writer/planned.rs`).

pub mod plan;
mod planned;
mod prompts;

use crate::augment::{Prompt, StoryOptions, build_prompt};
use crate::error::Result;
use crate::generator::{Generation, StoryGenerator};
use crate::model::Collection;

pub use plan::{Issue, PlannedCharacter, PlotPoint, StoryPlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMethod {
    /// One call; the story streams as it is written.
    Direct,
    /// Plan, then draft each plot point, then review (several calls).
    #[default]
    Planned,
}

#[derive(Debug, Clone, Copy)]
pub struct WriterOptions {
    pub method: WritingMethod,
    /// Planned method only: review the draft and fix fidelity issues.
    pub review: bool,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            method: WritingMethod::Planned,
            review: true,
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
    /// Issues found by the review (fixed in the final text).
    pub issues: Vec<Issue>,
    pub calls: u32,
}

/// First prompt sent by `method`, for `--dry-run`.
pub fn first_prompt(boxes: &Collection, options: &StoryOptions, method: WritingMethod) -> Prompt {
    match method {
        WritingMethod::Direct => build_prompt(boxes, options),
        WritingMethod::Planned => prompts::plan(boxes, options),
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
                issues: Vec::new(),
                calls: 1,
            })
        }
        WritingMethod::Planned => {
            planned::write(generator, boxes, options, writer.review, observer).await
        }
    }
}
