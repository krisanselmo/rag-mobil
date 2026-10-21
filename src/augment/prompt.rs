//! Prompt templates: a stable system prompt (the storyteller's rules) and a
//! user message carrying the retrieved inventory plus the request.

use std::fmt::Write;

use crate::augment::context::{ContextOptions, build_context};
use crate::model::{Collection, reference_key};
use crate::saga::Saga;

/// What the user wants from the story.
#[derive(Debug, Clone)]
pub struct StoryOptions {
    /// Output language, e.g. `English`, `français`.
    pub language: String,
    /// Age of the target reader.
    pub reader_age: u8,
    /// Approximate length in words.
    pub target_words: u32,
    /// Free-form wish ("a story about ghost pirates"...).
    pub extra_instructions: Option<String>,
    /// Cite each part the story names, in the format of [`CITATION_FORMAT`].
    pub citations: bool,
    pub context: ContextOptions,
    /// Previous episodes, when the story continues a saga.
    pub saga: Saga,
    /// The child's favorite parts, by reference (`--star`): at the heart of
    /// the story. Each must be in the boxes.
    pub stars: Vec<String>,
}

impl Default for StoryOptions {
    fn default() -> Self {
        Self {
            language: "English".into(),
            reader_age: 7,
            target_words: 600,
            extra_instructions: None,
            citations: false,
            context: ContextOptions::default(),
            saga: Saga::default(),
            stars: Vec::new(),
        }
    }
}

/// The fully augmented prompt, ready for any LLM backend.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub system: String,
    pub user: String,
    /// Length in bytes of the start of `user` that the next calls for the
    /// same story repeat, with the same `system` (the inventory and the
    /// plan, for each part drafted): a provider with prompt caching (Claude)
    /// stores this prefix once, then reads it back at a fraction of the
    /// price. 0 = nothing worth caching.
    pub cached: usize,
}

impl Prompt {
    /// `user` split after its [`cached`](Self::cached) prefix; the prefix
    /// is empty when nothing is worth caching.
    pub fn split_user(&self) -> (&str, &str) {
        if self.cached > 0 && self.user.is_char_boundary(self.cached) {
            self.user.split_at(self.cached)
        } else {
            ("", &self.user)
        }
    }
}

/// How a story cites a part with `citations` on: the inventory's reference
/// after the name, several in one group for a multi-piece object. A reader
/// strips `\s*\[ref\.[^\]]*\]` or turns each reference into a link.
pub const CITATION_FORMAT: &str = "[ref. 30 06 368], or [ref. 30 06 198, 30 06 199]";

/// Storyteller role and inventory-fidelity rules, shared by every writing
/// method (see [`crate::writer`]); see [`storyteller_rules`] for the ending.
const STORYTELLER_ROLE: &str = "\
You are a children's storyteller specializing in the Playmobil world. You write \
stories that a child can play out with the exact contents of a Playmobil box set \
out in front of them.

The official inventory of the box is given to you between <set> tags. It comes \
from PlaymoDB, a fan database, and it is your only source of truth about what the \
box contains. Faithfulness to this inventory is what makes the story useful: the \
child must be able to find every character and every object mentioned.

Faithfulness rules:
- The main characters are the figures from <characters>. You can give them a \
name and a personality, but their appearance (clothes, colors, hair, \
accessories) must match their description. Don't invent any extra character who \
would play an active role; a character outside the box can only be mentioned \
(\"the neighboring king\", \"a dragon people talk about\").
- The objects, animals, vehicles and pieces of scenery the characters use must \
appear in <pieces>. Respect the quantities: if there are 2 horses, there are not \
3. A quantity \"?\" means it is unknown.
- Purely technical pieces (connectors, clips, hinges, wall sections) are not \
objects of the story, but they show what can be built (a castle, a tower, a \
drawbridge...): use them to set the scene.
- Piece names are in English in the inventory; translate them naturally into the \
language of the story. Each piece is followed by its reference [ref. …]: ";

