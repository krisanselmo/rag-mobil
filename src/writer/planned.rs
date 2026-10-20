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
    Review, StoryPlan, feature_stars, in_your_box, plan_schema, review_schema, sanitize,
    scene_setup, scene_setup_markdown, validate, validate_stars,
};
use crate::writer::{Observer, Step, WriterOptions, Written, prompts};

/// Aggregated stats over the calls of one story.
#[derive(Default)]
pub(crate) struct Tally {
    pub(crate) calls: u32,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) truncated: bool,
    pub(crate) model: String,
}

impl Tally {
    pub(crate) fn add(&mut self, generation: &Generation) {
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
pub(crate) fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T> {
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
    let check = |plan: &StoryPlan| {
        let mut errors = validate(plan, boxes);
        errors.extend(options.saga.validate(plan, boxes));
        errors.extend(validate_stars(plan, boxes, &options.stars));
        errors
    };
    let errors = match parse_json::<StoryPlan>(&first.text) {
        Ok(plan) => {
            let errors = check(&plan);
            if errors.is_empty() {
                return Ok(Ok(plan));
            }
            errors
        }
        Err(err) => vec![format!("your answer is not valid JSON ({err})")],
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
        Err(err) => return Ok(Err(format!("unreadable corrected outline ({err})"))),
    };
    let errors = check(&plan);
    if errors.is_empty() {
        return Ok(Ok(plan));
    }
    observer.step(Step::PlanRepaired(&errors));
    let plan = options.saga.restore_names(sanitize(plan, boxes), boxes);
    let plan = feature_stars(plan, boxes, &options.stars);
    if plan.outline.is_empty() {
        return Ok(Err(format!("no valid step left ({})", errors.join("; "))));
    }
    Ok(Ok(plan))
}

/// The plan couldn't be fixed (small local models often break the JSON):
/// rather than failing, write the story in one call, like `--method direct`.
pub(crate) async fn fall_back_to_direct<G: StoryGenerator, O: Observer>(
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
        scenes: Vec::new(),
        staged: false,
        paragraphs: Vec::new(),
        issues: Vec::new(),
        calls: tally.calls,
    })
}

/// Splits a revised story on the scene breaks kept by the reviser; `None`
/// if it didn't keep `count` parts.
fn split_scenes(story: &str, count: usize) -> Option<Vec<String>> {
    let mut scenes = vec![String::new()];
    for line in story.lines() {
        if line.trim() == prompts::SCENE_BREAK {
            scenes.push(String::new());
        } else {
            let scene = scenes.last_mut().expect("never empty");
            scene.push_str(line);
            scene.push('\n');
        }
    }
    let scenes: Vec<String> = scenes.iter().map(|s| s.trim().to_owned()).collect();
    (scenes.len() == count && scenes.iter().all(|s| !s.is_empty())).then_some(scenes)
}

pub(crate) async fn write<G: StoryGenerator, O: Observer>(
    generator: &G,
    boxes: &Collection,
    options: &StoryOptions,
    writer: WriterOptions,
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
    let mut scenes: Vec<String> = Vec::with_capacity(plan.outline.len());
    for index in 0..plan.outline.len() {
        observer.step(Step::Drafting {
            part: index + 1,
            total: plan.outline.len(),
        });
        let story = scenes.join("\n\n");
        let prompt = prompts::draft(boxes, options, &plan, &story, index);
        let part = generator.generate(&prompt, |_| {}).await?;
        tally.add(&part);
        scenes.push(part.text.trim().to_owned());
    }
    let story = scenes.join("\n\n");

    // 3. Edit: review, then one rewrite if needed
    let mut issues = Vec::new();
    if writer.review {
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
            // With scene setups, the parts must stay apart: the reviser
            // gets them separated by scene breaks, and is asked to keep them.
            let staged = writer.scene_setup && scenes.len() > 1;
            let prompt = if staged {
                let story = scenes.join(&format!("\n\n{}\n\n", prompts::SCENE_BREAK));
                prompts::revise(boxes, options, &story, &issues, Some(scenes.len()))
            } else {
                prompts::revise(boxes, options, &story, &issues, None)
            };
            let revised = generator.generate(&prompt, |_| {}).await?;
            tally.add(&revised);
            let revised = revised.text.trim();
            scenes = match split_scenes(revised, scenes.len()) {
                Some(parts) if staged => parts,
                _ => {
                    if staged {
                        observer.step(Step::SceneSetupDropped);
                    }
                    // Without the breaks, should some be left over.
                    let text = revised
                        .lines()
                        .filter(|l| !staged || l.trim() != prompts::SCENE_BREAK)
                        .collect::<Vec<_>>()
                        .join("\n");
                    vec![text.trim().to_owned()]
                }
            };
        }
    }

    let staged = writer.scene_setup && scenes.len() == plan.outline.len();
    let story = if staged {
        scenes
            .iter()
            .zip(&plan.outline)
            .map(|(scene, step)| {
                let setup = scene_setup(step, &plan, boxes);
                if setup.is_empty() {
                    scene.clone()
                } else {
                    format!("{}\n\n{scene}", scene_setup_markdown(&setup, options))
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        scenes.join("\n\n")
    };
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
        scenes,
        staged,
        paragraphs: Vec::new(),
        issues,
        calls: tally.calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_scenes_on_breaks() {
        let story = "Un.\n\n* * *\n\nDeux,\nsuite.\n  * * *  \nTrois.";
        assert_eq!(
            split_scenes(story, 3).unwrap(),
            ["Un.", "Deux,\nsuite.", "Trois."]
        );
        assert!(split_scenes(story, 2).is_none());
        assert!(split_scenes("Un.\n* * *\n* * *\nTrois.", 3).is_none());
    }

    #[test]
    fn parses_json_wrapped_in_fences() {
        let review: Review = parse_json("Voici :\n```json\n{\"issues\": []}\n```").unwrap();
        assert!(review.issues.is_empty());
        assert!(parse_json::<Review>("no json").is_err());
    }
}
