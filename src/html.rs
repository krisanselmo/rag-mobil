//! The story as a standalone HTML page, to read on screen or print: the
//! Markdown story, with the pictures of the parts (the `image` field of the
//! inventory) in the scene setups, in "In your box", and in place of each
//! `[réf. …]` citation.
//!
//! Printed (or saved as PDF), it is a booklet: a cover with the cast, one
//! page per scene, and "In your box" as a checklist, with page numbers.
//!
//! Everything the LLM wrote is escaped: its raw HTML is shown as text.

use std::fmt::Write;

use pulldown_cmark::{CowStr, Event, Parser, html::push_html};

use crate::augment::StoryOptions;
use crate::model::{Collection, Part, SetInventory};
use crate::pipeline::Story;
use crate::writer::StoryPlan;
use crate::writer::plan::{Labels, SceneSetup, box_contents, find_part, part_label, scene_setup};

/// How the page is laid out when printed (or saved as PDF).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Layout {
    /// A booklet: full-page cover, one page per scene, "In your box" on its
    /// own page.
    #[default]
    Booklet,
    /// Saves paper: a short header instead of the cover, the story in two
    /// columns without page breaks, small pictures.
    Compact,
}

/// The whole page.
pub fn render(story: &Story, options: &StoryOptions, layout: Layout) -> String {
    let collection = &story.collection;
    let written = &story.written;
    let labels = Labels::new(options);
    let texts = Texts::new(options);

    let (title, body) = match &written.plan {
        Some(plan) => {
            let mut body = cover_html(plan, collection, &texts);
            body.push_str("<div class=\"story\">\n");
            if !written.paragraphs.is_empty() {
                body.push_str(&gamebook_html(written, plan, collection, &labels));
            } else if written.staged {
                for (scene, step) in written.scenes.iter().zip(&plan.outline) {
                    body.push_str("<section class=\"scene\">\n");
                    let setup = scene_setup(step, plan, collection);
                    if !setup.is_empty() {
                        body.push_str(&setup_html(&setup, &labels));
                    }
                    body.push_str(&markdown(scene, collection));
                    body.push_str("</section>\n");
                }
            } else {
                let _ = write!(
                    body,
                    "<section class=\"scene\">\n{}</section>\n",
                    markdown(&written.scenes.join("\n\n"), collection)
                );
            }
            body.push_str("</div>\n");
            body.push_str(&in_your_box_html(plan, collection, &labels, &texts));
            (plan.title.trim().to_owned(), body)
        }
        // Direct method: the model's Markdown, title included.
        None => {
            let text = written.generation.text.trim();
            let title = text
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .map_or_else(|| collection.label(), |t| t.trim().to_owned());
            let body = format!(
                "<div class=\"story\">\n{}</div>\n",
                markdown(text, collection)
            );
            (title, body)
        }
    };
    page(&title, &body, collection, &texts, layout)
}

/// Words of the page outside the story (HTML).
struct Texts {
    /// `lang` attribute, for hyphenation.
    lang: &'static str,
    play_with: &'static str,
    tick: &'static str,
    found: &'static str,
    written_from: &'static str,
    trademark: &'static str,
}

impl Texts {
    fn new(options: &StoryOptions) -> Self {
        let language = options.language.to_lowercase();
        if language.starts_with("fr") {
            Self {
                lang: "fr",
                play_with: "Une histoire à rejouer avec ta boîte Playmobil",
                tick: "Coche chaque pièce quand tu l'as trouvée.",
                found: "trouvée",
                written_from: "Histoire écrite par rag-mobil d'après l'inventaire de",
                trademark: "Projet non officiel. PLAYMOBIL® est une marque déposée de geobra \
                            Brandstätter Stiftung &amp; Co. KG. Images des pièces : PlaymoDB.",
            }
        } else {
            Self {
                lang: if language.starts_with("en") { "en" } else { "" },
                play_with: "A story to play with your Playmobil box",
                tick: "Tick each piece once you have found it.",
                found: "found",
                written_from: "Story written by rag-mobil from the inventory of",
                trademark: "Unofficial project. PLAYMOBIL® is a registered trademark of geobra \
                            Brandstätter Stiftung &amp; Co. KG. Part pictures: PlaymoDB.",
            }
        }
    }
}

