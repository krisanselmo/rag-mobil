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
use rag_mobil::illustrate::{self, Illustrator};
use rag_mobil::model::{Collection, MissingPart, NamedPart, reference_key};
use rag_mobil::retriever::{
    Answered, Cached, Fallback, HttpJson, InventorySource, JsonFile, PlaymoDb, cache, playmodb,
};
use rag_mobil::saga::{Episode, Saga};
use rag_mobil::writer::plan::{find_part, part_label};
use rag_mobil::writer::{Observer, Step, WriterOptions, WritingMethod, first_prompt};
use rag_mobil::{RagError, RagPipeline, SetInventory, SetNumber, html, pdf};

/// Writing method.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Method {
    /// Plan, step-by-step drafting, review (Re3/DOC method).
    Planned,
    /// A single call, the story is shown as it is written.
    Direct,
    /// Story where the reader chooses what happens: numbered paragraphs and
    /// choices (same as --interactive).
    Interactive,
}

impl From<Method> for WritingMethod {
    fn from(method: Method) -> Self {
        match method {
            Method::Planned => Self::Planned,
            Method::Direct => Self::Direct,
            Method::Interactive => Self::Interactive,
        }
    }
}

/// Format of the story on standard output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Markdown, shown as it is written with --method direct.
    Markdown,
    /// Standalone HTML page, with pictures of the pieces (to read or print).
    Html,
    /// The HTML booklet printed to PDF (A4) by Chrome, Chromium or Edge.
    Pdf,
}

/// Layout of the printed booklet (HTML and PDF).
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Layout {
    /// Booklet: full-page cover, one page per scene.
    Booklet,
    /// Compact: short header, two-column text, small pictures.
    Compact,
}

