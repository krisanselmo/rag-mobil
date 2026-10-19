use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::builder::BoolishValueParser;
use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};

use rag_mobil::augment::{ContextOptions, StoryOptions};
use rag_mobil::generator::{
    LlmConfig, LlmGenerator, ReasoningEffort, ReplayGenerator, StoryGenerator, llm,
};
use rag_mobil::model::{Collection, MissingPart};
use rag_mobil::retriever::{
    Answered, Cached, Fallback, HttpJson, InventorySource, JsonFile, PlaymoDb, cache, playmodb,
};
use rag_mobil::writer::{Observer, Step, WriterOptions, WritingMethod, first_prompt};
use rag_mobil::{RagError, RagPipeline, SetInventory, SetNumber, html};

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

/// Format de l'histoire sur la sortie standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Markdown, affiché au fil de l'écriture avec --method direct.
    Markdown,
    /// Page HTML autonome, avec les images des pièces (à lire ou imprimer).
    Html,
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

/// Génère une histoire Playmobil fidèle au contenu d'une boîte (inventaire + LLM).
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Numéros des sets Playmobil (ex. 3666). Avec plusieurs sets, une seule
    /// histoire mélange le contenu de toutes les boîtes.
    #[arg(required = true, value_name = "SET")]
    set_numbers: Vec<String>,

    /// Langue de l'histoire.
    #[arg(short, long, env = "RAG_MOBIL_LANGUAGE", default_value = "français")]
    language: String,

    /// Âge du lecteur.
    #[arg(short, long, env = "RAG_MOBIL_AGE", default_value_t = 7)]
    age: u8,

    /// Longueur visée, en mots.
    #[arg(short, long, env = "RAG_MOBIL_WORDS", default_value_t = 600)]
    words: u32,

    /// Souhait libre (ex. "une histoire de fantômes").
    #[arg(short, long)]
    theme: Option<String>,

    /// Cite chaque pièce nommée par sa référence, juste après son nom :
    /// [réf. 30 06 368], ou [réf. 30 06 198, 30 06 199] pour un objet en
    /// plusieurs pièces. Pour un affichage qui les retire ou en fait des liens.
    #[arg(long, env = "RAG_MOBIL_CITATIONS", value_parser = BoolishValueParser::new())]
    citations: bool,

    /// Méthode de rédaction.
    #[arg(long, value_enum, default_value_t = Method::Planned)]
    method: Method,

    /// Méthode planned : avant chaque partie, une « Mise en place » liste les
    /// personnages et objets à installer, d'après le plan validé.
    #[arg(long, env = "RAG_MOBIL_SCENE_SETUP", value_parser = BoolishValueParser::new())]
    scene_setup: bool,

    /// Format de sortie.
    #[arg(short, long, value_enum, env = "RAG_MOBIL_FORMAT", default_value_t = Format::Markdown)]
    format: Format,

    /// Pièce absente de la boîte (perdue, cassée), par sa référence : toutes
    /// les copies (30 07 640) ou N d'entre elles (30 07 640:2). Répétable.
    #[arg(long, value_name = "REF[:N]", value_parser = MissingPart::parse)]
    missing: Vec<MissingPart>,

    /// Fichier des pièces absentes : une par ligne, au format de --missing ;
    /// lignes vides et commentaires (#) ignorés.
    #[arg(long, env = "RAG_MOBIL_MISSING_FILE", value_name = "FICHIER")]
    missing_file: Option<PathBuf>,

    /// Méthode planned : pas de relecture de fidélité à la fin.
    #[arg(long)]
    no_review: bool,

    /// Garde toutes les catégories de pièces (y compris les notices papier).
    #[arg(long)]
    include_all: bool,

    /// Modèle LLM ; le fournisseur est déduit du nom : claude-… (Anthropic),
    /// gpt-… (OpenAI), gemini-… (Gemini), ollama::<modèle> (local, gratuit)…
    /// Avec RAG_MOBIL_OPENAI_BASE_URL, openai::<modèle> vise ce serveur compatible
    /// OpenAI (llama.cpp, vLLM…) au lieu d'api.openai.com.
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

    /// Lit les inventaires dans un fichier JSON (sortie de --inventory-json)
    /// au lieu d'interroger PlaymoDB.
    #[arg(long, value_name = "FICHIER")]
    inventory_file: Option<PathBuf>,

    /// Source d'inventaires HTTP au format v1 (docs/format-inventaire.md) ;
    /// {set} est remplacé par le numéro du set, ex.
    /// http://localhost:8096/api/inventories/{set}. Un set qu'elle ne connaît
    /// pas (HTTP 404) est demandé à PlaymoDB. Incompatible avec
    /// --inventory-file, qui l'emporte sur la variable d'environnement.
    #[arg(long, env = "RAG_MOBIL_INVENTORY_URL", value_name = "MODELE")]
    inventory_url: Option<String>,

    /// Avec --inventory-url : pas de repli sur PlaymoDB, un set inconnu de la
    /// source est une erreur.
    #[arg(long)]
    no_playmodb: bool,

    /// Dossier du cache des inventaires PlaymoDB [défaut :
    /// ~/.cache/rag-mobil/playmodb, ou l'équivalent du système].
    #[arg(long, env = "RAG_MOBIL_CACHE_DIR", value_name = "DOSSIER")]
    cache_dir: Option<PathBuf>,

    /// N'utilise pas le cache : PlaymoDB est interrogé à chaque fois.
    #[arg(long, conflicts_with = "refresh")]
    no_cache: bool,

    /// Interroge PlaymoDB même si le cache est à jour, et met le cache à jour.
    #[arg(long)]
    refresh: bool,

    /// Contourne le LLM : chaque prompt est écrit dans DOSSIER/NN-*.prompt.txt
    /// et la réponse lue dans DOSSIER/NN-*.answer.txt ; relancez après avoir
    /// écrit la réponse manquante.
    #[arg(long, value_name = "DOSSIER")]
    replay: Option<PathBuf>,
}

