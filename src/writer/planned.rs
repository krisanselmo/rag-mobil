//! Planned writing: Plan → Draft → Edit, adapted from Re3 (Yang et al.,
//! EMNLP 2022) and DOC (Yang et al., ACL 2023), both MIT-licensed:
//! <https://github.com/yangkevin2/emnlp22-re3-story-generation>,
//! <https://github.com/yangkevin2/doc-story-generation>.
//!
//! Differences with the papers: the plan is grounded in the inventory and
//! checked by code (instead of learned rerankers), the drafting is one call
//! per plot point (instead of many short passages), and a review pass (Re3's
//! Edit module) lists fidelity issues that a single rewrite then fixes.

use serde::de::DeserializeOwned;

use crate::augment::{StoryOptions, build_prompt};
use crate::error::Result;
use crate::generator::{Generation, StoryGenerator};
use crate::model::Collection;
use crate::writer::plan::{
    Review, StoryPlan, in_your_box, plan_schema, review_schema, sanitize, validate,
};
use crate::writer::{Observer, Step, Written, prompts};

/// Aggregated stats over the calls of one story.
#[derive(Default)]
struct Tally {
    calls: u32,
    output_tokens: Option<u64>,
    truncated: bool,
    model: String,
}

impl Tally {
    fn add(&mut self, generation: &Generation) {
        self.calls += 1;
        self.output_tokens = match (self.output_tokens, generation.output_tokens) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.truncated |= generation.truncated;
        self.model.clone_from(&generation.model);
    }
}

/// Parses a JSON object, tolerating code fences or prose around it
/// (providers without structured outputs).
fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    let json = match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if start < end => &text[start..=end],
        _ => text,
    };
    Ok(serde_json::from_str(json)?)
}

async fn make_plan<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    observer: &mut O,
    tally: &mut Tally,
) -> Result<std::result::Result<StoryPlan, String>> {
    observer.step(Step::Planning);
    let base = prompts::plan(boxes, options);
    let schema = plan_schema();

    let first = generator
        .generate_json(&base, "story_plan", &schema)
        .await?;
    tally.add(&first);
    let errors = match parse_json::<StoryPlan>(&first.text) {
        Ok(plan) => {
            let errors = validate(&plan, boxes);
            if errors.is_empty() {
                return Ok(Ok(plan));
            }
            errors
        }
        Err(err) => vec![format!("ta réponse n'est pas un JSON valide ({err})")],
    };

    // One retry with the errors spelled out (Re3 also retries its planner).
    observer.step(Step::PlanRejected(&errors));
    let retry = prompts::plan_fix(&base, &first.text, &errors);
    let second = generator
        .generate_json(&retry, "story_plan", &schema)
        .await?;
    tally.add(&second);
    let plan: StoryPlan = match parse_json(&second.text) {
        Ok(plan) => plan,
        Err(err) => return Ok(Err(format!("plan corrigé illisible ({err})"))),
    };
    let errors = validate(&plan, boxes);
    if errors.is_empty() {
        return Ok(Ok(plan));
    }
    observer.step(Step::PlanRepaired(&errors));
    let plan = sanitize(plan, boxes);
    if plan.outline.is_empty() {
        return Ok(Err(format!(
            "plus aucune étape valide ({})",
            errors.join(" ; ")
        )));
    }
    Ok(Ok(plan))
}

/// The plan couldn't be fixed (small local models often break the JSON):
/// rather than failing, write the story in one call, like `--method direct`.
async fn fall_back_to_direct<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    observer: &mut O,
    mut tally: Tally,
) -> Result<Written> {
    let prompt = build_prompt(boxes, options);
    let generation = generator.generate(&prompt, |t| observer.text(t)).await?;
    tally.add(&generation);
    Ok(Written {
        generation: Generation {
            text: generation.text,
            model: tally.model,
            truncated: tally.truncated,
            output_tokens: tally.output_tokens,
        },
        plan: None,
        issues: Vec::new(),
        calls: tally.calls,
    })
}

pub(crate) async fn write<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    review: bool,
    observer: &mut O,
) -> Result<Written> {
    let mut tally = Tally::default();

    // 1. Plan
    let plan = match make_plan(generator, boxes, options, observer, &mut tally).await? {
        Ok(plan) => plan,
        Err(reason) => {
            observer.step(Step::PlanAbandoned(&reason));
            return fall_back_to_direct(generator, boxes, options, observer, tally).await;
        }
    };
    observer.step(Step::PlanReady(&plan));

    // 2. Draft, one plot point at a time
    let mut story = String::new();
    for index in 0..plan.outline.len() {
        observer.step(Step::Drafting {
            part: index + 1,
            total: plan.outline.len(),
        });
        let prompt = prompts::draft(boxes, options, &plan, &story, index);
        let part = generator.generate(&prompt, |_| {}).await?;
        tally.add(&part);
        if !story.is_empty() {
            story.push_str("\n\n");
        }
        story.push_str(part.text.trim());
    }

    // 3. Edit: review, then one rewrite if needed
    let mut issues = Vec::new();
    if review {
        observer.step(Step::Reviewing);
        let prompt = prompts::review(boxes, options, &story);
        let report = generator
            .generate_json(&prompt, "story_review", &review_schema())
            .await?;
        tally.add(&report);
        // The story is written: an unreadable review must not lose it.
        issues = match parse_json::<Review>(&report.text) {
            Ok(review) => review.issues,
            Err(_) => {
                observer.step(Step::ReviewUnreadable);
                Vec::new()
            }
        };
        if !issues.is_empty() {
            observer.step(Step::Revising(&issues));
            let prompt = prompts::revise(boxes, options, &story, &issues);
            let revised = generator.generate(&prompt, |_| {}).await?;
            tally.add(&revised);
            story = revised.text.trim().to_owned();
        }
    }

    let text = format!(
        "# {}\n\n{story}\n\n{}",
        plan.title.trim(),
        in_your_box(&plan, boxes, options)
    );
    observer.text(&text);

    Ok(Written {
        generation: Generation {
            text,
            model: tally.model,
            truncated: tally.truncated,
            output_tokens: tally.output_tokens,
        },
        plan: Some(plan),
        issues,
        calls: tally.calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_wrapped_in_fences() {
        let review: Review = parse_json("Voici :\n```json\n{\"issues\": []}\n```").unwrap();
        assert!(review.issues.is_empty());
        assert!(parse_json::<Review>("pas de json").is_err());
    }
}