/// The storyteller's rules, with or without citing references in the story.
pub fn storyteller_rules(options: &StoryOptions) -> String {
    if options.citations {
        format!(
            "{STORYTELLER_ROLE}each time the story names a figure or a piece from the \
             box, copy its reference right after the name, in square brackets: \
             {CITATION_FORMAT} for an object made of several pieces. These references are \
             removed or turned into links when displayed: the sentence must still read \
             correctly without them. Only cite references present in <set>."
        )
    } else {
        format!("{STORYTELLER_ROLE}it is for the outline, never cite it in the story.")
    }
}

/// Style for the single-call ("direct") method.
const DIRECT_STYLE: &str = "\
Style: a complete story (beginning, complication, resolution), suited to the \
requested age, warm, no graphic violence, with dialogue. Start with a title on \
the first line, then the story. End with a short \"In your box\" section listing \
the figures and objects from the inventory used in the story, so the child can \
find them.";

/// "A story in {language} for a {age}-year-old, ~{words} words, set N" +
/// the child's wish: the reader-facing part of every request.
pub fn reader_request(collection: &Collection, options: &StoryOptions) -> String {
    let wish = options
        .extra_instructions
        .as_deref()
        .map(|w| format!("\nThe child's special wish: {w}"))
        .unwrap_or_default();
    let stars = favorite_parts(collection, &options.stars);
    let boxes = match collection.sets.as_slice() {
        [set] => format!(
            "the Playmobil set {} \"{}\" described above",
            set.set_number, set.name
        ),
        _ => format!(
            "the {} Playmobil sets described above, together: {}. The characters \
             and pieces of all the boxes meet in the same story; a reference present \
             in several boxes counts the copies of each",
            collection.sets.len(),
            collection.label()
        ),
    };
    format!(
        "The story is in {language}, for a {age}-year-old child, about {words} words \
         long, and is played with {boxes}.{wish}{stars}",
        language = options.language,
        age = options.reader_age,
        words = options.target_words,
    )
}

/// The child's favorite parts (`--star`), one line each; references
/// missing from the boxes are skipped.
fn favorite_parts(collection: &Collection, stars: &[String]) -> String {
    let mut lines = String::new();
    for star in stars {
        let wanted = reference_key(star);
        let Some((_, part)) = collection
            .parts()
            .find(|(_, p)| reference_key(&p.number) == wanted)
        else {
            continue;
        };
        let label = match &part.color {
            Some(color) => format!("{} ({color}) [ref. {}]", part.name, part.number),
            None => format!("{} [ref. {}]", part.name, part.number),
        };
        let role = if part.is_figure() {
            "make it the main character, present from the beginning to the end of the story"
        } else {
            "give it a central role in the plot: use it several times, not just in \
             passing"
        };
        let _ = write!(lines, "\nThe child's favorite piece: {label}; {role}.");
    }
    lines
}

/// Single-call prompt: the whole story in one generation.
pub fn build_prompt(collection: &Collection, options: &StoryOptions) -> Prompt {
    let context = build_context(collection, &options.context);
    let request = reader_request(collection, options);
    Prompt {
        system: format!("{}\n\n{DIRECT_STYLE}", storyteller_rules(options)),
        user: format!("{context}\n\nWrite this story. {request}\n"),
        cached: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_switch_the_reference_rule() {
        let hidden = storyteller_rules(&StoryOptions::default());
        assert!(hidden.ends_with("never cite it in the story."));
        let cited = storyteller_rules(&StoryOptions {
            citations: true,
            ..StoryOptions::default()
        });
        assert!(cited.contains(CITATION_FORMAT));
        assert!(!cited.contains("never cite it"));
    }

    #[test]
    fn user_splits_after_its_cached_prefix() {
        let mut prompt = Prompt {
            system: "rules".into(),
            user: "<set>…</set>\n\nWrite part 2.".into(),
            cached: "<set>…</set>\n\n".len(),
        };
        assert_eq!(prompt.split_user(), ("<set>…</set>\n\n", "Write part 2."));
        prompt.cached = 0;
        assert_eq!(prompt.split_user(), ("", prompt.user.as_str()));
        prompt.cached = "<set>".len() + 1; // inside "…"
        assert_eq!(prompt.split_user().0, "");
    }
}
