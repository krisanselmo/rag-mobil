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
             Avant d'écrire, tu prépares le plan de l'histoire : une prémisse, un décor, \
             les personnages, puis les grandes étapes de l'intrigue. Réponds uniquement \
             avec le JSON demandé.",
            storyteller_rules(options)
        ),
        user: format!(
            "{context}\n\n{request}\n\n\
             Prépare le plan de cette histoire en {steps} étapes (un début, des \
             péripéties, une résolution). Chaque personnage correspond à une figurine \
             différente de <personnages>, désignée par sa référence ; une même figurine ne \
             peut jouer plusieurs personnages que si la boîte en contient plusieurs. Pour \
             chaque étape, indique les personnages présents (par leur nom) et les \
             références des objets, animaux ou véhicules de <pieces> qu'ils utilisent. \
             Rédige les textes du plan en {language}.",
            language = options.language,
        ),
    }
}

/// Re-asks for the plan, with the problems found in the previous one.
pub(crate) fn plan_fix(base: &Prompt, previous: &str, errors: &[String]) -> Prompt {
    let mut user = format!(
        "{}\n\nVoici ton plan précédent :\n{previous}\n\n\
         Il n'est pas fidèle à la boîte :\n",
        base.user
    );
    for error in errors {
        let _ = writeln!(user, "- {error}");
    }
    user.push_str(
        "\nRenvoie le plan complet corrigé, en n'utilisant que des références présentes \
         dans <set>.",
    );
    Prompt {
        system: base.system.clone(),
        user,
    }
}

/// Plan of an interactive story: a graph of paragraphs linked by choices.
pub(crate) fn game_plan(boxes: &Collection, options: &StoryOptions) -> Prompt {
    let context = plan_context(boxes, options);
    let request = plan_request(boxes, options);
    Prompt {
        system: format!(
            "{}\n\n\
             Avant d'écrire, tu prépares le plan d'une histoire interactive, dont le \
             lecteur choisit la suite (comme un livre « dont tu es le héros ») : une \
             prémisse, un décor, les personnages, puis des paragraphes reliés par des \
             choix. Réponds uniquement avec le JSON demandé.",
            storyteller_rules(options)
        ),
        user: format!(
            "{context}\n\n{request}\n\n\
             Prépare le plan de cette histoire interactive en 6 à 9 paragraphes. Le premier \
             paragraphe est le début. Chaque paragraphe qui n'est pas une fin se termine \
             par 2 choix (3 au plus) proposés au lecteur, chacun menant à un autre \
             paragraphe, désigné par son identifiant ; deux choix peuvent mener au même \
             paragraphe. Un paragraphe sans choix est une fin : prévois 2 à 4 fins \
             différentes, toutes heureuses ou rassurantes. Pas de boucle : un choix mène \
             toujours plus loin dans l'histoire, et chaque paragraphe est atteint par au \
             moins un choix. Chaque personnage correspond à une figurine différente de \
             <personnages>, désignée par sa référence ; une même figurine ne peut jouer \
             plusieurs personnages que si la boîte en contient plusieurs. Pour chaque \
             paragraphe, indique les personnages présents (par leur nom) et les références \
             des objets, animaux ou véhicules de <pieces> qu'ils utilisent ; un choix est \
             une décision courte, adressée au lecteur (« Pépin entre dans la grotte »). \
             Rédige les textes du plan en {language}.",
            language = options.language,
        ),
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
    if !path.is_empty() {
        let _ = write!(
            user,
            "<histoire_jusqu_ici>\n{path}\n</histoire_jusqu_ici>\n\n"
        );
    }
    let (characters, objects) = step_cast(boxes, options, plan, index);
    let position = if index == 0 {
        "C'est le début : installe le décor et présente les personnages.".to_owned()
    } else if choices.is_empty() {
        "C'est une des fins de l'histoire : conclus-la, de façon heureuse ou rassurante.".to_owned()
    } else {
        String::new()
    };
    let ending = if choices.is_empty() {
        String::new()
    } else {
        format!(
            "Le paragraphe s'arrête au moment où il faut choisir entre : {}. N'écris pas \
             ces choix, ils seront ajoutés après ton texte.",
            choices.join(" ; ")
        )
    };
    let _ = write!(
        user,
        "Écris le paragraphe {n} en {language}, pour un enfant de {age} ans, d'environ \
         {words} mots.\n\
         Ce qui se passe : {event}\n\
         Personnages présents : {characters}\n\
         Objets de la boîte à utiliser : {objects}\n\
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
    }
}

const DRAFT_STYLE: &str = "\
Tu rédiges l'histoire partie par partie, en suivant un plan validé. Style : adapté \
à l'âge demandé, chaleureux, sans violence graphique, avec des dialogues. Écris \
uniquement la partie demandée, en prose : ni titre, ni intertitre, ni commentaire, \
ni liste de pièces.";

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
    if !story_so_far.is_empty() {
        let _ = write!(
            user,
            "<histoire_jusqu_ici>\n{story_so_far}\n</histoire_jusqu_ici>\n\n"
        );
    }

    let (characters, objects) = step_cast(boxes, options, plan, index);
    let position = if index == 0 {
        "C'est le début : installe le décor et présente les personnages."
    } else if index + 1 == total {
        "C'est la dernière partie : conclus l'histoire."
    } else {
        "Ne conclus pas encore l'histoire."
    };

    let _ = write!(
        user,
        "Écris la partie {n}/{total} en {language}, pour un enfant de {age} ans, \
         d'environ {words} mots.\n\
         Ce qui se passe : {event}\n\
         Personnages présents : {characters}\n\
         Objets de la boîte à utiliser : {objects}\n\
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
    }
}

