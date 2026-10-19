use thiserror::Error;

/// Every error the pipeline can surface, by stage.
#[derive(Debug, Error)]
pub enum RagError {
    #[error("numéro de set invalide : « {0} » (attendu : chiffres uniquement, ex. 3666)")]
    InvalidSetNumber(String),

    /// The payload says which set and which source(s) were asked.
    #[error("set introuvable : {0}")]
    SetNotFound(String),

    #[error("le set {0} est connu mais son inventaire est vide")]
    EmptyInventory(String),

    #[error("requête HTTP échouée : {0}")]
    Http(#[from] reqwest::Error),

    #[error("réponse PlaymoDB inattendue ({url}) : HTTP {status}")]
    ScraperStatus { url: String, status: u16 },

    #[error("réponse inattendue de la source d'inventaire ({url}) : HTTP {status}")]
    SourceStatus { url: String, status: u16 },

    #[error("inventaire invalide ({url}) : {reason}")]
    InvalidInventory { url: String, reason: String },

    #[error(
        "adresse de source d'inventaire invalide : « {0} » (attendu : une adresse http(s) contenant {{set}})"
    )]
    InvalidInventoryUrl(String),

    #[error("erreur du fournisseur LLM : {0}")]
    Llm(#[from] genai::Error),

    #[error("le modèle {model} a décliné la requête")]
    LlmRefusal { model: String },

    #[error(
        "réponse manquante : écrivez {} (le prompt est dans {}) puis relancez",
        answer.display(),
        prompt.display()
    )]
    ReplayMissing {
        prompt: std::path::PathBuf,
        answer: std::path::PathBuf,
    },

    #[error("fichier {} : {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("JSON invalide : {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RagError>;