fn page(title: &str, body: &str, collection: &Collection, texts: &Texts, layout: Layout) -> String {
    let Texts {
        written_from,
        trademark,
        ..
    } = texts;
    let lang = if texts.lang.is_empty() {
        String::new()
    } else {
        format!(" lang=\"{}\"", texts.lang)
    };
    let class = match layout {
        Layout::Booklet => "booklet",
        Layout::Compact => "compact",
    };
    // The heading comes from the plan (or the text): escaped, unlike the body.
    let heading = if body.contains("<h1>") {
        String::new()
    } else {
        format!("<h1>{}</h1>\n", escape(title))
    };
    format!(
        "<!doctype html>\n<html{lang}>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>{STYLE}{COMPACT}</style>\n</head>\n<body class=\"{class}\">\n<main>\n\
         {heading}{body}</main>\n<footer>{written_from} {sets}.<br>{trademark}</footer>\n\
         </body>\n</html>\n",
        title = escape(title),
        sets = escape(&collection.label()),
    )
}

/// Markdown to HTML; citations become pictures, raw HTML becomes text, and
/// a line break is kept (dialogue lines).
fn markdown(text: &str, collection: &Collection) -> String {
    // Merge consecutive texts: the parser splits `[réf. …]` at the brackets.
    let mut events: Vec<Event<'_>> = Vec::new();
    for event in Parser::new(text) {
        let event = match event {
            Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
            Event::SoftBreak => Event::HardBreak,
            other => other,
        };
        match (events.last_mut(), event) {
            (Some(Event::Text(previous)), Event::Text(more)) => {
                *previous = CowStr::from(format!("{previous}{more}"));
            }
            (_, event) => events.push(event),
        }
    }
    let events = events.into_iter().flat_map(|event| match event {
        Event::Text(text) => citations(&text, collection),
        other => vec![other],
    });
    let mut html = String::new();
    push_html(&mut html, events);
    html
}

const CITATION: &str = "[réf.";

/// Splits a text around its `[réf. …]` citations, each reference becoming a
/// thumbnail (or a label when the part has no picture).
fn citations<'a>(text: &str, collection: &Collection) -> Vec<Event<'a>> {
    let mut events = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(CITATION) {
        let Some(len) = rest[start..].find(']') else {
            break;
        };
        let before = rest[..start].trim_end();
        if !before.is_empty() {
            events.push(Event::Text(CowStr::from(before.to_owned())));
        }
        let references = &rest[start + CITATION.len()..start + len];
        let html: String = references
            .split(',')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(|reference| match find_part(collection, reference) {
                Some(part) => thumbnail(part),
                None => format!("<span class=\"ref\">réf. {}</span>", escape(reference)),
            })
            .collect();
        events.push(Event::InlineHtml(CowStr::from(html)));
        rest = &rest[start + len + 1..];
    }
    if !rest.is_empty() {
        events.push(Event::Text(CowStr::from(rest.to_owned())));
    }
    events
}

/// A small inline picture of a part, with its name and reference on hover.
fn thumbnail(part: &Part) -> String {
    let tip = escape(&format!("{} — réf. {}", part_label(part), part.number));
    match image(part) {
        Some(src) => format!(
            "<img class=\"ref\" src=\"{src}\" alt=\"{alt}\" title=\"{tip}\" loading=\"lazy\">",
            alt = escape(&part_label(part)),
        ),
        None => format!(
            "<span class=\"ref\" title=\"{tip}\">réf. {}</span>",
            escape(&part.number)
        ),
    }
}

/// The picture's address, escaped, when it is an http(s) one.
fn image(part: &Part) -> Option<String> {
    let src = part.image.as_deref()?;
    (src.starts_with("https://") || src.starts_with("http://")).then(|| escape(src))
}

