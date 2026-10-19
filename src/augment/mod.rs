//! **A** of RAG: augment the request with the retrieved inventory.

pub mod context;
pub mod prompt;

pub use context::{ContextOptions, build_context};
pub use prompt::{
    CITATION_FORMAT, Prompt, StoryOptions, build_prompt, reader_request, storyteller_rules,
};