impl From<Layout> for html::Layout {
    fn from(layout: Layout) -> Self {
        match layout {
            Layout::Booklet => Self::Booklet,
            Layout::Compact => Self::Compact,
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

/// Writes a Playmobil story faithful to what's in the box (inventory + LLM).
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Playmobil set numbers (e.g. 3666). With several sets, a single story
    /// mixes the contents of all the boxes.
    #[arg(required = true, value_name = "SET")]
    set_numbers: Vec<String>,

    /// Language of the story.
    #[arg(short, long, env = "RAG_MOBIL_LANGUAGE", default_value = "English")]
    language: String,

    /// Age of the reader.
    #[arg(short, long, env = "RAG_MOBIL_AGE", default_value_t = 7)]
    age: u8,

    /// Target length, in words.
    #[arg(short, long, env = "RAG_MOBIL_WORDS", default_value_t = 600)]
    words: u32,

    /// Free-form wish (e.g. "a ghost story").
    #[arg(short, long)]
    theme: Option<String>,

    /// Cites each named piece by its reference, right after its name:
    /// [ref. 30 06 368], or [ref. 30 06 198, 30 06 199] for an object made of
    /// several pieces. For a display that strips them or turns them into links.
    #[arg(long, env = "RAG_MOBIL_CITATIONS", value_parser = BoolishValueParser::new())]
    citations: bool,

    /// Writing method.
    #[arg(long, value_enum, default_value_t = Method::Planned)]
    method: Method,

    /// Choose-your-own-adventure story: numbered paragraphs, each ending
    /// with choices that lead to what comes next, and several endings
    /// (shorthand for --method interactive).
    #[arg(long, conflicts_with = "method")]
    interactive: bool,

    /// Saga: JSON file of the episodes already told (created if missing).
    /// The story continues them, with the same names for the same figures,
    /// and is added to it as a new episode.
    #[arg(long, env = "RAG_MOBIL_SAGA", value_name = "FILE")]
    saga: Option<PathBuf>,

    /// Planned method: before each part, a "Set the scene" section lists the
    /// characters and objects to lay out, from the validated plan.
    #[arg(long, env = "RAG_MOBIL_SCENE_SETUP", value_parser = BoolishValueParser::new())]
    scene_setup: bool,

    /// Output format; pdf is written to a file (> booklet.pdf).
    #[arg(short, long, value_enum, env = "RAG_MOBIL_FORMAT", default_value_t = Format::Markdown)]
    format: Format,

    /// --format html or pdf: print layout.
    #[arg(long, value_enum, env = "RAG_MOBIL_LAYOUT", default_value_t = Layout::Booklet)]
    layout: Layout,

    /// Also writes the booklet to FILE: as PDF if its name ends in .pdf
    /// (same browser as --format pdf), as HTML otherwise. Standard output
    /// stays that of --format: the story in Markdown and its booklet, from a
    /// single LLM run.
    #[arg(long, value_name = "FILE", alias = "livret")]
    booklet: Option<PathBuf>,

    /// --format html or pdf, or --booklet: an image model draws a cover
    /// picture of the characters in their setting, from the plan (one more
    /// call, after the story).
    #[arg(long, env = "RAG_MOBIL_ILLUSTRATE", value_parser = BoolishValueParser::new())]
    illustrate: bool,

    /// Image model for --illustrate; the provider is inferred from the name:
    /// gpt-image-… or dall-e-… (OpenAI, OPENAI_API_KEY), gemini-…
    /// (e.g. gemini-2.5-flash-image, GEMINI_API_KEY), or a local model:
    /// openai::<model> (an OpenAI-compatible server at
    /// RAG_MOBIL_OPENAI_IMAGE_BASE_URL), sd::<checkpoint> or sd (AUTOMATIC1111,
    /// Forge or SD.Next at RAG_MOBIL_SD_BASE_URL, default
    /// http://127.0.0.1:7860). With --replay, the
    /// picture is read from DIR/illustration.{png,jpg,webp,svg}.
    #[arg(long, env = "RAG_MOBIL_IMAGE_MODEL", default_value = illustrate::DEFAULT_IMAGE_MODEL)]
    image_model: String,

    /// --format pdf or --booklet … .pdf: argument passed to the browser (e.g.
    /// --no-sandbox in a container running as root). Repeatable. The browser
    /// is chosen with RAG_MOBIL_BROWSER.
    #[arg(long, value_name = "ARG", allow_hyphen_values = true)]
    browser_arg: Vec<String>,

    /// Piece missing from the box (lost, broken), by its reference: every
    /// copy (30 07 640) or N of them (30 07 640:2). Repeatable.
    #[arg(long, value_name = "REF[:N]", value_parser = MissingPart::parse)]
    missing: Vec<MissingPart>,

    /// The child's favorite piece, by its reference: a figure becomes the
    /// main character, an object is used in several steps. The plan is
    /// checked accordingly. Repeatable.
    #[arg(long, value_name = "REF")]
    star: Vec<String>,

    /// File of missing pieces: one per line, in the --missing format; blank
    /// lines and comments (#) are ignored.
    #[arg(long, env = "RAG_MOBIL_MISSING_FILE", value_name = "FILE")]
    missing_file: Option<PathBuf>,

    /// The name the child gave a figure or piece (30 06 368=Coco): every
    /// story calls it that. Repeatable; wins over --nickname-file and the
    /// inventory.
    #[arg(long, value_name = "REF=NAME", value_parser = NamedPart::parse)]
    nickname: Vec<NamedPart>,

    /// File of nicknames: one per line, in the --nickname format; blank
    /// lines and comments (#) are ignored. Pieces not in the boxes are
    /// skipped, so one file can serve every box.
    #[arg(long, env = "RAG_MOBIL_NICKNAME_FILE", value_name = "FILE")]
    nickname_file: Option<PathBuf>,

    /// Planned and interactive methods: no faithfulness review at the end
    /// (same as --reviews 0).
    #[arg(long)]
    no_review: bool,

    /// Planned and interactive methods: rounds of faithfulness review, each
    /// followed by a rewrite; they stop at the first review that finds
    /// nothing.
    #[arg(
        long,
        env = "RAG_MOBIL_REVIEWS",
        default_value_t = 1,
        value_parser = clap::value_parser!(u8).range(0..=3)
    )]
    reviews: u8,

