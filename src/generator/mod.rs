//! **G** of RAG: generate the story from the augmented prompt.

pub mod llm;
pub mod replay;

use std::future::Future;

use serde_json::Value;

use crate::augment::Prompt;
use crate::error::Result;

pub use genai::chat::ReasoningEffort;
pub use llm::{LlmConfig, LlmGenerator, OPENAI_BASE_URL_ENV, openai_compatible_client};
pub use replay::ReplayGenerator;

/// The model's answer.
#[derive(Debug, Clone)]
pub struct Generation {
    pub text: String,
    /// Model that actually produced the text (may differ after a fallback).
    pub model: String,
    /// `true` if generation stopped on `max_tokens`.
    pub truncated: bool,
    pub usage: TokenUsage,
}

/// Tokens of one or more calls, as reported by the provider (aggregated by
/// genai, approximate); `None` when it didn't say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// Every input token, read from the prompt cache or not.
    pub input: Option<u64>,
    /// Input tokens read from the prompt cache (a fraction of the price).
    pub cache_read: Option<u64>,
    /// Input tokens written to the prompt cache (a little more than the
    /// price, for the next calls to read them).
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
}

impl TokenUsage {
    /// Adds the tokens of another call.
    pub fn add(&mut self, other: &Self) {
        let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.input = sum(self.input, other.input);
        self.cache_read = sum(self.cache_read, other.cache_read);
        self.cache_write = sum(self.cache_write, other.cache_write);
        self.output = sum(self.output, other.output);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_what_is_known() {
        let mut total = TokenUsage::default();
        total.add(&TokenUsage {
            input: Some(100),
            cache_write: Some(80),
            output: Some(10),
            ..TokenUsage::default()
        });
        total.add(&TokenUsage {
            input: Some(120),
            cache_read: Some(80),
            output: Some(20),
            ..TokenUsage::default()
        });
        assert_eq!(
            total,
            TokenUsage {
                input: Some(220),
                cache_read: Some(80),
                cache_write: Some(80),
                output: Some(30),
            }
        );
    }
}
