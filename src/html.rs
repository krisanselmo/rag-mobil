//! The story as a standalone HTML page, to read on screen or print: the
//! Markdown story, with the pictures of the parts (the `image` field of the
//! inventory) in the scene setups, in "In your box", and in place of each
//! `[réf. …]` citation.
//!
//! Everything the LLM wrote is escaped: its raw HTML is shown as text.

use std::fmt::Write;

use pulldown_cmark::{CowStr, Event, Parser, html::push_html};

use crate::augment::StoryOptions;
use crate::model::{Collection, Part, SetInventory};
use crate::pipeline::Story;
use crate::writer::plan::{Labels, SceneSetup, box_contents, find_part, part_label, scene_setup};

/// The whole page.
pub fn render(story: &Story, options: &StoryOptions) -> String {
    let collection = &story.collection;
    let written = &story.written;
    let labels = Labels::new(options);

    let (title, body) = match &written.plan {
        Some(plan) => {
            let mut body = String::new();
            if written.staged {
                for (scene, step) in written.scenes.iter().zip(&plan.outline) {
                    let setup = scene_setup(step, plan, collection);
                    if !setup.is_empty() {
                        body.push_str(&setup_html(&setup, &labels));
                    }
                    body.push_str(&markdown(scene, collection));
                }
            } else {
                body.push_str(&markdown(&written.scenes.join("\n\n"), collection));
            }
            body.push_str(&in_your_box_html(plan, collection, &labels));
            (plan.title.trim().to_owned(), body)
        }
        // Direct method: the model's Markdown, title included.
        None => {
            let text = written.generation.text.trim();
            let title = text
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .map_or_else(|| collection.label(), |t| t.trim().to_owned());
            (title, markdown(text, collection))
        }
    };
    page(&title, &body, collection, options)
}

fn page(title: &str, body: &str, collection: &Collection, options: &StoryOptions) -> String {
    let french = options.language.to_lowercase().starts_with("fr");
    let lang = if french {
        "fr"
    } else if options.language.to_lowercase().starts_with("en") {
        "en"
    } else {
        ""
    };
    let (written_from, trademark) = if french {
        (
            "Histoire écrite par rag-mobil d'après l'inventaire de",
            "Projet non officiel. PLAYMOBIL® est une marque déposée de geobra \
             Brandstätter Stiftung &amp; Co. KG. Images des pièces : PlaymoDB.",
        )
    } else {
        (
            "Story written by rag-mobil from the inventory of",
            "Unofficial project. PLAYMOBIL® is a registered trademark of geobra \
             Brandstätter Stiftung &amp; Co. KG. Part pictures: PlaymoDB.",
        )
    };
    let lang = if lang.is_empty() {
        String::new()
    } else {
        format!(" lang=\"{lang}\"")
    };
    // The heading comes from the plan (or the text): escaped, unlike the body.
    let heading = if body.trim_start().starts_with("<h1>") {
        String::new()
    } else {
        format!("<h1>{}</h1>\n", escape(title))
    };
    format!(
        "<!doctype html>\n<html{lang}>\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n\
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

/// A card: the pictures of the pieces, then a caption (already HTML).
fn card(pieces: &[&Part], caption: &str) -> String {
    let mut out = String::from("<li><figure>");
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

fn setup_html(setup: &SceneSetup<'_>, labels: &Labels) -> String {
    let mut out = format!(
        "<aside class=\"setup\">\n<h3>🧱 {}</h3>\n<ul class=\"cards\">",
        labels.scene_setup
    );
    for (name, figure) in &setup.characters {
        let caption = format!("<b>{}</b>{}", escape(name), labels_of(&[figure]));
        out.push_str(&card(&[figure], &caption));
    }
    for pieces in &setup.objects {
        out.push_str(&card(pieces, &labels_of(pieces)));
    }
    out.push_str("</ul>\n</aside>\n");
    out
}

fn in_your_box_html(
    plan: &crate::writer::StoryPlan,
    collection: &Collection,
    labels: &Labels,
) -> String {
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
    let mut out = format!("<section class=\"box\">\n<h2>{}</h2>\n", labels.in_your_box);
    if !contents.characters.is_empty() {
        let _ = write!(out, "<h3>{}</h3>\n<ul class=\"cards\">", labels.characters);
        for (character, figure) in &contents.characters {
            let name = format!("<b>{}</b>", escape(&character.name));
            out.push_str(&match figure {
                Some((set, part)) => card(
                    &[part],
                    &format!("{name}{}{}", labels_of(&[part]), origin(set)),
                ),
                None => card(&[], &name),
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
            out.push_str(&card(&[part], &caption));
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
@media (prefers-color-scheme: dark) {
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
  margin: 0; height: 100%; background: var(--card); border: 1px solid var(--line);
  border-radius: 10px; padding: .5rem; text-align: center;
}
.pictures { display: flex; justify-content: center; gap: .25rem; height: 6rem; }
.pictures img { max-height: 100%; max-width: 100%; object-fit: contain; min-width: 0; }
.no-image { display: block; font-size: 3rem; line-height: 6rem; opacity: .5; }
figcaption { font-size: .85rem; line-height: 1.35; margin-top: .35rem; }
figcaption b, figcaption i, figcaption small { display: block; }
figcaption b { font-size: .95rem; }
figcaption i, figcaption small { color: var(--muted); }
footer {
  max-width: 46rem; margin: 2rem auto; padding: 0 1rem;
  font-size: .75rem; color: var(--muted); text-align: center;
}
@media print {
  :root { --paper: #fff; --ink: #000; --card: #fff; }
  main { max-width: none; padding-top: 0; }
  h2 { break-before: page; border: 0; }
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
        let html = render(&story, &StoryOptions::default());
        assert!(html.starts_with("<!doctype html>\n<html lang=\"fr\">"));
        assert!(html.contains("<title>Le cheval perdu</title>"));
        assert!(html.contains("<h1>Le cheval perdu</h1>"));
        // Raw HTML from the model is text.
        assert!(html.contains("<p>Léon sonne &lt;b&gt;fort&lt;/b&gt;.<br />\n— Debout !</p>"));
        assert_eq!(html.matches("<aside class=\"setup\">").count(), 2);
        assert!(html.contains("<h3>🧱 Mise en place</h3>"));
        // The trumpeter's picture, in both setups and in the box.
        assert_eq!(
            html.matches("<img src=\"https://playmodb.org/pics/30004960.jpg\"")
                .count(),
            3
        );
        // Only http(s) pictures.
        assert!(!html.contains("javascript:"));
        assert!(html.contains("<h2>Dans ta boîte</h2>"));
        assert!(html.contains("<b>4×</b>Halberd<small>réf. 30 07 640</small>"));
        let setups = html.find("Mise en place").unwrap();
        assert!(setups < html.find("Rose trouve").unwrap());
    }

    #[test]
    fn unstaged_story_has_no_setup() {
        let story = story("", true, &["Une seule partie."], false);
        let html = render(&story, &StoryOptions::default());
        assert!(!html.contains("class=\"setup\""));
        assert!(html.contains("<p>Une seule partie.</p>"));
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
        let html = render(&story, &StoryOptions::default());
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
    fn english_labels() {
        let story = story("", true, &["One.", "Two."], true);
        let options = StoryOptions {
            language: "English".into(),
            ..StoryOptions::default()
        };
        let html = render(&story, &options);
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("Set the scene"));
        assert!(html.contains("<h2>In your box</h2>"));
    }
}
