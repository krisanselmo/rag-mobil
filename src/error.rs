use thiserror::Error;

/// Every error the pipeline can surface, by stage.
#[derive(Debug, Error)]
pub enum RagError {
    #[error(
        "invalid set number: \"{0}\" (expected: digits, optionally followed by a letter, e.g. 3666 or 3258a)"
    )]
    InvalidSetNumber(String),

    #[error(
        "invalid missing piece: \"{0}\" (expected: a reference, optionally followed \
         by :N for N copies, e.g. 30 07 640:2)"
    )]
    InvalidMissingPart(String),

    /// The payload says which set and which source(s) were asked.
    #[error("set not found: {0}")]
    SetNotFound(String),

    #[error("set {0} is known but its inventory is empty")]
    EmptyInventory(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("unexpected PlaymoDB response ({url}): HTTP {status}")]
    ScraperStatus { url: String, status: u16 },

    #[error("unexpected response from the inventory source ({url}): HTTP {status}")]
    SourceStatus { url: String, status: u16 },

    #[error("invalid inventory ({url}): {reason}")]
    InvalidInventory { url: String, reason: String },

    #[error(
        "invalid inventory source address: \"{0}\" (expected: an http(s) address containing {{set}})"
    )]
    InvalidInventoryUrl(String),

    #[error("LLM provider error: {0}")]
    Llm(#[from] genai::Error),

    #[error("model {model} declined the request")]
    LlmRefusal { model: String },

    #[error(
        "missing answer: write {} (the prompt is in {}) then run again",
        answer.display(),
        prompt.display()
    )]
    ReplayMissing {
        prompt: std::path::PathBuf,
        answer: std::path::PathBuf,
    },

    #[error("file {}: {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("illustration failed: {0}")]
    Image(String),

    #[error("PDF printing failed: {0}")]
    Pdf(String),

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RagError>;
