//! Prompts of the planned method (Plan → Draft → Edit).

use std::fmt::Write;

use crate::augment::{Prompt, STORYTELLER_RULES, StoryOptions, build_context, reader_request};
use crate::model::Collection;
use crate::writer::plan::{Issue, StoryPlan, find_part, part_label, render_plan, step_objects};

/// Number of plot points for a story of `words` words.
pub(crate) fn outline_length(words: u32) -> usize {
    match words {
        0..=400 => 3,
        401..=900 => 4,
        _ => 5,
    }
}

pub(crate) fn plan(boxes: &Collection, options: &StoryOptions) -> Prompt {
    let context = build_context(boxes, &options.context);
    let request = reader_request(boxes, options);
    let steps = outline_length(options.target_words);
    Prompt {
        system: format!(
            "{STORYTELLER_RULES}\n\n\
             Avant d'écrire, tu prépares le plan de l'histoire : une prémisse, un décor, \
             les personnages, puis les grandes étapes de l'intrigue. Réponds uniquement \
             avec le JSON demandé."
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

    let characters: Vec<String> = step
        .characters
        .iter()
        .map(|name| {
            plan.characters
                .iter()
                .find(|c| c.name.trim().eq_ignore_ascii_case(name.trim()))
                .and_then(|c| find_part(boxes, &c.figure))
                .map_or_else(|| name.clone(), |p| format!("{name} ({})", part_label(p)))
        })
        .collect();
    let objects = step_objects(step, boxes);
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
        system: format!("{STORYTELLER_RULES}\n\n{DRAFT_STYLE}"),
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

/// Rewrites the story, fixing only the reviewed issues.
pub(crate) fn revise(
    boxes: &Collection,
    options: &StoryOptions,
    story: &str,
    issues: &[Issue],
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
    Prompt {
        system: format!("{STORYTELLER_RULES}\n\nTu corriges une histoire existante."),
        user,
    }
}