    /// Keeps every category of pieces (including paper instructions).
    #[arg(long)]
    include_all: bool,

    /// LLM model; the provider is inferred from the name: claude-… (Anthropic),
    /// gpt-… (OpenAI), gemini-… (Gemini), ollama::<model> (local, free)…
    /// With RAG_MOBIL_OPENAI_BASE_URL, openai::<model> targets that
    /// OpenAI-compatible server (llama.cpp, vLLM…) instead of api.openai.com.
    #[arg(short, long, env = "RAG_MOBIL_MODEL", default_value = llm::DEFAULT_MODEL)]
    model: String,

    /// Effort / thinking level (default: medium for Claude, the provider's
    /// otherwise).
    #[arg(long, value_enum)]
    effort: Option<Effort>,

    /// Model retried once if Claude declines the request.
    #[arg(long, default_value = llm::DEFAULT_FALLBACK_MODEL, conflicts_with = "no_fallback")]
    fallback_model: String,

    /// No retry after a refusal.
    #[arg(long)]
    no_fallback: bool,

    /// PlaymoDB base URL.
    #[arg(long, env = "PLAYMODB_BASE_URL", default_value = playmodb::DEFAULT_BASE_URL)]
    playmodb_url: String,

    /// Prints the fetched inventory as JSON and stops (no LLM call).
    #[arg(long, conflicts_with = "dry_run")]
    inventory_json: bool,

    /// Prints the augmented prompt and stops (no LLM call).
    #[arg(long)]
    dry_run: bool,

    /// Reads the inventories from a JSON file (output of --inventory-json)
    /// instead of querying PlaymoDB.
    #[arg(long, value_name = "FILE")]
    inventory_file: Option<PathBuf>,

    /// HTTP inventory source in the v1 format (docs/inventory-format.md);
    /// {set} is replaced by the set number, e.g.
    /// http://localhost:8096/api/inventories/{set}. A set it doesn't know
    /// (HTTP 404) is asked to PlaymoDB. Incompatible with --inventory-file,
    /// which wins over the environment variable.
    #[arg(long, env = "RAG_MOBIL_INVENTORY_URL", value_name = "TEMPLATE")]
    inventory_url: Option<String>,

    /// With --inventory-url: no fallback to PlaymoDB, a set unknown to the
    /// source is an error.
    #[arg(long)]
    no_playmodb: bool,

    /// Cache directory of the PlaymoDB inventories [default:
    /// ~/.cache/rag-mobil/playmodb, or the system's equivalent].
    #[arg(long, env = "RAG_MOBIL_CACHE_DIR", value_name = "DIR")]
    cache_dir: Option<PathBuf>,

    /// Doesn't use the cache: PlaymoDB is queried every time.
    #[arg(long, conflicts_with = "refresh")]
    no_cache: bool,

    /// Queries PlaymoDB even if the cache is fresh, and updates the cache.
    #[arg(long)]
    refresh: bool,

    /// Bypasses the LLM: each prompt is written to DIR/NN-*.prompt.txt and
    /// the answer read from DIR/NN-*.answer.txt; run again after writing the
    /// missing answer.
    #[arg(long, value_name = "DIR")]
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
            Self::File(source) => (source.fetch(set).await?, "file".into()),
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
            eprintln!("Error: {err:#}");
            if let Some(hint) = err.downcast_ref::<RagError>().and_then(hint) {
                eprintln!("Hint: {hint}");
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
                    "--inventory-file and --inventory-url are incompatible",
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
            "set {env_name}, pick a free local model \
             (e.g. -m ollama::llama3.2) or use --dry-run"
        )),
        _ => None,
    }
}

