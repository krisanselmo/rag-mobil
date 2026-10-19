use std::io::Write;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, ValueEnum};

use rag_mobil::augment::{ContextOptions, StoryOptions, build_prompt};
use rag_mobil::generator::{AnthropicClient, AnthropicConfig, anthropic};
use rag_mobil::retriever::{InventorySource, PlaymoDb, playmodb};
use rag_mobil::{RagPipeline, SetNumber};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// Génère une histoire Playmobil fidèle au contenu d'une boîte (inventaire PlaymoDB + Claude).
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

    /// Garde toutes les catégories de pièces (y compris les notices papier).
    #[arg(long)]
    include_all: bool,

    /// Modèle Claude.
    #[arg(long, env = "RAG_MOBIL_MODEL", default_value = anthropic::DEFAULT_MODEL)]
    model: String,

    /// Niveau d'effort du modèle.
    #[arg(long, value_enum, default_value_t = Effort::Medium)]
    effort: Effort,

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
            ExitCode::FAILURE
        }
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
            let prompt = build_prompt(&inventory, &options);
            println!(
                "=== SYSTEM ===\n{}\n\n=== USER ===\n{}",
                prompt.system, prompt.user
            );
        }
        return Ok(());
    }

    let mut config = AnthropicConfig::from_env()
        .context("définissez ANTHROPIC_API_KEY (ou utilisez --dry-run)")?;
    config.model = cli.model;
    config.effort = cli.effort.as_str().to_owned();
    let pipeline = RagPipeline::new(source, AnthropicClient::new(config)?);

    eprintln!("[rag-mobil] récupération de l'inventaire du set {set} sur PlaymoDB…");
    let mut stdout = std::io::stdout();
    let story = pipeline
        .run(&set, &options, |chunk| {
            let _ = stdout.write_all(chunk.as_bytes());
            let _ = stdout.flush();
        })
        .await?;
    println!();

    let g = &story.generation;
    eprintln!(
        "[rag-mobil] set {} « {} » — {} pièces distinctes, {} figurine(s) — modèle {}{}",
        story.inventory.set_number,
        story.inventory.name,
        story.inventory.parts.len(),
        story.inventory.figures().count(),
        g.model,
        g.output_tokens
            .map(|t| format!(", {t} tokens générés"))
            .unwrap_or_default(),
    );
    if g.truncated {
        eprintln!("[rag-mobil] attention : histoire tronquée (limite max_tokens atteinte)");
    }
    Ok(())
}
