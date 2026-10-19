use thiserror::Error;

/// Every error the pipeline can surface, by stage.
#[derive(Debug, Error)]
pub enum RagError {
    #[error("numéro de set invalide : « {0} » (attendu : chiffres uniquement, ex. 3666)")]
    InvalidSetNumber(String),

    #[error("le set {0} n'existe pas dans PlaymoDB")]
    SetNotFound(String),

    #[error("le set {0} existe dans PlaymoDB mais son inventaire est vide")]
    EmptyInventory(String),

    #[error("requête HTTP échouée : {0}")]
    Http(#[from] reqwest::Error),

    #[error("réponse PlaymoDB inattendue ({url}) : HTTP {status}")]
    ScraperStatus { url: String, status: u16 },

    #[error("variable d'environnement {0} manquante")]
    MissingEnv(&'static str),

    #[error("erreur de l'API Claude (HTTP {status}, {kind}) : {message}")]
    LlmApi {
        status: u16,
        kind: String,
        message: String,
    },

    #[error("erreur pendant le streaming Claude ({kind}) : {message}")]
    LlmStream { kind: String, message: String },

    #[error("Claude a décliné la requête (catégorie : {category})")]
    LlmRefusal { category: String },

    #[error("JSON invalide : {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RagError>;