/// `--missing-file`, `--nickname-file`: one entry per line, `#` starting a
/// comment.
fn read_list_file<T>(
    path: &PathBuf,
    parse: impl Fn(&str) -> rag_mobil::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let text = std::fs::read_to_string(path).map_err(|source| RagError::Io {
        path: path.clone(),
        source,
    })?;
    let mut entries = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if !line.is_empty() {
            let entry = parse(line)
                .map_err(|err| anyhow::anyhow!("{}, line {}: {err}", path.display(), n + 1))?;
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// One nickname per piece, the last given winning (the command line after
/// the file); two pieces can't share a name, or the story couldn't tell
/// them apart.
fn merge_nicknames(names: Vec<NamedPart>) -> anyhow::Result<Vec<NamedPart>> {
    let mut merged: Vec<NamedPart> = Vec::with_capacity(names.len());
    for named in names {
        let key = reference_key(&named.reference);
        merged.retain(|m| reference_key(&m.reference) != key);
        merged.push(named);
    }
    for (i, a) in merged.iter().enumerate() {
        if let Some(b) = merged[i + 1..]
            .iter()
            .find(|b| a.nickname.to_lowercase() == b.nickname.to_lowercase())
        {
            anyhow::bail!(
                "--nickname: \"{}\" is given to both {} and {}",
                a.nickname,
                a.reference,
                b.reference
            );
        }
    }
    Ok(merged)
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
            anyhow::bail!("--no-playmodb needs another source: --inventory-url")
        }
        (None, None) => Source::Playmo(playmo()?),
    };
    let method = if cli.interactive {
        WritingMethod::Interactive
    } else {
        cli.method.into()
    };
    let saga = match &cli.saga {
        Some(path) if method == WritingMethod::Direct => {
            eprintln!(
                "[rag-mobil] --saga ignored: it needs the planned or interactive method \
                 ({} not read)",
                path.display()
            );
            Saga::default()
        }
        Some(path) => Saga::load(path)?,
        None => Saga::default(),
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
        saga,
        stars: cli.star,
    };

    let mut missing = match &cli.missing_file {
        Some(path) => read_list_file(path, MissingPart::parse)?,
        None => Vec::new(),
    };
    missing.extend(cli.missing);
    let mut nicknames = match &cli.nickname_file {
        Some(path) => read_list_file(path, NamedPart::parse)?,
        None => Vec::new(),
    };
    nicknames.extend(cli.nickname);
    let nicknames = merge_nicknames(nicknames)?;
    let (mut collection, origins) = source.fetch_all(&sets).await?;
    let offline = cli.inventory_json || cli.dry_run;
    if !offline {
        eprintln!("[rag-mobil] inventories: {origins}");
    }
    if !missing.is_empty() {
        eprintln!("[rag-mobil] pieces missing from the box:");
        for line in collection.remove_missing(&missing) {
            eprintln!("  - {line}");
        }
    }
    if !nicknames.is_empty() {
        eprintln!("[rag-mobil] nicknames:");
        for line in collection.name_parts(&nicknames) {
            eprintln!("  - {line}");
        }
    }

    // Before any LLM call: a favorite part must be in the boxes.
    for star in &options.stars {
        match find_part(&collection, star) {
            Some(part) => eprintln!(
                "[rag-mobil] favorite piece: {} ({})",
                part_label(part),
                part.number
            ),
            None => anyhow::bail!(
                "--star {star}: this piece is not in {}",
                if collection.is_single() {
                    "the box"
                } else {
                    "the boxes"
                }
            ),
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
            let prompt = first_prompt(&collection, &options, method);
            println!(
                "=== SYSTEM ===\n{}\n\n=== USER ===\n{}",
                prompt.system, prompt.user
            );
        }
        return Ok(());
    }

    if cli.scene_setup && method == WritingMethod::Direct {
        eprintln!("[rag-mobil] --scene-setup ignored: it needs the planned method");
    }
    let saga = cli.saga.filter(|_| method != WritingMethod::Direct);
    if saga.is_some() {
        eprintln!("[rag-mobil] saga: episode {}", options.saga.next_number());
    }
    let writer = WriterOptions {
        method,
        reviews: if cli.no_review { 0 } else { cli.reviews },
        scene_setup: cli.scene_setup,
    };
    // Before any LLM call: a PDF needs a browser and a file.
    if cli.format == Format::Pdf {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() {
            anyhow::bail!("--format pdf writes a file: redirect the output (> booklet.pdf)");
        }
    }
    let booklet_pdf = cli
        .booklet
        .as_deref()
        .and_then(|p| p.extension())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    let printer = if cli.format == Format::Pdf || booklet_pdf {
        Some((pdf::find_browser()?, cli.browser_arg))
    } else {
        None
    };
    // Before any LLM call: a missing image key fails now, not after the story.
    let illustrator = match (&cli.replay, cli.illustrate) {
        (_, false) => None,
        _ if cli.format == Format::Markdown && cli.booklet.is_none() => {
            eprintln!("[rag-mobil] --illustrate ignored: it needs --format html/pdf or --booklet");
            None
        }
        (Some(dir), true) => Some(Illustrator::Replay(dir.clone())),
        (None, true) => Some(Illustrator::new(&cli.image_model)?),
    };
    let telling = Telling {
        illustrator,
        collection,
        sets: &sets,
        options: &options,
        writer,
        format: cli.format,
        layout: cli.layout.into(),
        printer,
        booklet: cli.booklet.map(|path| (path, booklet_pdf)),
        saga,
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
    layout: html::Layout,
    /// `--format pdf` or a PDF `--booklet`: the browser and its extra arguments.
    printer: Option<(PathBuf, Vec<String>)>,
    /// `--booklet`: where to write the booklet, and whether as a PDF.
    booklet: Option<(PathBuf, bool)>,
    /// `--saga`: where to add the new episode.
    saga: Option<PathBuf>,
    /// `--illustrate`: who draws the cover.
    illustrator: Option<Illustrator>,
}

/// The booklet printed to PDF by the browser found before writing.
fn print_pdf(printer: Option<&(PathBuf, Vec<String>)>, page: &str) -> anyhow::Result<Vec<u8>> {
    let (browser, args) = printer.expect("browser found before writing");
    eprintln!("[rag-mobil] printing PDF ({})…", browser.display());
    Ok(pdf::print(page, browser, args)?)
}

impl Telling<'_> {
    async fn tell<G: StoryGenerator>(self, generator: G) -> anyhow::Result<()> {
        let pipeline = RagPipeline::new(Fetched(self.collection), generator);
        let mut terminal = Terminal {
            stream: self.format == Format::Markdown,
        };
        let mut story = pipeline
            .run(self.sets, self.options, self.writer, &mut terminal)
            .await?;
        // The story is written: a picture that fails is a warning, not a lost story.
        if let Some(illustrator) = &self.illustrator {
            eprintln!("[rag-mobil] drawing the cover…");
            let prompt =
                illustrate::prompt(story.written.plan.as_ref(), &story.collection, self.options);
            match illustrator.draw(&prompt).await {
                Ok(picture) => {
                    eprintln!(
                        "[rag-mobil] cover drawn by {} ({} KB)",
                        picture.model,
                        picture.bytes.len() / 1024
                    );
                    story.illustration = Some(picture);
                }
                Err(err) => eprintln!("[rag-mobil] no cover illustration: {err}"),
            }
        }
        match self.format {
            Format::Markdown => println!(),
            Format::Html => print!("{}", html::render(&story, self.options, self.layout)),
            Format::Pdf => {
                let bytes = print_pdf(
                    self.printer.as_ref(),
                    &html::render(&story, self.options, self.layout),
                )?;
                let mut stdout = std::io::stdout();
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
        }
        // The story is already out: a booklet that fails is a warning, not a lost story.
        if let Some((path, as_pdf)) = &self.booklet {
            let page = html::render(&story, self.options, self.layout);
            let written = if *as_pdf {
                print_pdf(self.printer.as_ref(), &page)
            } else {
                Ok(page.into_bytes())
            }
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|e| anyhow::anyhow!("{}", e)));
            match written {
                Ok(()) => eprintln!("[rag-mobil] booklet: {}", path.display()),
                Err(err) => eprintln!(
                    "[rag-mobil] booklet not written ({}): {err:#}",
                    path.display()
                ),
            }
        }
        summary(&story);
        if let Some(path) = &self.saga {
            match &story.written.plan {
                Some(plan) => {
                    let mut saga = self.options.saga.clone();
                    let linear = story.written.paragraphs.is_empty();
                    saga.episodes
                        .push(Episode::new(&story.collection, plan, linear));
                    saga.save(path)?;
                    eprintln!(
                        "[rag-mobil] episode {} added to {}",
                        saga.episodes.len(),
                        path.display()
                    );
                }
                None => eprintln!(
                    "[rag-mobil] no plan (written in a single call): episode not \
                     added to {}",
                    path.display()
                ),
            }
        }
        Ok(())
    }
}