/// PlaymoDB, behind the on-disk cache or not.
enum Playmo {
    Web(PlaymoDb),
    Cached(Cached<PlaymoDb>),
}

impl Playmo {
    fn origin(&self, set: &SetNumber) -> String {
        match self {
            Self::Cached(cache) if cache.cached(set).is_some() => "cache".into(),
            _ => "PlaymoDB".into(),
        }
    }
}

impl InventorySource for Playmo {
    async fn fetch(&self, set: &SetNumber) -> rag_mobil::Result<SetInventory> {
        match self {
            Self::Web(source) => source.fetch(set).await,
            Self::Cached(source) => source.fetch(set).await,
        }
    }
}

enum Source {
    Playmo(Playmo),
    File(JsonFile),
    Http(HttpJson),
    HttpThenPlaymo(Fallback<HttpJson, Playmo>),
}

impl Source {
    /// The inventory and where it came from (`cache`, `PlaymoDB`, a host...).
    async fn fetch_traced(&self, set: &SetNumber) -> rag_mobil::Result<(SetInventory, String)> {
        Ok(match self {
            Self::Playmo(source) => {
                let origin = source.origin(set);
                (source.fetch(set).await?, origin)
            }
            Self::File(source) => (source.fetch(set).await?, "fichier".into()),
            Self::Http(source) => (source.fetch(set).await?, source.host()),
            Self::HttpThenPlaymo(chain) => {
                // Before fetching: afterwards the set is always in the cache.
                let playmo_origin = chain.second().origin(set);
                match chain.fetch_traced(set).await? {
                    (inventory, Answered::First) => (inventory, chain.first().host()),
                    (inventory, Answered::Second) => (inventory, playmo_origin),
                }
            }
        })
    }

