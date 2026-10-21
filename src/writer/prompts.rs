//! Prompts of the planned method (Plan → Draft → Edit).

use std::fmt::Write;

use crate::augment::{Prompt, StoryOptions, build_context, reader_request, storyteller_rules};
use crate::model::Collection;
use crate::writer::plan::{Issue, StoryPlan, cited_label, find_part, render_plan, step_objects};

/// Number of plot points for a story of `words` words.
pub(crate) fn outline_length(words: u32) -> usize {
    match words {
        0..=400 => 3,
        401..=900 => 4,
        _ => 5,
    }
}

/// The inventory, then the previous episodes of a saga, if any.
fn plan_context(boxes: &Collection, options: &StoryOptions) -> String {
    let context = build_context(boxes, &options.context);
    let saga = options.saga.context(boxes);
    if saga.is_empty() {
        context
    } else {
        format!("{context}\n\n{saga}")
    }
}

/// The reader's request, then the saga's, if any.
fn plan_request(boxes: &Collection, options: &StoryOptions) -> String {
    let request = reader_request(boxes, options);
    let saga = options.saga.request();
    if saga.is_empty() {
        request
    } else {
        format!("{request}\n{saga}")
    }
}

pub(crate) fn plan(boxes: &Collection, options: &StoryOptions) -> Prompt {
    let context = plan_context(boxes, options);
    let request = plan_request(boxes, options);
    let steps = outline_length(options.target_words);
    Prompt {
        system: format!(
            "{}\n\n\
             Before writing, you prepare the outline of the story: a premise, a setting, \
             the characters, then the main steps of the plot. Answer only with the \
             requested JSON.",
            storyteller_rules(options)
        ),
        user: format!(
            "{context}\n\n{request}\n\n\
             Prepare the outline of this story in {steps} steps (a beginning, \
             complications, a resolution). Each character is a different figure from \
             <characters>, given by its reference; the same figure can play several \
             characters only if the box contains several of it. For each step, list the \
             characters present (by name) and the references of the objects, animals or \
             vehicles from <pieces> they use. Write the texts of the outline in \
             {language}.",
            language = options.language,
        ),
        cached: 0,
    }
}

/// Re-asks for the plan, with the problems found in the previous one.
pub(crate) fn plan_fix(base: &Prompt, previous: &str, errors: &[String]) -> Prompt {
    let mut user = format!(
        "{}\n\nHere is your previous outline:\n{previous}\n\n\
         It is not faithful to the box:\n",
        base.user
    );
    for error in errors {
        let _ = writeln!(user, "- {error}");
    }
    user.push_str(
        "\nSend back the complete, corrected outline, using only references present in \
         <set>.",
    );
    Prompt {
        system: base.system.clone(),
        user,
        cached: 0,
    }
}

/// Plan of an interactive story: a graph of paragraphs linked by choices.
pub(crate) fn game_plan(boxes: &Collection, options: &StoryOptions) -> Prompt {
    let context = plan_context(boxes, options);
    let request = plan_request(boxes, options);
    Prompt {
        system: format!(
            "{}\n\n\
             Before writing, you prepare the outline of an interactive story, where the \
             reader chooses what happens next (like a \"choose your own adventure\" \
             book): a premise, a setting, the characters, then paragraphs linked by \
             choices. Answer only with the requested JSON.",
            storyteller_rules(options)
        ),
        user: format!(
            "{context}\n\n{request}\n\n\
             Prepare the outline of this interactive story in 6 to 9 paragraphs. The \
             first paragraph is the beginning. Each paragraph that is not an ending ends \
             with 2 choices (3 at most) offered to the reader, each leading to another \
             paragraph, given by its identifier; two choices may lead to the same \
             paragraph. A paragraph without choices is an ending: plan 2 to 4 different \
             endings, all happy or reassuring. No loops: a choice always leads further \
             into the story, and every paragraph is reached by at least one choice. Each \
             character is a different figure from <characters>, given by its reference; \
             the same figure can play several characters only if the box contains \
             several of it. For each paragraph, list the characters present (by name) \
             and the references of the objects, animals or vehicles from <pieces> they \
             use; a choice is a short decision, addressed to the reader (\"Pip goes \
             into the cave\"). Write the texts of the outline in {language}.",
            language = options.language,
        ),
        cached: 0,
    }
}

/// Characters of a step with their figure, and its objects.
fn step_cast(
    boxes: &Collection,
    options: &StoryOptions,
    plan: &StoryPlan,
    index: usize,
) -> (Vec<String>, Vec<String>) {
    let step = &plan.outline[index];
    let characters = step
        .characters
        .iter()
        .map(|name| {
            plan.characters
                .iter()
                .find(|c| c.name.trim().eq_ignore_ascii_case(name.trim()))
                .and_then(|c| find_part(boxes, &c.figure))
                .map_or_else(
                    || name.clone(),
                    |p| format!("{name} ({})", cited_label(p, options.citations)),
                )
        })
        .collect();
    (characters, step_objects(step, boxes, options.citations))
}

