//! End-to-end tests with the LLM bypassed (`ReplayGenerator`): the whole
//! pipeline runs for real (saved inventory, prompts, plan validation,
//! drafting, review, revision, "In your box"), the model's answers being
//! read from files written by a person or an assistant.
//!
//! Each case lives in `tests/e2e/<sets joined by +>/`: `inventory.json` (from
//! `--inventory-json`), `answers/NN-*.answer.txt`, and the expected outputs
//! in `expected/` (final story and every prompt). After an intended change
//! of prompts or output, regenerate them with:
//! `UPDATE_GOLDEN=1 cargo test --test e2e_replay`.

use std::path::{Path, PathBuf};

use rag_mobil::augment::StoryOptions;
use rag_mobil::generator::ReplayGenerator;
use rag_mobil::retriever::JsonFile;
use rag_mobil::writer::WriterOptions;
use rag_mobil::{RagPipeline, SetNumber, Story};

fn case_dir(sets: &[&str]) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e")
        .join(sets.join("+"))
}

/// Compares `actual` with the reference file, or rewrites it with UPDATE_GOLDEN.
fn golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "{} manquant (UPDATE_GOLDEN=1 pour le créer)",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "{} diffère de la sortie actuelle (UPDATE_GOLDEN=1 si le changement est voulu)\n\
         --- attendu ---\n{expected}\n--- obtenu ---\n{actual}",
        path.display()
    );
}

/// Runs the replayed pipeline on `sets` and checks the story and every
/// prompt against `expected/`.
async fn replay(sets: &[&str]) -> Story {
    let dir = case_dir(sets);
    let prompts = std::env::temp_dir().join(format!(
        "rag-mobil-e2e-{}-{}",
        sets.join("+"),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&prompts);

    let pipeline = RagPipeline::new(
        JsonFile(dir.join("inventory.json")),
        ReplayGenerator::with_prompt_dir(dir.join("answers"), &prompts),
    );
    let story = pipeline
        .run(
            &sets
                .iter()
                .map(|s| SetNumber::parse(s).unwrap())
                .collect::<Vec<_>>(),
            &StoryOptions::default(),
            WriterOptions::default(),
            &mut (),
        )
        .await
        .unwrap();

    golden(
        &dir.join("expected/story.md"),
        &story.written.generation.text,
    );
    let mut names: Vec<_> = std::fs::read_dir(&prompts)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    for name in &names {
        let actual = std::fs::read_to_string(prompts.join(name)).unwrap();
        golden(&dir.join("expected/prompts").join(name), &actual);
    }
    let expected_count = std::fs::read_dir(dir.join("expected/prompts"))
        .unwrap()
        .count();
    assert_eq!(names.len(), expected_count, "nombre d'appels au LLM");
    let _ = std::fs::remove_dir_all(&prompts);
    story
}

/// Set 3799 "Pirates Secret Island", answered by Claude through the replay.
#[tokio::test]
async fn set_3799_pirates_secret_island() {
    let story = replay(&["3799"]).await;
    let written = &story.written;
    let text = &written.generation.text;

    // Plan accepted at once, 4 drafts, review, one revision.
    assert_eq!(written.calls, 7);
    assert_eq!(written.plan.as_ref().unwrap().outline.len(), 4);

    // The review caught the key that isn't in the box; the revision fixed it.
    assert_eq!(written.issues.len(), 1);
    assert!(written.issues[0].excerpt.contains("tourna la clé"));
    assert!(!text.contains("tourna la clé"));
    assert!(text.contains("fit glisser le loquet de la serrure"));

    // "In your box" completes objects split in several pieces.
    assert!(text.contains("- 1× Parrot body (light blue)\n- 1× Parrot wings (yellow)\n"));
    assert!(
        text.contains("- 9× Flower blossom (dark pink)\n- 4× Flower stem (single) (dark green)\n")
    );
    assert!(text.contains("- Pépin — Pirate, red vest and ponytail"));
}

/// Two boxes mixed: 3799 "Pirates Secret Island" + 70145 "Diving Scooter"
/// (whose diver is "(no name)", with no counts and German part names).
#[tokio::test]
async fn sets_3799_and_70145_mixed() {
    let story = replay(&["3799", "70145"]).await;
    let written = &story.written;
    let text = &written.generation.text;

    assert_eq!(story.collection.sets.len(), 2);
    assert_eq!(written.calls, 7);
    // Characters from both boxes, accepted by the plan validation.
    let plan = written.plan.as_ref().unwrap();
    assert_eq!(plan.characters.len(), 4);
    assert!(text.contains("- Pépin — Pirate, red vest and ponytail (boîte 3799)\n"));
    assert!(text.contains("- Marine — (no name) (boîte 70145)\n"));

    // The review caught a fishing float and a diving lamp, in neither box.
    assert_eq!(written.issues.len(), 2);
    assert!(!text.contains("bouchon"));
    assert!(!text.contains("en éclairant"));

    // Each object says which box to look in.
    assert!(text.contains("- 2× Sea turtle (brown/tan) (boîte 3799)\n"));
    assert!(text.contains("- Diver's scuba airtank (grey) (boîte 70145)\n"));
}
