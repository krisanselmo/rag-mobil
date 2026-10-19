//! Prompt templates: a stable system prompt (the storyteller's rules) and a
//! user message carrying the retrieved inventory plus the request.

use crate::augment::context::{ContextOptions, build_context};
use crate::model::SetInventory;

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

const SYSTEM_PROMPT: &str = "\
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
naturellement dans la langue de l'histoire.

Style : une histoire complète (début, péripétie, résolution), adaptée à l'âge \
demandé, chaleureuse, sans violence graphique, avec des dialogues. Commence par un \
titre sur la première ligne, puis l'histoire. Termine par une courte section \
\"Dans ta boîte\" listant les figurines et les objets de l'inventaire utilisés \
dans l'histoire, pour que l'enfant puisse les retrouver.";

pub fn build_prompt(inventory: &SetInventory, options: &StoryOptions) -> Prompt {
    let context = build_context(inventory, &options.context);
    let wish = options
        .extra_instructions
        .as_deref()
        .map(|w| format!("\nSouhait particulier de l'enfant : {w}\n"))
        .unwrap_or_default();

    let user = format!(
        "{context}\n\n\
         Écris une histoire en {language} pour un enfant de {age} ans, d'environ \
         {words} mots, qui se joue avec le set Playmobil {number} « {name} » décrit \
         ci-dessus.\n{wish}",
        language = options.language,
        age = options.reader_age,
        words = options.target_words,
        number = inventory.set_number,
        name = inventory.name,
    );

    Prompt {
        system: SYSTEM_PROMPT.to_owned(),
        user,
    }
}