/// The first page: title, cast, premise, boxes.
fn cover_html(plan: &StoryPlan, collection: &Collection, texts: &Texts) -> String {
    let mut out = format!(
        "<header class=\"cover\">\n<p class=\"kicker\">{}</p>\n<h1>{}</h1>\n",
        texts.play_with,
        escape(plan.title.trim())
    );
    let contents = box_contents(plan, collection);
    if !contents.characters.is_empty() {
        out.push_str("<ul class=\"cast\">");
        for (character, figure) in &contents.characters {
            let picture = figure.and_then(|(_, part)| image(part)).map_or_else(
                || "<span class=\"no-image\" aria-hidden=\"true\">🧩</span>".to_owned(),
                |src| format!("<img src=\"{src}\" alt=\"\">"),
            );
            let _ = write!(out, "<li>{picture}<b>{}</b></li>", escape(&character.name));
        }
        out.push_str("</ul>\n");
    }
    let _ = write!(
        out,
        "<p class=\"premise\">{}</p>\n<p class=\"boxes\">{}</p>\n</header>\n",
        escape(plan.premise.trim()),
        escape(&collection.label())
    );
    out
}

/// A card: the pictures of the pieces, then a caption (already HTML). With
/// `check`, a box to tick once the pieces are found.
fn card(pieces: &[&Part], caption: &str, check: Option<&str>) -> String {
    let mut out = String::from("<li><figure>");
    if let Some(found) = check {
        let _ = write!(
            out,
            "<input type=\"checkbox\" aria-label=\"{}\">",
            escape(&format!("{} — {found}", labels_of(pieces)))
        );
    }
    let pictures: Vec<String> = pieces
        .iter()
        .filter_map(|p| {
            image(p).map(|src| {
                format!(
                    "<img src=\"{src}\" alt=\"{}\" loading=\"lazy\">",
                    escape(&part_label(p))
                )
            })
        })
        .collect();
    if pictures.is_empty() {
        out.push_str("<span class=\"no-image\" aria-hidden=\"true\">🧩</span>");
    } else {
        let _ = write!(out, "<span class=\"pictures\">{}</span>", pictures.concat());
    }
    let references: Vec<&str> = pieces.iter().map(|p| p.number.as_str()).collect();
    let _ = write!(
        out,
        "<figcaption>{caption}<small>réf. {}</small></figcaption></figure></li>",
        escape(&references.join(", "))
    );
    out
}

fn labels_of(pieces: &[&Part]) -> String {
    escape(
        &pieces
            .iter()
            .map(|p| part_label(p))
            .collect::<Vec<_>>()
            .join(" + "),
    )
}

/// Interactive story: numbered paragraphs, each ending with links to the
/// paragraphs its choices lead to.
fn gamebook_html(
    written: &crate::writer::Written,
    plan: &StoryPlan,
    collection: &Collection,
    labels: &Labels,
) -> String {
    let mut out = format!("<p class=\"how\">🎲 {}</p>\n", labels.interactive);
    let paragraphs = written
        .scenes
        .iter()
        .zip(&written.paragraphs)
        .zip(&plan.outline);
    for (i, ((text, paragraph), step)) in paragraphs.enumerate() {
        let n = i + 1;
        let _ = write!(
            out,
            "<section class=\"paragraph\" id=\"p{n}\">\n<h2 class=\"number\">{n}</h2>\n"
        );
        if written.staged {
            let setup = scene_setup(step, plan, collection);
            if !setup.is_empty() {
                out.push_str(&setup_html(&setup, labels));
            }
        }
        out.push_str(&markdown(text, collection));
        if paragraph.choices.is_empty() {
            let _ = writeln!(out, "<p class=\"end\">— {} —</p>", labels.the_end);
        } else {
            let _ = write!(
                out,
                "<nav class=\"choices\">\n<p>{}</p>\n<ul>",
                labels.choose
            );
            for (label, target) in &paragraph.choices {
                let _ = write!(
                    out,
                    "<li><a href=\"#p{target}\">{} <span>→ {} <b>{target}</b></span></a></li>",
                    escape(label),
                    labels.go_to
                );
            }
            out.push_str("</ul>\n</nav>\n");
        }
        out.push_str("</section>\n");
    }
    out
}

