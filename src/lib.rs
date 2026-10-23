//! # rag-mobil
//!
//! A RAG pipeline that writes Playmobil stories faithful to a set's contents:
//!
//! 1. [`retriever`] — scrape the set inventory from PlaymoDB;
//! 2. [`augment`] — turn it into a grounding context and a prompt;
//! 3. [`generator`] — LLM backend (Claude by default, via `genai`);
//! 4. [`writer`] — the writing method: one call, or Plan → Draft → Edit
//!    adapted from the open-source Re3/DOC story generators.
//!
//! [`pipeline::RagPipeline`] chains them; [`html`] renders the story as a
//! page with the pictures of the parts (and the cover drawn by
//! [`illustrate`]), [`pdf`] prints it.

pub mod augment;
pub mod error;
pub mod generator;
pub mod html;
pub mod illustrate;
pub mod model;
pub mod pdf;
pub mod pipeline;
pub mod retriever;
pub mod saga;
pub mod writer;

pub use error::{RagError, Result};
pub use model::{Part, SetInventory, SetNumber};
pub use pipeline::{RagPipeline, Story};
