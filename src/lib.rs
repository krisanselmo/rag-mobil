//! # rag-mobil
//!
//! A RAG pipeline that writes Playmobil stories faithful to a set's contents:
//!
//! 1. [`retriever`] — scrape the set inventory from PlaymoDB;
//! 2. [`augment`] — turn it into a grounding context and a prompt;
//! 3. [`generator`] — have an LLM (Claude by default, via `genai`) write the story.
//!
//! [`pipeline::RagPipeline`] chains the three.

pub mod augment;
pub mod error;
pub mod generator;
pub mod model;
pub mod pipeline;
pub mod retriever;

pub use error::{RagError, Result};
pub use model::{Part, SetInventory, SetNumber};
pub use pipeline::{RagPipeline, Story};
