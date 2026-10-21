//! `--citations`: the `[ref. …]` the LLM copies into the story, checked by
//! code against the boxes. A reference the boxes don't hold becomes an issue
//! for the revision, and is removed from the final text if it survives it.

use std::ops::Range;

use crate::model::Collection;
use crate::writer::plan::{Issue, find_part};

/// `[réf.` is the marker of stories written before the English prompts.
const MARKERS: [&str; 2] = ["[ref.", "[réf."];

/// One `[ref. …]` group in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation<'a> {
    /// The whole group, brackets included.
    pub range: Range<usize>,
    /// The marker opening it (`[ref.` or `[réf.`).
    pub marker: &'a str,
    /// Its references, trimmed, in order.
    pub references: Vec<&'a str>,
}

/// The first citation marker in `text`: its position and length.
fn find_marker(text: &str) -> Option<(usize, usize)> {
    MARKERS
        .iter()
        .filter_map(|marker| text.find(marker).map(|at| (at, marker.len())))
        .min()
}

/// Every citation group of `text`, in order. An unclosed group ends the search.
pub fn find(text: &str) -> Vec<Citation<'_>> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some((start, marker)) = find_marker(&text[offset..]) {
        let start = offset + start;
        let Some(len) = text[start..].find(']') else {
            break;
        };
        let end = start + len + 1;
        found.push(Citation {
            range: start..end,
            marker: &text[start..start + marker],
            references: text[start + marker..end - 1]
                .split(',')
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .collect(),
        });
        offset = end;
    }
    found
}

/// The references of `citation` that no box holds.
fn unknown<'a>(citation: &Citation<'a>, collection: &Collection) -> Vec<&'a str> {
    citation
        .references
        .iter()
        .copied()
        .filter(|r| find_part(collection, r).is_none())
        .collect()
}

/// The citation with up to `WORDS` words before it, as found in `text`, so
/// that a reviser (or the interactive writer) can locate it.
fn excerpt<'a>(text: &'a str, citation: &Citation<'_>) -> &'a str {
    const WORDS: usize = 4;
    let before = &text[..citation.range.start];
    let line = before.rfind('\n').map_or(0, |at| at + 1);
    let mut start = citation.range.start;
    let mut words = 0;
    let mut in_word = false;
    for (at, c) in before[line..].char_indices().rev() {
        if c.is_whitespace() {
            if in_word {
                words += 1;
                if words == WORDS {
                    break;
                }
            }
            in_word = false;
        } else {
            in_word = true;
            start = line + at;
        }
    }
    &text[start..citation.range.end]
}

/// One issue per citation naming a reference the boxes don't hold, in the
/// format of the reviewer's.
pub fn issues(text: &str, collection: &Collection) -> Vec<Issue> {
    let boxes = if collection.is_single() {
        "the box"
    } else {
        "the boxes"
    };
    find(text)
        .iter()
        .filter_map(|citation| {
            let unknown = unknown(citation, collection);
            (!unknown.is_empty()).then(|| Issue {
                excerpt: excerpt(text, citation).to_owned(),
                problem: format!(
                    "cites {}, not in {boxes}",
                    unknown
                        .iter()
                        .map(|r| format!("ref. {r}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                fix: format!(
                    "use a piece from {boxes} with its own reference, or no reference \
                     if the thing is only mentioned"
                ),
            })
        })
        .collect()
}

/// `text` without the references the boxes don't hold, and those references.
/// A group left empty is removed with the space before it.
pub fn strip_unknown(text: &str, collection: &Collection) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut dropped = Vec::new();
    let mut copied = 0;
    for citation in find(text) {
        let unknown = unknown(&citation, collection);
        if unknown.is_empty() {
            continue;
        }
        let kept: Vec<&str> = citation
            .references
            .iter()
            .copied()
            .filter(|r| !unknown.contains(r))
            .collect();
        dropped.extend(unknown.iter().map(|r| (*r).to_owned()));
        if kept.is_empty() {
            out.push_str(text[copied..citation.range.start].trim_end_matches([' ', '\t']));
        } else {
            out.push_str(&text[copied..citation.range.start]);
            out.push_str(citation.marker);
            out.push(' ');
            out.push_str(&kept.join(", "));
            out.push(']');
        }
        copied = citation.range.end;
    }
    out.push_str(&text[copied..]);
    (out, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::plan::tests::inventory;

    fn boxes() -> Collection {
        Collection::new(vec![inventory()])
    }

    #[test]
    fn finds_every_group() {
        let text = "Leon [ref. 30 00 496] plays, the halberd [réf. 30 07 640, 99 99 999].";
        let found = find(text);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].range.clone()], "[ref. 30 00 496]");
        assert_eq!(found[1].marker, "[réf.");
        assert_eq!(found[1].references, ["30 07 640", "99 99 999"]);
        assert!(find("An unclosed [ref. 30 00 496").is_empty());
    }

    #[test]
    fn unknown_references_become_issues() {
        let reference = inventory().parts[0].number.clone();
        let text = format!(
            "Pip [ref. {reference}] waves.\nThen he picks up the shiny golden key \
             [ref. 99 99 999, {reference}] and runs."
        );
        let issues = issues(&text, &boxes());
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].excerpt,
            format!("the shiny golden key [ref. 99 99 999, {reference}]")
        );
        assert_eq!(issues[0].problem, "cites ref. 99 99 999, not in the box");
        assert!(text.contains(&issues[0].excerpt));
    }

    #[test]
    fn excerpt_stays_on_its_line() {
        let text = "One.\nKey [ref. 99 99 999]";
        let issues = issues(text, &boxes());
        assert_eq!(issues[0].excerpt, "Key [ref. 99 99 999]");
    }

    #[test]
    fn strips_only_unknown_references() {
        let reference = inventory().parts[0].number.clone();
        let text = format!(
            "A key [ref. 99 99 999], a hat [réf. {reference}, 88 88 888] and [ref. {reference}]."
        );
        let (text, dropped) = strip_unknown(&text, &boxes());
        assert_eq!(
            text,
            format!("A key, a hat [réf. {reference}] and [ref. {reference}].")
        );
        assert_eq!(dropped, ["99 99 999", "88 88 888"]);

        let clean = format!("A hat [ref. {reference}].");
        assert_eq!(strip_unknown(&clean, &boxes()), (clean.clone(), Vec::new()));
    }
}
