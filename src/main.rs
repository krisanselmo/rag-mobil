use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use rag_mobil::augment::{ContextOptions, StoryOptions};
use rag_mobil::generator::{LlmConfig, LlmGenerator, ReasoningEffort, llm};
use rag_mobil::retriever::{InventorySource, PlaymoDb, playmodb};
use rag_mobil::writer::{Observer, Step, WriterOptions, WritingMethod, first_prompt};
use rag_mobil::{RagError, RagPipeline, SetNumber};

/// Méthode de rédaction.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Method {
    /// Plan, rédaction étape par étape, relecture (méthode Re3/DOC).
    Planned,
    /// Un seul appel, l'histoire s'affiche au fil de l'écriture.
    Direct,
}

impl From<Method> for WritingMethod {
    fn from(method: Method) -> Self {
        match method {
            Method::Planned => Self::Planned,
            Method::Direct => Self::Direct,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl From<Effort> for ReasoningEffort {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
            Effort::Xhigh => Self::XHigh,
            Effort::Max => Self::Max,
        }
    }
}

/// Génère une histoire Playmobil fidèle au contenu d'une boîte (inventaire PlaymoDB + LLM).
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Numéro du set Playmobil (ex. 3666).
    set_number: String,

    /// Langue de l'histoire.
    #[arg(short, long, default_value = "français")]
    language: String,

    /// Âge du lecteur.
    #[arg(short, long, default_value_t = 7)]
    age: u8,

    /// Longueur visée, en mots.
    #[arg(short, long, default_value_t = 600)]
    words: u32,

    /// Souhait libre (ex. "une histoire de fantômes").
    #[arg(short, long)]
    theme: Option<String>,

    /// Méthode de rédaction.
    #[arg(long, value_enum, default_value_t = Method::Planned)]
    method: Method,

    /// Méthode planned : pas de relecture de fidélité à la fin.
    #[arg(long)]
    no_review: bool,

    /// Garde toutes les catégories de pièces (y compris les notices papier).
    #[arg(long)]
    include_all: bool,

    /// Modèle LLM ; le fournisseur est déduit du nom : claude-… (Anthropic),
    /// gpt-… (OpenAI), gemini-… (Gemini), ollama::<modèle> (local, gratuit)…
    #[arg(short, long, env = "RAG_MOBIL_MODEL", default_value = llm::DEFAULT_MODEL)]
    model: String,

    /// Niveau d'effort / de réflexion (défaut : medium pour Claude, celui du
    /// fournisseur sinon).
    #[arg(long, value_enum)]
    effort: Option<Effort>,

    /// Modèle relancé une fois si Claude décline la requête.
    #[arg(long, default_value = llm::DEFAULT_FALLBACK_MODEL, conflicts_with = "no_fallback")]
    fallback_model: String,

    /// Pas de nouvelle tentative en cas de refus.
    #[arg(long)]
    no_fallback: bool,

    /// URL de base de PlaymoDB.
    #[arg(long, env = "PLAYMODB_BASE_URL", default_value = playmodb::DEFAULT_BASE_URL)]
    playmodb_url: String,

    /// Affiche l'inventaire récupéré en JSON et s'arrête (pas d'appel LLM).
    #[arg(long, conflicts_with = "dry_run")]
    inventory_json: bool,

    /// Affiche le prompt augmenté et s'arrête (pas d'appel LLM).
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Erreur : {err:#}");
            if let Some(hint) = err.downcast_ref::<RagError>().and_then(hint) {
                eprintln!("Astuce : {hint}");
            }
            ExitCode::FAILURE
        }
    }
}

fn hint(err: &RagError) -> Option<String> {
    match err {
        RagError::Llm(genai::Error::Resolver {
            resolver_error: genai::resolver::Error::ApiKeyEnvNotFound { env_name },
            ..
        }) => Some(format!(
            "définissez {env_name}, choisissez un modèle local gratuit \
             (ex. -m ollama::llama3.2) ou utilisez --dry-run"
        )),
        _ => None,
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let set = SetNumber::parse(&cli.set_number)?;
    let source = PlaymoDb::new(&cli.playmodb_url)?;
    let options = StoryOptions {
        language: cli.language,
        reader_age: cli.age,
        target_words: cli.words,
        extra_instructions: cli.theme,
        context: ContextOptions {
            include_all: cli.include_all,
        },
    };

    // Offline modes: retrieval (+ augmentation) only.
    if cli.inventory_json || cli.dry_run {
        let inventory = source.fetch(&set).await?;
        if cli.inventory_json {
            println!("{}", serde_json::to_string_pretty(&inventory)?);
        } else {
            let prompt = first_prompt(&inventory, &options, cli.method.into());
            println!(
                "=== SYSTEM ===\n{}\n\n=== USER ===\n{}",
                prompt.system, prompt.user
            );
        }
        return Ok(());
    }

    let generator = LlmGenerator::new(LlmConfig {
        model: cli.model,
        effort: cli.effort.map(Into::into),
        fallback_model: (!cli.no_fallback).then_some(cli.fallback_model),
    });
    let pipeline = RagPipeline::new(source, generator);

    let writer = WriterOptions {
        method: cli.method.into(),
        review: !cli.no_review,
    };

    eprintln!("[rag-mobil] récupération de l'inventaire du set {set} sur PlaymoDB…");
    let story = pipeline.run(&set, &options, writer, &mut Terminal).await?;
    println!();

    let g = &story.written.generation;
    eprintln!(
        "[rag-mobil] set {} « {} » — {} pièces distinctes, {} figurine(s) — modèle {}, \
         {} appel(s){}",
        story.inventory.set_number,
        story.inventory.name,
        story.inventory.parts.len(),
        story.inventory.figures().count(),
        g.model,
        story.written.calls,
        g.output_tokens
            .map(|t| format!(", ~{t} tokens générés"))
            .unwrap_or_default(),
    );
    if g.truncated {
        eprintln!("[rag-mobil] attention : histoire tronquée (limite max_tokens atteinte)");
    }
    Ok(())
}

/// Story on stdout, progress on stderr.
struct Terminal;

impl Observer for Terminal {
    fn text(&mut self, chunk: &str) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(chunk.as_bytes());
        let _ = stdout.flush();
    }

    fn step(&mut self, step: Step<'_>) {
        match step {
            Step::Planning => eprintln!("[rag-mobil] plan de l'histoire…"),
            Step::PlanRejected(errors) => eprintln!(
                "[rag-mobil] plan pas fidèle à la boîte ({} écart(s)), nouvelle demande…",
                errors.len()
            ),
            Step::PlanRepaired(errors) => {
                eprintln!("[rag-mobil] références invalides retirées du plan :");
                for error in errors {
                    eprintln!("  - {error}");
                }
            }
            Step::PlanReady(plan) => eprintln!(
                "[rag-mobil] plan : « {} » — {} personnage(s), {} étape(s)",
                plan.title,
                plan.characters.len(),
                plan.outline.len()
            ),
            Step::Drafting { part, total } => {
                eprintln!("[rag-mobil] rédaction {part}/{total}…");
            }
            Step::Reviewing => eprintln!("[rag-mobil] relecture de fidélité…"),
            Step::ReviewUnreadable => {
                eprintln!("[rag-mobil] relecture illisible, histoire gardée sans correction");
            }
            Step::Revising(issues) => {
                eprintln!("[rag-mobil] {} écart(s) corrigé(s) :", issues.len());
                for issue in issues {
                    eprintln!("  - {}", issue.problem);
                }
            }
        }
    }
}