fn or_none(items: &[String]) -> String {
    if items.is_empty() {
        "aucun en particulier".to_owned()
    } else {
        items.join(" ; ")
    }
}

/// Re3's "Edit" module, adapted: a reviewer looks for anything the story
/// uses that the box doesn't contain.
pub(crate) fn review(boxes: &Collection, options: &StoryOptions, story: &str) -> Prompt {
    let context = build_context(boxes, &options.context);
    Prompt {
        system: "\
Tu es relecteur pour une maison d'édition jeunesse. Tu vérifies qu'une histoire peut \
être rejouée avec le contenu exact d'une boîte Playmobil, dont l'inventaire est \
fourni entre balises <set>. Réponds uniquement avec le JSON demandé."
            .to_owned(),
        user: format!(
            "{context}\n\n<histoire>\n{story}\n</histoire>\n\n\
             Relève les problèmes de fidélité à la boîte :\n\
             - un personnage qui agit sans correspondre à une figurine de <personnages> ;\n\
             - un objet, animal ou véhicule utilisé qui n'est pas dans <pieces>, ou en \
             plus grand nombre que dans la boîte ;\n\
             - une apparence (vêtements, couleurs, coiffure) qui contredit la figurine.\n\
             Ne relève pas ce qui est seulement évoqué, imaginé par les personnages ou \
             construit avec les pièces de décor. Ne relève que de vrais problèmes : une \
             liste vide est une bonne réponse. Écris en {}.",
            options.language
        ),
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
        "{context}\n\n<histoire>\n{story}\n</histoire>\n\n\
         Un relecteur a relevé ces écarts avec le contenu de la boîte :\n"
    );
    for issue in issues {
        let _ = writeln!(
            user,
            "- « {} » : {} → {}",
            issue.excerpt, issue.problem, issue.fix
        );
    }
    let _ = write!(
        user,
        "\nRéécris l'histoire complète en {} en corrigeant uniquement ces écarts ; \
         garde tout le reste à l'identique. Ni titre, ni commentaire, ni liste de pièces.",
        options.language
    );
    if let Some(count) = scenes {
        let _ = write!(
            user,
            " L'histoire est en {count} parties séparées par des lignes « {SCENE_BREAK} » : \
             garde ces séparateurs, au même endroit."
        );
    }
    Prompt {
        system: format!(
            "{}\n\nTu corriges une histoire existante.",
            storyteller_rules(options)
        ),
        user,
    }
}