    /// Every set once, in order, with a summary such as
    /// `3666 (localhost:8096), 5135 (cache)`.
    async fn fetch_all(&self, sets: &[SetNumber]) -> rag_mobil::Result<(Collection, String)> {
        let mut inventories = Vec::with_capacity(sets.len());
        let mut origins = Vec::with_capacity(sets.len());
        for set in sets {
            let (inventory, origin) = self.fetch_traced(set).await?;
            inventories.push(inventory);
            origins.push(format!("{set} ({origin})"));
        }
        Ok((Collection::new(inventories), origins.join(", ")))
    }
}

/// Inventories fetched up front, so the progress line can say where each one
/// actually came from (with a fallback, that is only known after the fetch).
struct Fetched(Collection);

impl InventorySource for Fetched {
    async fn fetch(&self, set: &SetNumber) -> rag_mobil::Result<SetInventory> {
        self.0
            .sets
            .iter()
            .find(|inventory| &inventory.set_number == set)
            .cloned()
            .ok_or_else(|| RagError::SetNotFound(set.to_string()))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(parse_cli()).await {
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

/// Like `Cli::parse()`, but `--inventory-file` only conflicts with an
/// `--inventory-url` typed on the command line, not with the environment
/// variable (clap's `conflicts_with` counts both).
fn parse_cli() -> Cli {
    let matches = Cli::command().get_matches();
    let mut cli = Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    if cli.inventory_file.is_some() && cli.inventory_url.is_some() {
        if matches.value_source("inventory_url") == Some(ValueSource::CommandLine) {
            Cli::command()
                .error(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--inventory-file et --inventory-url sont incompatibles",
                )
                .exit();
        }
        cli.inventory_url = None;
    }
    cli
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

/// `--missing-file`: one missing part per line, `#` starting a comment.
fn read_missing_file(path: &PathBuf) -> anyhow::Result<Vec<MissingPart>> {
    let text = std::fs::read_to_string(path).map_err(|source| RagError::Io {
        path: path.clone(),
        source,
    })?;
    let mut missing = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if !line.is_empty() {
            let part = MissingPart::parse(line)
                .map_err(|err| anyhow::anyhow!("{}, ligne {} : {err}", path.display(), n + 1))?;
            missing.push(part);
        }
    }
    Ok(missing)
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let mut sets: Vec<SetNumber> = Vec::new();
    for raw in &cli.set_numbers {
        let set = SetNumber::parse(raw)?;
        if !sets.contains(&set) {
            sets.push(set);
        }
    }
    let playmo = || -> anyhow::Result<Playmo> {
        let scraper = PlaymoDb::new(&cli.playmodb_url)?;
        if cli.no_cache {
            return Ok(Playmo::Web(scraper));
        }
        let dir = cli
            .cache_dir
            .clone()
            .unwrap_or_else(|| cache::default_cache_dir().join("playmodb"));
        Ok(Playmo::Cached(
            Cached::new(scraper, dir).refresh(cli.refresh),
        ))
    };
    let source = match (&cli.inventory_file, &cli.inventory_url) {
        (Some(path), _) => Source::File(JsonFile(path.clone())),
        (None, Some(template)) if cli.no_playmodb => Source::Http(HttpJson::new(template)?),
        (None, Some(template)) => {
            Source::HttpThenPlaymo(Fallback::new(HttpJson::new(template)?, playmo()?))
        }
        (None, None) if cli.no_playmodb => {
            anyhow::bail!("--no-playmodb demande une autre source : --inventory-url")
        }
        (None, None) => Source::Playmo(playmo()?),
    };
    let options = StoryOptions {
        language: cli.language,
        reader_age: cli.age,
        target_words: cli.words,
        extra_instructions: cli.theme,
        citations: cli.citations,
        context: ContextOptions {
            include_all: cli.include_all,
        },
    };

    let mut missing = match &cli.missing_file {
        Some(path) => read_missing_file(path)?,
        None => Vec::new(),
    };
    missing.extend(cli.missing);
    let (mut collection, origins) = source.fetch_all(&sets).await?;
    let offline = cli.inventory_json || cli.dry_run;
    if !offline {
        eprintln!("[rag-mobil] inventaires : {origins}");
    }
    if !missing.is_empty() {
        eprintln!("[rag-mobil] pièces absentes de la boîte :");
        for line in collection.remove_missing(&missing) {
            eprintln!("  - {line}");
        }
    }

    // Offline modes: retrieval (+ augmentation) only.
    if offline {
        if cli.inventory_json {
            // One set: the inventory itself; several: an array.
            let json = match collection.sets.as_slice() {
                [one] => serde_json::to_string_pretty(one)?,
                many => serde_json::to_string_pretty(many)?,
            };
            println!("{json}");
        } else {
            let prompt = first_prompt(&collection, &options, cli.method.into());
            println!(
                "=== SYSTEM ===\n{}\n\n=== USER ===\n{}",
                prompt.system, prompt.user
            );
        }
        return Ok(());
    }

    if cli.scene_setup && matches!(cli.method, Method::Direct) {
        eprintln!("[rag-mobil] --scene-setup ignoré : il demande la méthode planned");
    }
    let writer = WriterOptions {
        method: cli.method.into(),
        review: !cli.no_review,
        scene_setup: cli.scene_setup,
    };
    let telling = Telling {
        collection,
        sets: &sets,
        options: &options,
        writer,
        format: cli.format,
    };
    match cli.replay {
        Some(dir) => telling.tell(ReplayGenerator::new(dir)).await,
        None => {
            let generator = LlmGenerator::new(LlmConfig {
                model: cli.model,
                effort: cli.effort.map(Into::into),
                fallback_model: (!cli.no_fallback).then_some(cli.fallback_model),
            });
            telling.tell(generator).await
        }
    }
}

/// What to tell, once the inventories are fetched.
struct Telling<'a> {
    collection: Collection,
    sets: &'a [SetNumber],
    options: &'a StoryOptions,
    writer: WriterOptions,
    format: Format,
}

impl Telling<'_> {
    async fn tell<G: StoryGenerator>(self, generator: G) -> anyhow::Result<()> {
        let pipeline = RagPipeline::new(Fetched(self.collection), generator);
        let mut terminal = Terminal {
            stream: self.format == Format::Markdown,
        };
        let story = pipeline
            .run(self.sets, self.options, self.writer, &mut terminal)
            .await?;
        match self.format {
            Format::Markdown => println!(),
            Format::Html => print!("{}", html::render(&story, self.options)),
        }
        summary(&story);
        Ok(())
    }
}

fn summary(story: &rag_mobil::Story) {
    let g = &story.written.generation;
    eprintln!(
        "[rag-mobil] {} — {} pièces distinctes, {} figurine(s) — modèle {}, {} appel(s){}",
        story.collection.label(),
        story.collection.part_count(),
        story.collection.figures().count(),
        g.model,
        story.written.calls,
        g.output_tokens
            .map(|t| format!(", ~{t} tokens générés"))
            .unwrap_or_default(),
    );
    if g.truncated {
        eprintln!("[rag-mobil] attention : histoire tronquée (limite max_tokens atteinte)");
    }
}

/// Story on stdout (unless rendered at the end), progress on stderr.
struct Terminal {
    stream: bool,
}

impl Observer for Terminal {
    fn text(&mut self, chunk: &str) {
        if !self.stream {
            return;
        }
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
            Step::PlanAbandoned(reason) => {
                eprintln!("[rag-mobil] plan inutilisable ({reason}) : rédaction en un seul appel…")
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
            Step::SceneSetupDropped => eprintln!(
                "[rag-mobil] mise en place retirée : la correction n'a pas gardé les parties \
                 séparées"
            ),
        }
    }
}