/// `, ~12000 input tokens (8000 read from cache), ~1500 generated`, or what
/// the provider reported of it.
fn tokens(usage: &rag_mobil::generator::TokenUsage) -> String {
    let mut out = String::new();
    if let Some(input) = usage.input {
        out.push_str(&format!(", ~{input} input tokens"));
        if let Some(read) = usage.cache_read {
            out.push_str(&format!(" ({read} read from cache)"));
        }
    }
    if let Some(output) = usage.output {
        out.push_str(&format!(", ~{output} generated"));
    }
    out
}

fn summary(story: &rag_mobil::Story) {
    let g = &story.written.generation;
    eprintln!(
        "[rag-mobil] {} — {} distinct pieces, {} figure(s) — model {}, {} call(s){}",
        story.collection.label(),
        story.collection.part_count(),
        story.collection.figures().count(),
        g.model,
        story.written.calls,
        tokens(&g.usage),
    );
    if g.truncated {
        eprintln!("[rag-mobil] warning: story truncated (max_tokens limit reached)");
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
            Step::Planning => eprintln!("[rag-mobil] planning the story…"),
            Step::PlanRejected(errors) => eprintln!(
                "[rag-mobil] plan not faithful to the box ({} issue(s)), asking again…",
                errors.len()
            ),
            Step::PlanRepaired(errors) => {
                eprintln!("[rag-mobil] invalid references removed from the plan:");
                for error in errors {
                    eprintln!("  - {error}");
                }
            }
            Step::PlanAbandoned(reason) => {
                eprintln!("[rag-mobil] unusable plan ({reason}): writing in a single call…")
            }
            Step::PlanReady(plan) => eprintln!(
                "[rag-mobil] plan: \"{}\" — {} character(s), {} step(s)",
                plan.title,
                plan.characters.len(),
                plan.outline.len()
            ),
            Step::Drafting { part, total } => {
                eprintln!("[rag-mobil] drafting {part}/{total}…");
            }
            Step::Reviewing => eprintln!("[rag-mobil] faithfulness review…"),
            Step::ReviewUnreadable => {
                eprintln!("[rag-mobil] unreadable review, story kept uncorrected");
            }
            Step::Revising(issues) => {
                eprintln!("[rag-mobil] {} issue(s) fixed:", issues.len());
                for issue in issues {
                    eprintln!("  - {}", issue.problem);
                }
            }
            Step::SceneSetupDropped => eprintln!(
                "[rag-mobil] scene setup dropped: the revision didn't keep the parts \
                 separate"
            ),
            Step::CitationsDropped(references) => eprintln!(
                "[rag-mobil] references not in the box removed from the story: {}",
                references.join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(raw: &str) -> NamedPart {
        NamedPart::parse(raw).unwrap()
    }

    #[test]
    fn the_last_nickname_of_a_piece_wins() {
        let merged = merge_nicknames(vec![
            named("30 06 368=Polly"),
            named("30 07 640=Pike"),
            named("3006368=Coco"),
        ])
        .unwrap();
        assert_eq!(merged, [named("30 07 640=Pike"), named("3006368=Coco")]);
    }

    #[test]
    fn two_pieces_cannot_share_a_nickname() {
        let err = merge_nicknames(vec![named("30 06 368=Coco"), named("30 07 640=coco")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("30 06 368 and 30 07 640"), "{err}");
    }
}
