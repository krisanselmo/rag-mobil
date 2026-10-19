//! LLM bypass: answers come from files instead of a model. Used by the
//! end-to-end tests and to run the real pipeline by hand (a person or
//! another assistant answers each prompt).
//!
//! Call `N` writes its prompt to `NN-<kind>.prompt.txt` in the prompt
//! directory and reads its answer from `NN-<kind>.answer.txt` in the answer
//! directory (`kind`: `text` or `json`). A missing answer stops the run with
//! [`RagError::ReplayMissing`]: write it, run again, the earlier answers are
//! replayed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::augment::Prompt;
use crate::error::{RagError, Result};
use crate::generator::{Generation, StoryGenerator};

pub struct ReplayGenerator {
    answers: PathBuf,
    prompts: PathBuf,
    calls: AtomicUsize,
}

impl ReplayGenerator {
    /// Prompts and answers in the same directory.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self::with_prompt_dir(dir.clone(), dir)
    }

    /// Reads answers from `answers`, writes prompts to `prompts`.
    pub fn with_prompt_dir(answers: impl Into<PathBuf>, prompts: impl Into<PathBuf>) -> Self {
        Self {
            answers: answers.into(),
            prompts: prompts.into(),
            calls: AtomicUsize::new(0),
        }
    }

    fn answer(&self, kind: &str, prompt: &Prompt) -> Result<Generation> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let prompt_path = self.prompts.join(format!("{n:02}-{kind}.prompt.txt"));
        let answer_path = self.answers.join(format!("{n:02}-{kind}.answer.txt"));
        write(&prompt_path, &render(prompt))?;
        match std::fs::read_to_string(&answer_path) {
            Ok(text) => Ok(Generation {
                text,
                model: "replay".into(),
                truncated: false,
                output_tokens: None,
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(RagError::ReplayMissing {
                    prompt: prompt_path,
                    answer: answer_path,
                })
            }
            Err(err) => Err(RagError::Io {
                path: answer_path,
                source: err,
            }),
        }
    }
}

/// A prompt as one text file.
pub fn render(prompt: &Prompt) -> String {
    format!(
        "=== SYSTEM ===\n{}\n\n=== USER ===\n{}\n",
        prompt.system, prompt.user
    )
}

fn write(path: &Path, content: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| RagError::Io {
            path: dir.to_owned(),
            source,
        })?;
    }
    std::fs::write(path, content).map_err(|source| RagError::Io {
        path: path.to_owned(),
        source,
    })
}

impl StoryGenerator for ReplayGenerator {
    async fn generate<F>(&self, prompt: &Prompt, mut on_text: F) -> Result<Generation>
    where
        F: FnMut(&str) + Send,
    {
        let generation = self.answer("text", prompt)?;
        on_text(&generation.text);
        Ok(generation)
    }

    async fn generate_json(&self, prompt: &Prompt, _: &str, _: &Value) -> Result<Generation> {
        self.answer("json", prompt)
    }
}