fn setup_html(setup: &SceneSetup<'_>, labels: &Labels) -> String {
    let mut out = format!(
        "<aside class=\"setup\">\n<h3>🧱 {}</h3>\n<ul class=\"cards\">",
        labels.scene_setup
    );
    for (name, figure) in &setup.characters {
        let caption = format!("<b>{}</b>{}", escape(name), labels_of(&[figure]));
        out.push_str(&card(&[figure], &caption, None));
    }
    for pieces in &setup.objects {
        out.push_str(&card(pieces, &labels_of(pieces), None));
    }
    out.push_str("</ul>\n</aside>\n");
    out
}

fn in_your_box_html(
    plan: &StoryPlan,
    collection: &Collection,
    labels: &Labels,
    texts: &Texts,
) -> String {
    let check = Some(texts.found);
    let contents = box_contents(plan, collection);
    let origin = |set: &SetInventory| {
        if collection.is_single() {
            String::new()
        } else {
            format!(
                "<i>{} {}</i>",
                labels.a_box,
                escape(set.set_number.as_str())
            )
        }
    };
    let mut out = format!(
        "<section class=\"box\">\n<h2>{}</h2>\n<p class=\"tick\">{}</p>\n",
        labels.in_your_box, texts.tick
    );
    if !contents.characters.is_empty() {
        let _ = write!(out, "<h3>{}</h3>\n<ul class=\"cards\">", labels.characters);
        for (character, figure) in &contents.characters {
            let name = format!("<b>{}</b>", escape(&character.name));
            out.push_str(&match figure {
                Some((set, part)) => card(
                    &[part],
                    &format!("{name}{}{}", labels_of(&[part]), origin(set)),
                    check,
                ),
                None => card(&[], &name, check),
            });
        }
        out.push_str("</ul>\n");
    }
    if !contents.objects.is_empty() {
        let _ = write!(out, "<h3>{}</h3>\n<ul class=\"cards\">", labels.objects);
        for (set, part) in &contents.objects {
            let quantity = part
                .quantity
                .map(|q| format!("<b>{q}×</b>"))
                .unwrap_or_default();
            let caption = format!("{quantity}{}{}", labels_of(&[part]), origin(set));
            out.push_str(&card(&[part], &caption, check));
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = r#"
:root {
  --paper: #fffdf7; --ink: #2b2b2b; --muted: #6b665c; --accent: #c8102e;
  --card: #ffffff; --line: #e6dfcf; --setup: #fff4d6;
}
@media screen and (prefers-color-scheme: dark) {
  :root {
    --paper: #1d1b18; --ink: #ece7dc; --muted: #a8a194; --accent: #ff6b6b;
    --card: #2a2723; --line: #3d3832; --setup: #332c1c;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--paper); color: var(--ink);
  font: 1.15rem/1.65 Georgia, "Times New Roman", serif;
}
main { max-width: 46rem; margin: 0 auto; padding: 2rem 1rem 1rem; }
h1 { font-size: 2.1rem; line-height: 1.2; color: var(--accent); text-align: center; }
h2, h3, figcaption, footer, .setup { font-family: system-ui, sans-serif; }
h2 { border-top: 2px dashed var(--line); padding-top: 1.5rem; margin-top: 2.5rem; }
h3 { font-size: 1rem; text-transform: uppercase; letter-spacing: .05em; color: var(--muted); }
p { margin: 0 0 1em; }
img.ref { height: 2.2em; vertical-align: middle; margin: 0 .15em; }
span.ref { font: .75em system-ui, sans-serif; color: var(--muted); margin: 0 .2em; }
.setup {
  background: var(--setup); border-radius: 12px; padding: .6rem 1rem 1rem;
  margin: 1.5rem 0 1rem; break-inside: avoid;
}
.setup h3 { margin: .3rem 0 .6rem; color: var(--ink); }
.cards {
  list-style: none; padding: 0; margin: 0;
  display: grid; grid-template-columns: repeat(auto-fill, minmax(8.5rem, 1fr)); gap: .75rem;
}
.cards li { break-inside: avoid; }
figure {
  position: relative; margin: 0; height: 100%; background: var(--card);
  border: 1px solid var(--line); border-radius: 10px; padding: .5rem; text-align: center;
}
figure input { position: absolute; top: .4rem; left: .4rem; width: 1.1rem; height: 1.1rem; margin: 0; }
.pictures { display: flex; justify-content: center; gap: .25rem; height: 6rem; }
.pictures img { max-height: 100%; max-width: 100%; object-fit: contain; min-width: 0; }
.no-image { display: block; font-size: 3rem; line-height: 6rem; opacity: .5; }
figcaption { font-size: .85rem; line-height: 1.35; margin-top: .35rem; }
figcaption b, figcaption i, figcaption small { display: block; }
figcaption b { font-size: .95rem; }
figcaption i, figcaption small { color: var(--muted); }
.cover { text-align: center; margin-bottom: 2rem; }
.kicker, .boxes, .tick { font: .85rem system-ui, sans-serif; color: var(--muted); }
.kicker { text-transform: uppercase; letter-spacing: .12em; margin: 0; }
.cover h1 { margin: .4rem 0 1.5rem; }
.cast {
  list-style: none; padding: 0; margin: 0 0 1.5rem;
  display: flex; flex-wrap: wrap; justify-content: center; gap: 1.25rem;
}
.cast li { width: 7rem; font-family: system-ui, sans-serif; }
.cast img { display: block; height: 9rem; max-width: 100%; margin: 0 auto .4rem; object-fit: contain; }
.cast .no-image { line-height: 9rem; }
.premise { font-style: italic; max-width: 32rem; margin: 0 auto 1rem; }
.how { font: .95rem system-ui, sans-serif; text-align: center; color: var(--muted); }
.paragraph { border-top: 2px dashed var(--line); margin-top: 2rem; padding-top: .5rem; }
h2.number {
  border: 0; margin: .5rem auto 1rem; padding: 0; width: 2.6rem; height: 2.6rem;
  line-height: 2.6rem; border-radius: 50%; text-align: center;
  background: var(--accent); color: #fff; font-size: 1.3rem;
}
.choices {
  font-family: system-ui, sans-serif; background: var(--setup); border-radius: 12px;
  padding: .6rem 1rem; margin: 1rem 0; break-inside: avoid;
}
.choices p { margin: 0 0 .4rem; font-weight: 600; }
.choices ul { list-style: none; margin: 0; padding: 0; }
.choices li { margin: .3rem 0; }
.choices a { color: var(--ink); text-decoration: none; }
.choices a span { color: var(--accent); white-space: nowrap; }
.end { text-align: center; font-weight: bold; letter-spacing: .1em; color: var(--accent); }
footer {
  max-width: 46rem; margin: 2rem auto; padding: 0 1rem;
  font-size: .75rem; color: var(--muted); text-align: center;
}
@page {
  size: A4; margin: 16mm 16mm 18mm;
  @bottom-center { content: counter(page); font: 9pt system-ui, sans-serif; color: #888; }
}
@page :first { @bottom-center { content: none; } }
@media print {
  :root {
    --paper: #fff; --ink: #1a1a1a; --muted: #666; --accent: #c8102e;
    --card: #fff; --line: #ddd; --setup: #fff4d6;
  }
  * { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
  body { font-size: 14pt; line-height: 1.6; }
  main { max-width: none; padding: 0; }
  p { text-align: justify; hyphens: auto; orphans: 3; widows: 3; }
  img.ref { height: 1.8em; }
  /* The cover fills the first page; then one page per scene. */
  .cover {
    min-height: 255mm; margin: 0; padding: 12mm; break-after: page;
    display: flex; flex-direction: column; justify-content: center;
    border: 3px double var(--accent); border-radius: 18px;
  }
  .cover p { text-align: center; }
  .cover h1 { font-size: 30pt; }
  .cast img { height: 11rem; }
  .cast li { width: 8.5rem; font-size: 13pt; }
  .scene + .scene { break-before: page; }
  /* A gamebook paragraph stays on one page, with its number and choices. */
  .paragraph { break-inside: avoid; margin-top: 1.2rem; }
  .how { margin: 0 0 .5rem; }
  /* Compact setups: the story keeps most of the page. */
  .setup { margin: 0 0 1.2rem; padding: .4rem .7rem .7rem; }
  .setup h3 { font-size: 9pt; margin: .2rem 0 .4rem; }
  .setup .cards { grid-template-columns: repeat(auto-fill, minmax(25mm, 1fr)); gap: .35rem; }
  .setup figure { padding: .3rem; border-radius: 6px; }
  .setup .pictures { height: 15mm; }
  .setup figcaption { font-size: 7pt; line-height: 1.25; }
  .setup figcaption b { font-size: 8pt; }
  .setup figcaption small { display: none; }
  .box { break-before: page; }
  .box h2 { border: 0; margin-top: 0; padding-top: 0; }
  .box .cards { grid-template-columns: repeat(auto-fill, minmax(32mm, 1fr)); gap: .5rem; }
  .box .pictures { height: 20mm; }
  .box figcaption { font-size: 8pt; }
  figure input { appearance: none; border: 1.5px solid #555; border-radius: 3px; background: #fff; }
  footer { margin: 1.5rem 0 0; font-size: 7.5pt; break-inside: avoid; }
}
"#;

/// Print rules of [`Layout::Compact`], over the booklet's.
const COMPACT: &str = r#"
@page compact { size: A4; margin: 11mm 12mm 14mm; }
@media print {
  body.compact { page: compact; font-size: 10.5pt; line-height: 1.4; }
  .compact .cover {
    min-height: 0; padding: 0 0 3mm; margin: 0 0 4mm; border: 0; border-radius: 0;
    border-bottom: 2px solid var(--accent); break-after: auto; display: block;
  }
  .compact .cover h1 { font-size: 20pt; margin: 1mm 0 2mm; }
  .compact .kicker { font-size: 7pt; }
  .compact .cast { gap: 4mm; margin: 0 0 2mm; }
  .compact .cast li { width: 18mm; font-size: 8pt; }
  .compact .cast img { height: 20mm; margin-bottom: 1mm; }
  .compact .cast .no-image { line-height: 20mm; font-size: 2rem; }
  .compact .premise { font-size: 9.5pt; margin-bottom: 1mm; }
  .compact .boxes { font-size: 7.5pt; margin: 0; }
  /* The story in two columns, no forced page break. */
  .compact .story { column-count: 2; column-gap: 7mm; column-rule: 1px solid var(--line); }
  .compact .scene + .scene { break-before: auto; margin-top: 3mm; padding-top: 2mm; border-top: 1px dashed var(--line); }
  .compact p { margin: 0 0 .5em; }
  .compact .setup { margin: 0 0 2mm; padding: 1.5mm 2mm 2mm; border-radius: 4px; }
  .compact .setup h3 { font-size: 7pt; margin: 0 0 1mm; }
  .compact .setup .cards { grid-template-columns: repeat(auto-fill, minmax(14mm, 1fr)); gap: 1mm; }
  .compact .setup figure { padding: .5mm; border-radius: 3px; }
  .compact .setup .pictures { height: 9mm; }
  .compact .setup .no-image { line-height: 9mm; font-size: 1.2rem; }
  .compact .setup figcaption { font-size: 5.5pt; line-height: 1.15; margin-top: .5mm; }
  .compact .setup figcaption b { font-size: 6pt; }
  .compact .how { font-size: 8pt; column-span: all; }
  .compact .paragraph { margin-top: 2mm; padding-top: 1mm; }
  .compact h2.number { width: 1.7rem; height: 1.7rem; line-height: 1.7rem; font-size: .9rem; margin: 1mm auto 2mm; }
  .compact .choices { padding: 1.5mm 2.5mm; margin: 2mm 0; font-size: 9pt; }
  .compact .end { margin: 1mm 0 0; }
  .compact img.ref { height: 1.4em; }
  /* "In your box": a dense checklist after the story. */
  .compact .box { break-before: auto; margin-top: 5mm; padding-top: 3mm; border-top: 2px solid var(--accent); }
  .compact .box h2 { font-size: 13pt; margin: 0 0 1mm; }
  .compact .box h3 { font-size: 7.5pt; margin: 2mm 0 1mm; }
  .compact .tick { font-size: 7.5pt; margin: 0 0 1mm; }
  .compact .box .cards { grid-template-columns: repeat(auto-fill, minmax(21mm, 1fr)); gap: 1.2mm; }
  .compact .box figure { padding: 1mm; border-radius: 4px; }
  .compact .box .pictures { height: 12mm; }
  .compact .box .no-image { line-height: 12mm; font-size: 1.4rem; }
  .compact .box figcaption { font-size: 6pt; line-height: 1.2; }
  .compact .box figcaption b { font-size: 6.5pt; }
  .compact figure input { width: 3mm; height: 3mm; top: 1mm; left: 1mm; }
  .compact footer { margin-top: 3mm; font-size: 6.5pt; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::Generation;
    use crate::writer::Written;
    use crate::writer::plan::tests::{inventory, valid_plan};

    fn story(text: &str, plan: bool, scenes: &[&str], staged: bool) -> Story {
        let mut inventory = inventory();
        inventory.parts[0].image = Some("https://playmodb.org/pics/30004960.jpg".into());
        inventory.parts[2].image = Some("javascript:alert(1)".into());
        Story {
            collection: inventory.into(),
            written: Written {
                generation: Generation {
                    text: text.into(),
                    model: "test".into(),
                    truncated: false,
                    output_tokens: None,
                },
                plan: plan.then(valid_plan),
                scenes: scenes.iter().map(|s| (*s).to_owned()).collect(),
                staged,
                paragraphs: vec![],
                issues: vec![],
                calls: 1,
            },
        }
    }

    #[test]
    fn planned_story_has_setups_and_box_cards() {
        let story = story(
            "",
            true,
            &[
                "Léon sonne <b>fort</b>.\n— Debout !",
                "Rose trouve le cheval.",
            ],
            true,
        );
        let html = render(&story, &StoryOptions::default(), Layout::Booklet);
        assert!(html.starts_with("<!doctype html>\n<html lang=\"fr\">"));
        assert!(html.contains("<title>Le cheval perdu</title>"));
        // The cover: title, cast, premise, boxes.
        assert!(html.contains(
            "<header class=\"cover\">\n<p class=\"kicker\">Une histoire à rejouer avec ta \
             boîte Playmobil</p>\n<h1>Le cheval perdu</h1>\n<ul class=\"cast\"><li><img \
             src=\"https://playmodb.org/pics/30004960.jpg\" alt=\"\"><b>Léon</b></li><li>\
             <span class=\"no-image\" aria-hidden=\"true\">🧩</span><b>Rose</b></li></ul>\n\
             <p class=\"premise\">Le cheval du château a disparu.</p>\n\
             <p class=\"boxes\">3666 « Kings Large Castle »</p>\n</header>"
        ));
        assert_eq!(html.matches("<h1>").count(), 1);
        // One section per scene, its setup first.
        assert_eq!(html.matches("<section class=\"scene\">").count(), 2);
        assert!(html.contains("<section class=\"scene\">\n<aside class=\"setup\">"));
        // Raw HTML from the model is text.
        assert!(html.contains("<p>Léon sonne &lt;b&gt;fort&lt;/b&gt;.<br />\n— Debout !</p>"));
        assert_eq!(html.matches("<aside class=\"setup\">").count(), 2);
        assert!(html.contains("<h3>🧱 Mise en place</h3>"));
        // The trumpeter's picture: on the cover, in both setups, in the box.
        assert_eq!(
            html.matches("<img src=\"https://playmodb.org/pics/30004960.jpg\"")
                .count(),
            4
        );
        // Only http(s) pictures.
        assert!(!html.contains("javascript:"));
        assert!(html.contains("<h2>Dans ta boîte</h2>"));
        assert!(html.contains("<b>4×</b>Halberd<small>réf. 30 07 640</small>"));
        // "In your box" is a checklist; the setups aren't.
        assert!(html.contains("<p class=\"tick\">Coche chaque pièce quand tu l'as trouvée.</p>"));
        assert!(
            html.contains("<li><figure><input type=\"checkbox\" aria-label=\"Halberd — trouvée\">")
        );
        let box_start = html.find("<section class=\"box\">").unwrap();
        assert!(!html[..box_start].contains("checkbox"));
        let setups = html.find("Mise en place").unwrap();
        assert!(setups < html.find("Rose trouve").unwrap());
    }

    #[test]
    fn unstaged_story_has_no_setup() {
        let story = story("", true, &["Une seule partie."], false);
        let html = render(&story, &StoryOptions::default(), Layout::Booklet);
        assert!(!html.contains("class=\"setup\""));
        assert!(html.contains("<section class=\"scene\">\n<p>Une seule partie.</p>\n</section>"));
    }

    #[test]
    fn citations_become_pictures() {
        let story = story(
            "# Le trompettiste\n\nLéon [réf. 30 00 496] joue, la hallebarde \
             [réf. 30 07 640, 99 99 999] à la main.",
            false,
            &[],
            false,
        );
        let html = render(&story, &StoryOptions::default(), Layout::Booklet);
        assert!(html.contains("<title>Le trompettiste</title>"));
        assert_eq!(html.matches("<h1>").count(), 1, "the text's own title");
        assert!(html.contains(
            "<p>Léon<img class=\"ref\" src=\"https://playmodb.org/pics/30004960.jpg\" \
             alt=\"Castle Trumpeter\" title=\"Castle Trumpeter — réf. 30 00 496\" \
             loading=\"lazy\"> joue, la hallebarde<span class=\"ref\" \
             title=\"Halberd — réf. 30 07 640\">réf. 30 07 640</span>\
             <span class=\"ref\">réf. 99 99 999</span> à la main.</p>"
        ));
    }

    #[test]
    fn interactive_story_links_its_paragraphs() {
        use crate::writer::Paragraph;
        let mut story = story("", true, &["Début <i>!</i>", "Fin heureuse."], false);
        story.written.paragraphs = vec![
            Paragraph {
                choices: vec![("Suivre <le> cheval".into(), 2)],
            },
            Paragraph { choices: vec![] },
        ];
        let html = render(&story, &StoryOptions::default(), Layout::Booklet);
        assert!(html.contains("<p class=\"how\">🎲 C'est toi qui choisis la suite"));
        assert!(
            html.contains("<section class=\"paragraph\" id=\"p1\">\n<h2 class=\"number\">1</h2>")
        );
        assert!(html.contains(
            "<li><a href=\"#p2\">Suivre &lt;le&gt; cheval <span>→ va au <b>2</b></span></a></li>"
        ));
        assert!(html.contains("<p class=\"end\">— Fin —</p>"));
        assert!(!html.contains("class=\"scene\""));
        assert!(!html.contains("class=\"setup\""), "not staged");
    }

    #[test]
    fn compact_layout_is_a_class() {
        let story = story("", true, &["Une partie."], false);
        let booklet = render(&story, &StoryOptions::default(), Layout::Booklet);
        let compact = render(&story, &StoryOptions::default(), Layout::Compact);
        assert!(booklet.contains("<body class=\"booklet\">"));
        assert!(compact.contains("<body class=\"compact\">"));
        assert!(compact.contains("<div class=\"story\">\n<section class=\"scene\">"));
        // Same content, only the print rules differ.
        assert_eq!(
            booklet.replace("class=\"booklet\"", "class=\"compact\""),
            compact
        );
    }

    #[test]
    fn english_labels() {
        let story = story("", true, &["One.", "Two."], true);
        let options = StoryOptions {
            language: "English".into(),
            ..StoryOptions::default()
        };
        let html = render(&story, &options, Layout::Booklet);
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("Set the scene"));
        assert!(html.contains("<h2>In your box</h2>"));
    }
}
