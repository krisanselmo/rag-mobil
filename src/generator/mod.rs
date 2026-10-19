//! **G** of RAG: generate the story from the augmented prompt.

pub mod llm;
pub mod replay;

use std::future::Future;

use serde_json::Value;

use crate::augment::Prompt;
use crate::error::Result;

pub use genai::chat::ReasoningEffort;
pub use llm::{LlmConfig, LlmGenerator};
pub use replay::ReplayGenerator;

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

    /// Structured output: the returned `text` is a JSON document following
    /// `schema` (`name` identifies the schema for providers that need one).
    fn generate_json(
        &self,
        prompt: &Prompt,
        name: &str,
        schema: &Value,
    ) -> impl Future<Output = Result<Generation>> + Send;
}

/// Lets a generator be shared by reference.
impl<T: StoryGenerator + Sync> StoryGenerator for &T {
    fn generate<F>(
        &self,
        prompt: &Prompt,
        on_text: F,
    ) -> impl Future<Output = Result<Generation>> + Send
    where
        F: FnMut(&str) + Send,
    {
        (**self).generate(prompt, on_text)
    }

    fn generate_json(
        &self,
        prompt: &Prompt,
        name: &str,
        schema: &Value,
    ) -> impl Future<Output = Result<Generation>> + Send {
        (**self).generate_json(prompt, name, schema)
    }
}
