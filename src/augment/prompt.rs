//! Prompt templates: a stable system prompt (the storyteller's rules) and a
//! user message carrying the retrieved inventory plus the request.

use std::fmt::Write;

use crate::augment::context::{ContextOptions, build_context};
use crate::model::{Collection, reference_key};
use crate::saga::Saga;

/// What the user wants from the story.
#[derive(Debug, Clone)]
pub struct StoryOptions {
    /// Output language, e.g. `français`, `English`.
    pub language: String,
    /// Age of the target reader.
    pub reader_age: u8,
    /// Approximate length in words.
    pub target_words: u32,
    /// Free-form wish ("une histoire de pirates fantômes"...).
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
            language: "français".into(),
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
#[derive(Debug, Clone)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

/// How a story cites a part with `citations` on: the inventory's reference
/// after the name, several in one group for a multi-piece object. A reader
/// strips `\s*\[réf\.[^\]]*\]` or turns each reference into a link.
pub const CITATION_FORMAT: &str = "[réf. 30 06 368], ou [réf. 30 06 198, 30 06 199]";

/// Storyteller role and inventory-fidelity rules, shared by every writing
/// method (see [`crate::writer`]); see [`storyteller_rules`] for the ending.
const STORYTELLER_ROLE: &str = "\
Tu es un conteur pour enfants spécialisé dans l'univers Playmobil. Tu écris des \
histoires qu'un enfant pourra rejouer avec le contenu exact d'une boîte Playmobil \
posée devant lui.

L'inventaire officiel de la boîte t'est fourni entre balises <set>. Il provient de \
PlaymoDB, une base de données de fans, et c'est ta seule source de vérité sur ce \
que contient la boîte. La fidélité à cet inventaire est ce qui rend l'histoire \
utile : l'enfant doit pouvoir trouver chaque personnage et chaque objet cités.

Règles de fidélité :
- Les personnages principaux sont les figurines de <personnages>. Tu peux leur \
donner un prénom et un caractère, mais leur apparence (vêtements, couleurs, \
coiffure, accessoires) doit correspondre à leur description. N'invente pas de \
personnage supplémentaire qui jouerait un rôle actif ; un personnage hors boîte ne \
peut être qu'évoqué (\"le roi voisin\", \"un dragon dont on parle\").
- Les objets, animaux, véhicules et éléments de décor que les personnages \
utilisent doivent figurer dans <pieces>. Respecte les quantités : s'il y a 2 \
chevaux, il n'y en a pas 3. Une quantité \"?\" signifie qu'elle est inconnue.
- Les pièces purement techniques (connecteurs, clips, charnières, morceaux de \
mur) ne sont pas des objets de l'histoire, mais elles indiquent ce qu'on peut \
construire (un château, une tour, un pont-levis...) : sers-t'en pour planter le \
décor.
- Les noms des pièces sont en anglais dans l'inventaire ; traduis-les \
naturellement dans la langue de l'histoire. Chaque pièce est suivie de sa \
référence [réf. …] : ";

/// The storyteller's rules, with or without citing references in the story.
pub fn storyteller_rules(options: &StoryOptions) -> String {
    if options.citations {
        format!(
            "{STORYTELLER_ROLE}chaque fois que l'histoire nomme une figurine ou une pièce \
             de la boîte, recopie sa référence juste après le nom, entre crochets : \
             {CITATION_FORMAT} pour un objet fait de plusieurs pièces. Ces références sont \
             retirées ou changées en liens à l'affichage : la phrase doit rester correcte \
             sans elles. Ne cite que des références présentes dans <set>."
        )
    } else {
        format!("{STORYTELLER_ROLE}elle sert au plan, ne la cite jamais dans l'histoire.")
    }
}

/// Style for the single-call ("direct") method.
const DIRECT_STYLE: &str = "\
Style : une histoire complète (début, péripétie, résolution), adaptée à l'âge \
demandé, chaleureuse, sans violence graphique, avec des dialogues. Commence par un \
titre sur la première ligne, puis l'histoire. Termine par une courte section \
\"Dans ta boîte\" listant les figurines et les objets de l'inventaire utilisés \
dans l'histoire, pour que l'enfant puisse les retrouver.";

/// "A story in {language} for a {age}-year-old, ~{words} words, set N" +
/// the child's wish: the reader-facing part of every request.
pub fn reader_request(collection: &Collection, options: &StoryOptions) -> String {
    let wish = options
        .extra_instructions
        .as_deref()
        .map(|w| format!("\nSouhait particulier de l'enfant : {w}"))
        .unwrap_or_default();
    let stars = favorite_parts(collection, &options.stars);
    let boxes = match collection.sets.as_slice() {
        [set] => format!(
            "le set Playmobil {} « {} » décrit ci-dessus",
            set.set_number, set.name
        ),
        _ => format!(
            "les {} sets Playmobil décrits ci-dessus, réunis : {}. Les personnages \
             et les pièces de toutes les boîtes se rencontrent dans la même histoire ; \
             une référence présente dans plusieurs boîtes compte les exemplaires de \
             chacune",
            collection.sets.len(),
            collection.label()
        ),
    };
    format!(
        "L'histoire est en {language}, pour un enfant de {age} ans, d'environ {words} \
         mots, et se joue avec {boxes}.{wish}{stars}",
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
            Some(color) => format!("{} ({color}) [réf. {}]", part.name, part.number),
            None => format!("{} [réf. {}]", part.name, part.number),
        };
        let role = if part.is_figure() {
            "fais-en le personnage principal, présent du début à la fin de l'histoire"
        } else {
            "donne-lui un rôle central dans l'intrigue : qu'il serve plusieurs fois, \
             et pas seulement en passant"
        };
        let _ = write!(lines, "\nPièce préférée de l'enfant : {label} ; {role}.");
    }
    lines
}

/// Single-call prompt: the whole story in one generation.
pub fn build_prompt(collection: &Collection, options: &StoryOptions) -> Prompt {
    let context = build_context(collection, &options.context);
    let request = reader_request(collection, options);
    Prompt {
        system: format!("{}\n\n{DIRECT_STYLE}", storyteller_rules(options)),
        user: format!("{context}\n\nÉcris cette histoire. {request}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_switch_the_reference_rule() {
        let hidden = storyteller_rules(&StoryOptions::default());
        assert!(hidden.ends_with("ne la cite jamais dans l'histoire."));
        let cited = storyteller_rules(&StoryOptions {
            citations: true,
            ..StoryOptions::default()
        });
        assert!(cited.contains(CITATION_FORMAT));
        assert!(!cited.contains("ne la cite jamais"));
    }
}
