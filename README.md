# rag-mobil 🏰

English · [Français](README.fr.md) · [Deutsch](README.de.md)

[![CI](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml/badge.svg)](https://github.com/krisanselmo/rag-mobil/actions/workflows/ci.yml)

A Playmobil story generator built on **RAG** (Retrieval-Augmented Generation): given one or
more set numbers, `rag-mobil` fetches the exact inventory of the boxes from
[PlaymoDB](https://playmodb.org) (or from [another database](#inventory-sources)), then asks
an LLM (Claude by default, or OpenAI, Gemini, Groq, a local model through Ollama…) to write a
story a child can play out **with what is really in their box**: the same figures, the same
objects, the same quantities.

```
set number ──► Retrieve ──► Augment ──► Writing ───────────────────► story
               PlaymoDB     context     Plan → Draft → Edit
               or another   + prompt    (Re3/DOC), through an LLM (genai)
               source
```

## Example

[**Pepper and the Pirate's Hat**](examples/3799-pirates-secret-island/): a story for set 3799
"Pirates Secret Island", written by Claude, with its
[illustrated booklet (PDF)](examples/3799-pirates-secret-island/booklet.pdf) and its cover picture. Captain Salty
rows to the island for the treasure behind the secret door; Pepper the parrot steals his hat.
Every figure and object of the story is in the box.

## Installation

```bash
cargo build --release
```

Then, depending on the provider:

| Model (`-m`) | Provider | Requirements |
|---|---|---|
| `claude-opus-5-5` (default), `claude-…` | Anthropic | `ANTHROPIC_API_KEY` |
| `gpt-…` | OpenAI | `OPENAI_API_KEY` |
| `gemini-…` | Google Gemini | `GEMINI_API_KEY` |
| `ollama::<model>` (e.g. `ollama::llama3.2`) | [Ollama](https://ollama.com), local and free | `ollama pull <model>`, no key |
| `openai::<model>` + `RAG_MOBIL_OPENAI_BASE_URL` | any OpenAI-compatible server, local or remote (llama.cpp, vLLM, LM Studio…) | the server's URL; no key if `OPENAI_API_KEY` is unset |

More providers are supported (Groq, DeepSeek, xAI, Cohere, OpenRouter, Bedrock…):
see [genai's list](https://github.com/jeremychone/rust-genai).

## Usage

```bash
# A story in English for a 7-year-old (the default)
rag-mobil 3666

# Several boxes: one story mixing their contents
rag-mobil 3799 70145

# Customized
rag-mobil 5135 --age 5 --words 400 --theme "a storm and a lost treasure"
rag-mobil 3666 --language français --effort high

# Written in a single call, streamed as it is written (faster)
rag-mobil 3666 --method direct

# Illustrated booklet, with how to set up each scene: HTML, or PDF to print
rag-mobil 3799 --scene-setup --format html > booklet.html
rag-mobil 3799 --scene-setup --format pdf > booklet.pdf
rag-mobil 3799 --scene-setup --format pdf --layout compact > booklet.pdf   # 2 pages instead of 7
rag-mobil 3799 --scene-setup --illustrate --format pdf > booklet.pdf       # with a cover picture

# Choose-your-own-adventure: numbered paragraphs, choices, several endings
rag-mobil 3799 --interactive --format pdf > adventure.pdf

# A saga: each story follows on from the previous one, with the same characters
rag-mobil 3799 --saga pirates.json        # episode 1
rag-mobil 3799 70145 --saga pirates.json  # episode 2, with a new box

# The parrot is the favorite piece: it is at the heart of the story
rag-mobil 3799 --star "30 06 368"

# The real box: the parrot is lost, and so are 2 of the 4 halberds
rag-mobil 3666 --missing "30 06 368" --missing "30 07 640:2"

# Other providers
rag-mobil 3666 -m gpt-5.4-mini
rag-mobil 3666 -m ollama::llama3.2        # local, no API key
RAG_MOBIL_OPENAI_BASE_URL=http://localhost:8080/v1 rag-mobil 3666 -m openai::ministral-3-3b  # llama.cpp, vLLM…

# No LLM call (no API key needed)
rag-mobil 3666 --inventory-json   # the fetched inventory, as JSON
rag-mobil 3666 --dry-run          # the first prompt sent to the LLM
```

The story is printed on `stdout` (as Markdown); steps and progress messages go to `stderr`
(`rag-mobil 3666 > story.md` works).

| Option | Default | Purpose |
|---|---|---|
| `-l, --language` / `RAG_MOBIL_LANGUAGE` | `English` | Language of the story (e.g. `français`, `Deutsch`) |
| `-a, --age` / `RAG_MOBIL_AGE` | `7` | Reader's age |
| `-w, --words` / `RAG_MOBIL_WORDS` | `600` | Target length |
| `-t, --theme` | — | Free-form wish |
| `--citations` / `RAG_MOBIL_CITATIONS` | off | Cite each named piece by its reference (see below) |
| `--method` | `planned` | `planned` (plan, draft, edit), `direct` (one call) or `interactive` |
| `--interactive` | off | Choose-your-own-adventure story (shorthand for `--method interactive`, see below) |
| `--saga` / `RAG_MOBIL_SAGA` | — | JSON file of episodes: the story follows on from them, then is added to it (see below) |
| `--scene-setup` / `RAG_MOBIL_SCENE_SETUP` | off | `planned` method: "Set the scene" before each part (see below) |
| `-f, --format` / `RAG_MOBIL_FORMAT` | `markdown` | `markdown`, `html` or `pdf` (illustrated booklet, see below) |
| `--booklet FILE` (alias `--livret`) | — | Also writes the booklet (HTML, or PDF for `.pdf`): the story stays Markdown on `stdout`, with a single LLM run |
| `--layout` / `RAG_MOBIL_LAYOUT` | `booklet` | `--format html` or `pdf`: `booklet` or `compact` (saves paper) |
| `--illustrate` / `RAG_MOBIL_ILLUSTRATE` | off | Booklet: a cover picture drawn by an image model (see below) |
| `--image-model` / `RAG_MOBIL_IMAGE_MODEL` | `gpt-image-1` | Image model for `--illustrate`: `gpt-image-…`, `dall-e-…` (OpenAI) or `gemini-…` (Gemini) |
| `--browser-arg` | — | `--format pdf`: argument passed to the browser (e.g. `--no-sandbox`); repeatable |
| `--star REF` | — | The child's favorite piece, at the heart of the story; repeatable (see below) |
| `--missing REF[:N]` | — | A piece missing from the box: every copy, or N; repeatable |
| `--missing-file` / `RAG_MOBIL_MISSING_FILE` | — | Missing pieces, one per line (see below) |
| `--reviews N` / `RAG_MOBIL_REVIEWS` | `1` | `planned` and `interactive` methods: rounds of faithfulness review (0 to 3), each followed by a rewrite; they stop at the first review that finds nothing |
| `--no-review` | off | Same as `--reviews 0`: skip the faithfulness review |
| `--include-all` | off | Also keep paper instructions in the context |
| `-m, --model` / `RAG_MOBIL_MODEL` | `claude-opus-5-5` | Model; the provider is inferred from the name |
| `--effort` | `medium` for Claude, provider default otherwise | `low`, `medium`, `high`, `xhigh`, `max` |
| `--fallback-model` | `claude-opus-4-8` | Model retried once if Claude declines the request |
| `--no-fallback` | off | Disables that retry |
| `--playmodb-url` / `PLAYMODB_BASE_URL` | `https://playmodb.org/cgi-bin` | PlaymoDB mirror |
| `--cache-dir` / `RAG_MOBIL_CACHE_DIR` | `~/.cache/rag-mobil/playmodb` | Inventory cache directory |
| `--refresh` | off | Query PlaymoDB even if the cache is fresh |
| `--no-cache` | off | Don't use the cache |
| `--inventory-file` | — | Read inventories from a JSON file (output of `--inventory-json`) |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | — | HTTP inventory source (`{set}` = set number), PlaymoDB as fallback |
| `--no-playmodb` | off | With `--inventory-url`: no fallback to PlaymoDB |
| `--replay` | — | Bypass the LLM: prompts and answers in files (see Development) |

Environment variables set a deployment's defaults; an option typed on the command line wins.

### Piece citations

With `--citations`, each named figure or piece is followed by its inventory reference, both in
the story and in "In your box": `[ref. 30 06 368]`, or `[ref. 30 06 198, 30 06 199]` for an
object made of several pieces. The format is stable: a front end can strip them
(`\s*\[ref\.[^\]]*\]`) or turn each reference into a link to the piece. With the `planned`
method, the "In your box" section is written by code, so its citations are always exact; in
the story text, the LLM copies them, and **code checks them** (`planned` and `interactive`
methods): a reference the boxes don't hold is added to the review's issues, for the rewrite to
fix, and if one is still there after the last rewrite (or with `--no-review`), it is removed
from the text (message on `stderr`). The story never cites a piece that isn't in the box.

### Choose-your-own-adventure

With `--interactive`, the plan is no longer a sequence of steps but a graph: 6 to 9
paragraphs, each ending with 2 or 3 choices that lead to another paragraph, and 2 to 4
endings. As with the linear plan, every character and object is a reference checked against
the box; code also checks the graph: every choice leads to an existing paragraph, no
paragraph is unreachable, there are no loops, and there are at least 2 endings. If anything is
off, the plan is requested once more with the list of errors; a graph that is still broken
makes the story be written in a single call (not interactive).

Paragraphs are numbered in reading order (breadth-first from the start), and each one is
written with the paragraphs leading to it as context. The review sees every paragraph; only
those it quotes are rewritten.

```markdown
## 1

Coco the parrot landed on Tom's hammock…

**What do you choose?**

- Pip suggests rowing around the island → **go to 2**
- Robinson wants to search the cave first → **go to 3**
```

In HTML and PDF, choices are links to the paragraphs, and a printed paragraph is never split
across two pages. `--scene-setup` adds the scene setup of each paragraph.

### Sagas

`--saga pirates.json` chains stories together: the file (created if it doesn't exist) keeps
every episode told — boxes, title, premise, characters (name, figure, portrait) and steps — and
the plan of the next episode receives them in `<previous_episodes>`, with the instruction not to
repeat the plot. Known characters whose figure is in the box are offered with their name and
personality; the others can only be mentioned.

Code checks continuity, figure by figure: a known figure keeps its name ("figure 30 13 020 was
called "Pip": keep that name"), and a name already given doesn't move to another figure. The
plan is requested once more; if discrepancies remain, the original names are restored. Boxes
can change from one episode to the next: only the figures present come back. The episode is
added to the file once the story is written, unless the plan failed (single-call writing).
`--saga` requires the `planned` or `interactive` method; an interactive story is saved without
its steps, since how it unfolds depends on the reader.

### Favorite piece

`--star REF` puts the child's favorite piece at the heart of the story: a figure becomes the
main character, an object plays a central role. The reference can be written with or without
spaces, as with `--missing`; it must be in the box (after missing pieces are removed),
otherwise the program stops before any LLM call. The option is repeatable.

With the `planned` and `interactive` methods, code checks the plan: a favorite figure must play
a character, present from the first step and in at least half of the steps; a favorite object
must be used in at least 2 steps. If anything is off, the plan is requested once more with
those errors; if some remain, code adds the piece to the first steps that lack it. A figure
that plays no character can't be added, for want of a name: the story is written without it
in the spotlight. With `--method direct`, the LLM only gets the instruction.

```bash
rag-mobil 3666 --star "30 00 496"                        # the trumpeter as the hero
rag-mobil 3799 --star 3006368 --star "30 13 020" --interactive
```

### Lost or broken pieces

The inventory describes the box as new; the real box has often lost some pieces. `--missing`
removes them from the inventory before anything else: the plan can no longer pick them, the
review flags them, and "In your box" doesn't list them.

```bash
rag-mobil 3666 --missing "30 06 368"          # every copy
rag-mobil 3666 --missing "30 07 640:2"        # 2 of the 4 halberds lost
rag-mobil 3666 --missing-file my-box.txt      # the list, kept in a file
```

```text
# my-box.txt: one piece per line, in the --missing format
30 06 368        # the parrot
30 07 640:2
```

The reference can be written with or without spaces (`3007640`, `ref. 30 07 640`). With
several boxes, copies are removed from the first box that holds them, then from the next ones.
A piece whose quantity is unknown (`?`) is only removed without `:N`. Each piece is reported on
`stderr` (removed, how many remain, or not in the boxes and ignored). `--inventory-json` shows
the inventory after removal.

### Setting the scene

With `--scene-setup` (`planned` method), each part of the story starts with what to lay out to
play it, taken from the validated plan and written **by code**, so no piece is ever made up:

```markdown
> 🧱 **Set the scene**: Pip (Pirate, red vest and ponytail); Rowboat, one-seater (dark brown) + Rowboat gear: seat, oarlocks, paddles (light brown), Lantern (silver/yellow)
```

To keep each scene setup in front of the right part, the rewrite that follows the review
receives the parts separated by `* * *` lines and must keep them. If it merges them, the
revised story is kept, without scene setups (with a message on `stderr`).

### HTML and PDF booklet

`--format html` writes a self-contained HTML page (styles included, light or dark theme)
instead of Markdown:

- a cover: title, character portraits, premise, boxes used;
- a thumbnail of each piece (the inventory's `image` field, PlaymoDB images);
- each scene's setup as illustrated cards, with `--scene-setup`;
- "In your box" as checklist cards: characters, objects, quantities, reference, box;
- with `--citations`, each `[ref. …]` in the text becomes the piece's thumbnail.

Any HTML the LLM might put in the text is shown as text, and only `http(s)` images are kept.
Images are loaded from PlaymoDB when the page is opened. With `--format html` or `pdf`, the
story is shown at the end, even with `--method direct`.

Printed (or saved as PDF from the browser), the page becomes an A4 booklet: a framed cover on
the first page, one page per scene with a compact scene setup, 14 pt justified hyphenated text,
"In your box" on a new page with a checkbox per piece to gather the pieces before playing,
and page numbers (except on the cover).

`--layout compact` keeps the same content but saves paper: a short header instead of the
cover, the story in two columns without page breaks, and scene setups and "In your box" with
small thumbnails, right after the story. For set 3799 with `--scene-setup`, that's 2 pages
instead of 7 (3 instead of 8 when interactive).

`--booklet booklet.pdf` (or `.html`) writes the booklet to a file on top of the output chosen
with `--format`: a service can thus keep both the Markdown story and its booklet from a single
LLM run. A booklet that can't be written (no browser) is reported on `stderr` without losing
the story.

### Cover illustration

`--illustrate` adds a picture to the booklet's cover (HTML and PDF, `--format` or `--booklet`):
once the story is written, an image model draws its characters in their setting, from the plan:
the setting, each character described by its figure ("Pirate, red vest and ponytail") and its
portrait, and a few of the objects used. The prompt asks for chunky toy figures, and for no text
in the picture. The picture is embedded in the page (`data:` address), which stays a single file.

| `--image-model` | Provider | Requirements |
|---|---|---|
| `gpt-image-1` (default), `gpt-image-…`, `dall-e-3` | OpenAI | `OPENAI_API_KEY` |
| `gemini-2.5-flash-image`, `gemini-…` | Google Gemini | `GEMINI_API_KEY` |

The key is checked before any LLM call. A picture that fails (quota, refused by the provider's
safety system…) is reported on `stderr`: the booklet comes out without it. With `--replay DIR`,
the prompt is written to `DIR/illustration.prompt.txt` and the picture read from
`DIR/illustration.png` (or `.jpg`, `.webp`, `.svg`).

`--format pdf` does the printing itself: the booklet is printed by a headless Chrome, Chromium
or Edge already installed (looked up in `PATH` and the usual locations, or given by
`RAG_MOBIL_BROWSER`). The PDF is written to `stdout`, to be redirected to a file. The browser
is looked up before any LLM call. In a container running as root, Chromium needs
`--browser-arg --no-sandbox`.

## Architecture

```
src/
├── main.rs              CLI (clap)
├── lib.rs
├── error.rs             RagError (thiserror), one variant per step
├── model.rs             SetNumber, Part, SetInventory, missing pieces
├── pipeline.rs          RagPipeline<S: InventorySource, G: StoryGenerator>
├── saga.rs              --saga: previous episodes, character continuity
├── html.rs              --format html: illustrated booklet (pulldown-cmark)
├── pdf.rs               --format pdf: the booklet printed by headless Chrome
├── illustrate.rs        --illustrate: the cover picture (OpenAI, Gemini, replay)
├── retriever/           R — retrieval
│   ├── mod.rs           InventorySource trait
│   ├── playmodb.rs      HTTP client (2 pages in parallel)
│   ├── parser.rs        pure HTML parsing, tested on fixtures
│   ├── cache.rs         disk cache, one JSON file per set
│   ├── json_file.rs     --inventory-file
│   ├── http_json.rs     HTTP source in format v1 (--inventory-url)
│   └── fallback.rs      one source, then another for unknown sets
├── augment/             A — augmentation
│   ├── context.rs       inventory → structured context (<set>, <characters>, <pieces>)
│   └── prompt.rs        system prompt (faithfulness rules) + user message
├── generator/           G — generation
│   ├── mod.rs           StoryGenerator trait (streamed text, structured JSON)
│   └── llm.rs           multi-provider generator (genai) + fallback on refusal
└── writer/              writing
    ├── mod.rs           direct / planned methods, Observer (progress)
    ├── plan.rs          plan, check against the inventory, scene setup, "In your box"
    ├── citations.rs     --citations: references cited in the text, checked against the boxes
    ├── interactive.rs   choose-your-own-adventure: graph of paragraphs, checked
    ├── prompts.rs       plan / draft / review / revision prompts
    └── planned.rs       Plan → Draft → Edit (adapted from Re3/DOC)
```

### Inventory sources

By default, inventories come from PlaymoDB (see below). Any other database can provide them,
in [inventory format v1](docs/inventory-format.md) (JSON schema:
[`schema/inventory-v1.schema.json`](schema/inventory-v1.schema.json)), the same format as
`--inventory-json` and the cache:

```bash
# A service answering GET /api/inventories/<set>; PlaymoDB for the sets it doesn't know
rag-mobil 3666 70145 --inventory-url 'http://localhost:8096/api/inventories/{set}'

# That service only
rag-mobil 3666 --inventory-url 'http://localhost:8096/api/inventories/{set}' --no-playmodb

# A file
rag-mobil 3666 --inventory-file my-boxes.json
```

| Source | When | Fallback |
|---|---|---|
| `--inventory-file` | a file, one set or an array | none |
| `--inventory-url` / `RAG_MOBIL_INVENTORY_URL` | HTTP service, `{set}` replaced by the number | PlaymoDB (with cache) if the service answers `404`, unless `--no-playmodb` |
| (nothing) | PlaymoDB, cached unless `--no-cache` | none |

With `--inventory-url`, only a `404` answer hands over to PlaymoDB; any other error (service
down, invalid JSON, a set other than the one requested, empty inventory) stops the program.
`--cache-dir`, `--refresh` and `--no-cache` apply to the PlaymoDB fallback. `--inventory-file`
given on the command line wins over `RAG_MOBIL_INVENTORY_URL`. A `[rag-mobil]` line on `stderr`
tells where each set came from (e.g. `3666 (localhost:8096), 70145 (cache)`).

In an inventory, the `Klicky` (PlaymoDB) or `figure` category marks a figure, that is a
possible character; the other pieces are grouped by category.

### Retrieve — PlaymoDB

Two pages are fetched in parallel, then merged:

- `showinv.pl?setnum=N&pics=on`: name, theme, release date, **category** and **image** of
  each piece (`Klicky` = figures, `Animal`, `Furniture`, `Vehicle`…). Both PlaymoDB layouts are
  handled (list of links, `lineup` blocks with pictures). The image (the `image` field, an
  absolute URL) isn't used in the prompt: it's for whoever displays the inventory.
- `listinv.pl?setnum=N`: a text table with the **quantities** and sub-items (wig, hat…
  attached to a figure).

Pieces from the "Extra parts not included in inventory" section (alternative numbers or spare
parts) are dropped so the model doesn't count the same object twice. For sets PlaymoDB hasn't
counted yet, the quantity is unknown (`?`). A piece listed without a count under the
same name as a counted one is an alternative number and is dropped as well; any other piece
without a count is kept, its quantity unknown (set 3693 counts only 7 of its 17 pieces).

The scraper identifies itself with an explicit User-Agent and makes only two requests per set;
on a transient error (PlaymoDB often answers 508, its host's resource limit), it retries up to
3 times, after 1, 3 and then 6 seconds. PlaymoDB is a site run by volunteer fans: please don't
use it for mass scraping.

**Cache**: each fetched inventory is saved to disk (`~/.cache/rag-mobil/playmodb/<set>.json`
on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows) and reused for 30 days
without calling PlaymoDB again. `--refresh` forces a new fetch, `--no-cache` disables the
cache. An unreadable entry is simply fetched again.

**Several sets**: `rag-mobil 3799 70145` fetches the inventories one after the other
(duplicates ignored), with no limit on their number. The context holds one `<set>` block per
box, and the prompt says everything meets in the same story. Characters can come from any box,
and a reference present in several boxes adds up their copies. "In your box" tells, for each
line, which box to look in (`(box 70145)`). With many large boxes, the prompt grows: no problem
for Claude, but a small local model may choke.

### Augment — context and prompt

The inventory becomes a pseudo-XML block: figures first (they are the characters), then pieces
by category, with quantities and colors. The system prompt sets the faithfulness rules:
characters only from the box, objects present in the inventory, quantities respected,
technical pieces only used to describe the setting. The story ends with an "In your box"
section listing the items used.

### Generate — LLM through genai

Generation relies on the open-source library [`genai`](https://github.com/jeremychone/rust-genai)
(MIT/Apache-2.0), which provides a single streaming client for some twenty providers.
`generator/llm.rs` adds what is specific to this project:

- **Claude**: `max_tokens` at 64,000 and `medium` effort by default, which genai translates to
  `thinking: {type: "adaptive"}` + `output_config.effort`;
- **refusals**: if the model declines the request (`stop_reason: "refusal"` for Claude, a
  content filter elsewhere), the partial text is discarded; for Claude, a single retry is made
  on `--fallback-model`;
- **truncation**: stopping on `max_tokens` is reported;
- **prompt cache**: the drafting calls (one per part, one per paragraph in interactive mode)
  all start with the same system prompt, inventory and plan. For Claude, that prefix is marked
  `cache_control` (5 minutes): the first call writes it, the next ones read it back at a
  fraction of the input price, and faster. Other providers get the prompt as is (OpenAI and
  Gemini cache repeated prefixes by themselves);
- **tokens**: the summary on `stderr` adds up, over every call, the input tokens (with those
  read from the cache) and the tokens generated.

Other providers keep their default settings (many local models reject reasoning parameters);
`--effort` forces them.

### Writing — the Re3/DOC method

By default, the story isn't written in one go: the writing follows the method of two
open-source research story generators (MIT license),
[Re3](https://github.com/yangkevin2/emnlp22-re3-story-generation) (*Recursive Reprompting and
Revision*, EMNLP 2022) and [DOC](https://github.com/yangkevin2/doc-story-generation)
(*Detailed Outline Control*, ACL 2023), adapted to the inventory constraint:

1. **Plan** (Re3's *Plan* module): the LLM produces, as structured JSON, a premise, a setting,
   the characters and 3 to 5 steps. Each character names **the reference of a figure** from the
   box, each step **the references of the objects** used.
2. **Plan check, by code**: each reference must exist in the inventory, a character must be a
   figure, and a figure doesn't play more roles than there are copies in the box. If anything
   is off, the plan is requested once more with the list of errors; if discrepancies remain,
   the invalid references are removed. This step replaces Re3's trained ranking models. If the
   plan is still unusable (broken JSON, common with small local models), the story is written
   in a single call, as with `--method direct`.
3. **Draft** (DOC's detailed outline control): one part per step, guided by the plan, the
   step's characters and objects, and the text written so far.
4. **Edit** (Re3's *Edit* module): a reviewer lists, as JSON, the discrepancies with the box
   (missing object, quantity exceeded, contradicted appearance), and with `--citations` the
   code adds the cited references the box doesn't hold; if there are any, a rewrite fixes
   them. `--reviews 2` (or 3) reviews the rewritten story again, as long as issues remain.
5. **"In your box"**: the final list of characters and objects is generated **by code** from
   the validated plan, so it only names pieces from the box. Objects PlaymoDB splits into
   several lines (the parrot's body and wings, chest and lock, rowboat and oars…) are grouped
   together.

That's 5 to 9 LLM calls instead of one (2 more per extra review round): the story is shown at the end, the steps on `stderr`.
`--method direct` goes back to a single streamed call.

Another LLM can be plugged in by implementing the `StoryGenerator` trait, another data source
through `InventorySource` (or with no code, through `--inventory-url`), and the run can be
followed through the `Observer` trait.

## Development

```bash
cargo test     # offline: HTML fixtures, fake Claude server, scripted LLM, end-to-end
cargo clippy --all-targets
cargo fmt
```

### Bypassing the LLM (end-to-end tests)

`--replay DIR` replaces the LLM with files: each call writes its prompt to
`DIR/NN-text.prompt.txt` (or `NN-json`) and reads the answer from `DIR/NN-text.answer.txt`. If
an answer is missing, the program stops and says which file to write; run it again and the
answers already written are replayed. The rest of the pipeline runs for real. With
`--inventory-file`, no network access is needed:

```bash
rag-mobil 3799 --inventory-json > 3799.json          # once
rag-mobil 3799 --inventory-file 3799.json --replay session/
# … write session/01-json.answer.txt (the plan), run again, and so on
```

A complete session becomes an end-to-end test in `tests/e2e/<sets>/` (`inventory.json`,
`answers/`, and the expected outputs in `expected/`: final story and every prompt). Two
examples, with answers written by Claude:

- `tests/e2e/3799/`: "Pirates Secret Island", whose review spots a key that isn't in the box;
- `tests/e2e/3799+70145/`: the same island together with the "Diving Scooter", whose review
  spots a fishing rod and a diving lamp missing from both boxes.

After an intended change of prompts or output:

```bash
UPDATE_GOLDEN=1 cargo test --test e2e_replay
```

## Disclaimer

Unofficial project, not affiliated with geobra Brandstätter Stiftung & Co. KG.
PLAYMOBIL® is a registered trademark of geobra Brandstätter Stiftung & Co. KG.
By default, inventory data comes from [PlaymoDB](https://playmodb.org).

## License

[MIT](LICENSE). PlaymoDB's inventory data and images are not covered by this license.