/// One paragraph of an interactive story, knowing the paragraphs that lead
/// to it and the choices it ends with (none for an ending).
pub(crate) fn paragraph(
    boxes: &Collection,
    options: &StoryOptions,
    plan: &StoryPlan,
    path: &str,
    index: usize,
    choices: &[&str],
    words: usize,
) -> Prompt {
    let context = build_context(boxes, &options.context);
    let mut user = format!(
        "{context}\n\n<plan>\n{}</plan>\n\n",
        render_plan(plan, boxes)
    );
    // The same for every paragraph.
    let cached = user.len();
    if !path.is_empty() {
        let _ = write!(user, "<story_so_far>\n{path}\n</story_so_far>\n\n");
    }
    let (characters, objects) = step_cast(boxes, options, plan, index);
    let position = if index == 0 {
        "This is the beginning: set the scene and introduce the characters.".to_owned()
    } else if choices.is_empty() {
        "This is one of the endings of the story: wrap it up in a happy or reassuring way."
            .to_owned()
    } else {
        String::new()
    };
    let ending = if choices.is_empty() {
        String::new()
    } else {
        format!(
            "The paragraph stops at the moment the reader must choose between: {}. Don't \
             write these choices, they will be added after your text.",
            choices.join("; ")
        )
    };
    let _ = write!(
        user,
        "Write paragraph {n} in {language}, for a {age}-year-old child, about {words} \
         words long.\n\
         What happens: {event}\n\
         Characters present: {characters}\n\
         Objects from the box to use: {objects}\n\
         {position}{sep}{ending}",
        n = index + 1,
        language = options.language,
        age = options.reader_age,
        event = plan.outline[index].event,
        characters = or_none(&characters),
        objects = or_none(&objects),
        sep = if position.is_empty() || ending.is_empty() {
            ""
        } else {
            "\n"
        },
    );
    Prompt {
        system: format!("{}\n\n{DRAFT_STYLE}", storyteller_rules(options)),
        user,
        cached,
    }
}

const DRAFT_STYLE: &str = "\
You write the story part by part, following an approved outline. Style: suited \
to the requested age, warm, no graphic violence, with dialogue. Write only the \
requested part, in prose: no title, no subheading, no commentary, no list of \
pieces.";

/// Draft of plot point `index` (DOC-style: the detailed plan steers each
/// passage; the story so far keeps continuity).
pub(crate) fn draft(
    boxes: &Collection,
    options: &StoryOptions,
    plan: &StoryPlan,
    story_so_far: &str,
    index: usize,
) -> Prompt {
    let context = build_context(boxes, &options.context);
    let total = plan.outline.len();
    let step = &plan.outline[index];
    let words = (options.target_words as usize / total.max(1)).max(60);

    let mut user = format!(
        "{context}\n\n<plan>\n{}</plan>\n\n",
        render_plan(plan, boxes)
    );
    // The same for every part.
    let cached = user.len();
    if !story_so_far.is_empty() {
        let _ = write!(user, "<story_so_far>\n{story_so_far}\n</story_so_far>\n\n");
    }

    let (characters, objects) = step_cast(boxes, options, plan, index);
    let position = if index == 0 {
        "This is the beginning: set the scene and introduce the characters."
    } else if index + 1 == total {
        "This is the last part: bring the story to a close."
    } else {
        "Don't bring the story to a close yet."
    };

    let _ = write!(
        user,
        "Write part {n}/{total} in {language}, for a {age}-year-old child, about \
         {words} words long.\n\
         What happens: {event}\n\
         Characters present: {characters}\n\
         Objects from the box to use: {objects}\n\
         {position}",
        n = index + 1,
        language = options.language,
        age = options.reader_age,
        event = step.event,
        characters = or_none(&characters),
        objects = or_none(&objects),
    );

    Prompt {
        system: format!("{}\n\n{DRAFT_STYLE}", storyteller_rules(options)),
        user,
        cached,
    }
}

fn or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none in particular".to_owned()
    } else {
        items.join("; ")
    }
}

/// Re3's "Edit" module, adapted: a reviewer looks for anything the story
/// uses that the box doesn't contain.
pub(crate) fn review(boxes: &Collection, options: &StoryOptions, story: &str) -> Prompt {
    let context = build_context(boxes, &options.context);
    Prompt {
        system: "\
You are a reviewer for a children's publisher. You check that a story can be \
played out with the exact contents of a Playmobil box, whose inventory is given \
between <set> tags. Answer only with the requested JSON."
            .to_owned(),
        user: format!(
            "{context}\n\n<story>\n{story}\n</story>\n\n\
             List the problems of faithfulness to the box:\n\
             - a character who acts without matching a figure from <characters>;\n\
             - an object, animal or vehicle used that is not in <pieces>, or in greater \
             numbers than in the box;\n\
             - an appearance (clothes, colors, hair) that contradicts the figure.\n\
             Don't list what is only mentioned, imagined by the characters or built with \
             the scenery pieces. List only real problems: an empty list is a good \
             answer. Write in {}.",
            options.language
        ),
        cached: 0,
    }
}

/// Line between two parts of the story, for a reviser that must keep them
/// apart (scene setups).
pub(crate) const SCENE_BREAK: &str = "* * *";

/// Rewrites the story, fixing only the reviewed issues. With `scenes`, the
/// story is made of that many parts separated by [`SCENE_BREAK`] lines, to
/// keep.
pub(crate) fn revise(
    boxes: &Collection,
    options: &StoryOptions,
    story: &str,
    issues: &[Issue],
    scenes: Option<usize>,
) -> Prompt {
    let context = build_context(boxes, &options.context);
    let mut user = format!(
        "{context}\n\n<story>\n{story}\n</story>\n\n\
         A reviewer found these mismatches with the contents of the box:\n"
    );
    for issue in issues {
        let _ = writeln!(
            user,
            "- \"{}\": {} → {}",
            issue.excerpt, issue.problem, issue.fix
        );
    }
    let _ = write!(
        user,
        "\nRewrite the complete story in {}, fixing only these mismatches; keep \
         everything else exactly as it is. No title, no commentary, no list of pieces.",
        options.language
    );
    if let Some(count) = scenes {
        let _ = write!(
            user,
            " The story is in {count} parts separated by \"{SCENE_BREAK}\" lines: keep \
             these separators, in the same places."
        );
    }
    Prompt {
        system: format!(
            "{}\n\nYou are correcting an existing story.",
            storyteller_rules(options)
        ),
        user,
        cached: 0,
    }
}
