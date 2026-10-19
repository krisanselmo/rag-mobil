//! Prompt templates: a stable system prompt (the storyteller's rules) and a
//! user message carrying the retrieved inventory plus the request.

use crate::augment::context::{ContextOptions, build_context};
use crate::model::Collection;

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
    pub context: ContextOptions,
}

impl Default for StoryOptions {
    fn default() -> Self {
        Self {
            language: "français".into(),
            reader_age: 7,
            target_words: 600,
            extra_instructions: None,
            context: ContextOptions::default(),
        }
    }
}

/// The fully augmented prompt, ready for any LLM backend.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

/// Storyteller role and inventory-fidelity rules, shared by every writing
/// method (see [`crate::writer`]).
pub const STORYTELLER_RULES: &str = "\
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
référence [réf. …] : elle sert au plan, ne la cite jamais dans l'histoire.";

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
         mots, et se joue avec {boxes}.{wish}",
        language = options.language,
        age = options.reader_age,
        words = options.target_words,
    )
}

/// Single-call prompt: the whole story in one generation.
pub fn build_prompt(collection: &Collection, options: &StoryOptions) -> Prompt {
    let context = build_context(collection, &options.context);
    let request = reader_request(collection, options);
    Prompt {
        system: format!("{STORYTELLER_RULES}\n\n{DIRECT_STYLE}"),
        user: format!("{context}\n\nÉcris cette histoire. {request}\n"),
    }
}
