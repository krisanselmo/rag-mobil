//! **A** of RAG: augment the request with the retrieved inventory.

pub mod context;
pub mod prompt;

pub use context::{ContextOptions, build_context};
pub use prompt::{Prompt, StoryOptions, build_prompt};
