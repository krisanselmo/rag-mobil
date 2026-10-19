//! **G** of RAG: generate the story from the augmented prompt.

pub mod llm;

use std::future::Future;

use crate::augment::Prompt;
use crate::error::Result;

pub use genai::chat::ReasoningEffort;
pub use llm::{LlmConfig, LlmGenerator};

/// The model's answer.
#[derive(Debug, Clone)]
pub struct Generation {
    pub text: String,
    /// Model that actually produced the text (may differ after a fallback).
    pub model: String,
    /// `true` if generation stopped on `max_tokens`.
    pub truncated: bool,
    /// Approximate (provider-reported, as aggregated by genai).
    pub output_tokens: Option<u64>,
}

/// An LLM backend. `on_text` receives text chunks as they stream in.
pub trait StoryGenerator {
    fn generate<F>(
        &self,
        prompt: &Prompt,
        on_text: F,
    ) -> impl Future<Output = Result<Generation>> + Send
    where
        F: FnMut(&str) + Send;
}
