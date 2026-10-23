//! The cover illustration: a cinematic still of the story's key moment,
//! drawn by an image model from the plan, for the HTML/PDF booklet.
//!
//! | Model name                          | Provider | API key          |
//! |-------------------------------------|----------|------------------|
//! | `gpt-image-…`, `dall-e-…`           | OpenAI   | `OPENAI_API_KEY` |
//! | `gemini-…` (e.g. `gemini-2.5-flash-image`) | Gemini | `GEMINI_API_KEY` |
//!
//! With `--replay DIR`, the prompt is written to `DIR/illustration.prompt.txt`
//! and the picture read from `DIR/illustration.{png,jpg,jpeg,webp,svg}`.

use std::fmt::Write;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::augment::StoryOptions;
use crate::error::{RagError, Result};
use crate::model::Collection;
use crate::writer::plan::{part_label, scene_setup};
use crate::writer::{PlotPoint, StoryPlan};

pub const DEFAULT_IMAGE_MODEL: &str = "gpt-image-1";
/// Replaces `https://api.openai.com` (tests, OpenAI-compatible servers).
pub const OPENAI_IMAGE_BASE_URL_ENV: &str = "RAG_MOBIL_OPENAI_IMAGE_BASE_URL";
/// Replaces `https://generativelanguage.googleapis.com` (tests).
pub const GEMINI_BASE_URL_ENV: &str = "RAG_MOBIL_GEMINI_BASE_URL";
/// Objects named in the prompt, at most: the picture is a shot, not an inventory.
const MAX_OBJECTS: usize = 8;
/// File name of the picture and its prompt, with `--replay`.
const REPLAY_NAME: &str = "illustration";
const REPLAY_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "svg"];

/// A picture, ready to be embedded in the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Illustration {
    pub bytes: Vec<u8>,
    /// Media type, e.g. `image/png`.
    pub mime: String,
    /// Model that drew it (`replay` with `--replay`).
    pub model: String,
}

impl Illustration {
    /// `data:` address, for an `<img src>` that needs no file next to the page.
    /// The media type comes from the provider: anything unexpected is sent
    /// as PNG, so it can't break out of the attribute.
    pub fn data_url(&self) -> String {
        let mime = match self.mime.as_str() {
            known @ ("image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/svg+xml") => {
                known
            }
            _ => "image/png",
        };
        format!("data:{mime};base64,{}", STANDARD.encode(&self.bytes))
    }
}

/// Where pictures come from.
pub enum Illustrator {
    OpenAi {
        client: reqwest::Client,
        base_url: String,
        key: String,
        model: String,
    },
    Gemini {
        client: reqwest::Client,
        base_url: String,
        key: String,
        model: String,
    },
    /// Prompt written to a file, picture read from another.
    Replay(PathBuf),
}

impl Illustrator {
    /// The provider of `model`, with its key read now: a missing key fails
    /// before any story is written.
    pub fn new(model: &str) -> Result<Self> {
        let key = |env: &str| {
            std::env::var(env)
                .ok()
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| RagError::Image(format!("{env} is not set (image model {model})")))
        };
        let base_url = |env: &str, default: &str| {
            std::env::var(env)
                .ok()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| default.to_owned())
                .trim_end_matches('/')
                .to_owned()
        };
        let client = reqwest::Client::builder()
            .user_agent(crate::retriever::USER_AGENT)
            .build()?;
        if model.starts_with("gpt-image") || model.starts_with("dall-e") {
            Ok(Self::OpenAi {
                client,
                base_url: base_url(OPENAI_IMAGE_BASE_URL_ENV, "https://api.openai.com"),
                key: key("OPENAI_API_KEY")?,
                model: model.to_owned(),
            })
        } else if model.starts_with("gemini") {
            Ok(Self::Gemini {
                client,
                base_url: base_url(
                    GEMINI_BASE_URL_ENV,
                    "https://generativelanguage.googleapis.com",
                ),
                key: key("GEMINI_API_KEY")?,
                model: model.to_owned(),
            })
        } else {
            Err(RagError::Image(format!(
                "unknown image model {model} (expected gpt-image-…, dall-e-… or gemini-…)"
            )))
        }
    }

    pub async fn draw(&self, prompt: &str) -> Result<Illustration> {
        match self {
            Self::OpenAi {
                client,
                base_url,
                key,
                model,
            } => {
                let mut body = json!({"model": model, "prompt": prompt, "n": 1});
                if model.starts_with("dall-e") {
                    // gpt-image always answers in base64, DALL·E only on request.
                    body["response_format"] = json!("b64_json");
                    body["size"] = json!("1792x1024");
                } else {
                    body["size"] = json!("1536x1024");
                }
                let url = format!("{base_url}/v1/images/generations");
                let answer = post(client.post(&url).bearer_auth(key), &url, &body).await?;
                let data = answer["data"][0]["b64_json"]
                    .as_str()
                    .ok_or_else(|| RagError::Image(format!("{url}: no image in the answer")))?;
                Ok(Illustration {
                    bytes: decode(data)?,
                    mime: "image/png".into(),
                    model: model.clone(),
                })
            }
            Self::Gemini {
                client,
                base_url,
                key,
                model,
            } => {
                let body = json!({
                    "contents": [{"parts": [{"text": prompt}]}],
                    // Widescreen, like the film still the prompt asks for.
                    "generationConfig": {
                        "responseModalities": ["IMAGE"],
                        "imageConfig": {"aspectRatio": "16:9"},
                    },
                });
                let url = format!("{base_url}/v1beta/models/{model}:generateContent");
                let answer =
                    post(client.post(&url).header("x-goog-api-key", key), &url, &body).await?;
                let image = answer["candidates"][0]["content"]["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find_map(|part| part.get("inlineData"))
                    .ok_or_else(|| RagError::Image(format!("{url}: no image in the answer")))?;
                Ok(Illustration {
                    bytes: decode(image["data"].as_str().unwrap_or_default())?,
                    mime: image["mimeType"].as_str().unwrap_or("image/png").to_owned(),
                    model: model.clone(),
                })
            }
            Self::Replay(dir) => replay(dir, prompt),
        }
    }
}

async fn post(request: reqwest::RequestBuilder, url: &str, body: &Value) -> Result<Value> {
    let response = request.json(body).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        // The provider's message says why (quota, safety system, model name…).
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or(text);
        return Err(RagError::Image(format!(
            "{url}: HTTP {}: {}",
            status.as_u16(),
            message.trim()
        )));
    }
    Ok(serde_json::from_str(&text)?)
}

fn decode(data: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(data.trim())
        .map_err(|err| RagError::Image(format!("invalid base64 image: {err}")))
}

fn replay(dir: &std::path::Path, prompt: &str) -> Result<Illustration> {
    let io = |path: PathBuf| move |source| RagError::Io { path, source };
    std::fs::create_dir_all(dir).map_err(io(dir.to_owned()))?;
    let prompt_path = dir.join(format!("{REPLAY_NAME}.prompt.txt"));
    std::fs::write(&prompt_path, prompt).map_err(io(prompt_path.clone()))?;
    for extension in REPLAY_EXTENSIONS {
        let path = dir.join(format!("{REPLAY_NAME}.{extension}"));
        if path.is_file() {
            let bytes = std::fs::read(&path).map_err(io(path.clone()))?;
            let mime = match *extension {
                "jpg" | "jpeg" => "image/jpeg",
                "svg" => "image/svg+xml",
                other => return Ok(found(bytes, format!("image/{other}"))),
            };
            return Ok(found(bytes, mime.into()));
        }
    }
    Err(RagError::ReplayMissing {
        prompt: prompt_path,
        answer: dir.join(format!("{REPLAY_NAME}.png")),
    })
}

fn found(bytes: Vec<u8>, mime: String) -> Illustration {
    Illustration {
        bytes,
        mime,
        model: "replay".into(),
    }
}

/// What the image model is asked to draw: a film still of the story's key
/// moment (the plot point about 3/5 through, where the trouble peaks), with
/// the characters present described by their figures and portraits, and the
/// objects of that moment. Camera, light and motion are spelled out: image
/// models default to a flat, posed group shot. In English whatever the
/// story's language: image models follow it best.
pub fn prompt(plan: Option<&StoryPlan>, collection: &Collection, options: &StoryOptions) -> String {
    let mut out = format!(
        "A cinematic film still for the cover of a story booklet for a {age}-year-old \
         child, as if taken from an animated adventure movie, but it is real toy \
         photography: the characters are chunky plastic toy figures with round heads, \
         a simple painted smile, dot eyes and C-shaped hands, like classic Playmobil \
         figures, in a detailed diorama built from the same toy line.\n\
         Camera: low angle, wide-angle lens close to the action, the main characters \
         large in the foreground on the rule of thirds, toy pieces blurred in the \
         foreground and the setting receding into the background with a shallow \
         depth of field, 16:9 widescreen framing.\n\
         Light: golden-hour sun low in the sky, warm rim light outlining the figures, \
         long shadows, light rays through haze, rich teal-and-orange color grading.\n\
         Action: the moment is caught mid-movement, with dynamic tilted poses and \
         flying details (spray, sand, leaves). Exciting and full of wonder, never \
         scary for a {age}-year-old.\n\
         Show only the characters and things listed below, each looking exactly like \
         its toy. No text, letters, logos or watermarks in the picture.\n",
        age = options.reader_age
    );
    let Some(plan) = plan else {
        // Direct method: no plan, the boxes' figures.
        let _ = write!(
            out,
            "\nThe toys come from {}.\n\nCharacters:\n",
            collection.label()
        );
        for figure in collection.figures() {
            let _ = writeln!(out, "- {}", part_label(figure));
        }
        return out;
    };
    let _ = write!(
        out,
        "\nStory: \"{}\". {}\nSetting: {}\n",
        plan.title.trim(),
        plan.premise.trim(),
        plan.setting.trim()
    );
    let Some(step) = key_moment(plan) else {
        return out;
    };
    let setup = scene_setup(step, plan, collection);
    let _ = writeln!(out, "\nThe moment in the shot: {}", step.event.trim());
    if !setup.characters.is_empty() {
        out.push_str("\nIn the shot:\n");
        for (name, figure) in &setup.characters {
            let portrait = plan
                .characters
                .iter()
                .find(|c| c.name == *name)
                .map(|c| format!(": {}", c.portrait.trim()))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "- {} (toy figure: {}){portrait}",
                name.trim(),
                part_label(figure)
            );
        }
    }
    if !setup.objects.is_empty() {
        out.push_str("\nThings in the shot:\n");
        for pieces in setup.objects.iter().take(MAX_OBJECTS) {
            let labels: Vec<String> = pieces.iter().map(|p| part_label(p)).collect();
            let _ = writeln!(out, "- {}", labels.join(" + "));
        }
    }
    out
}

/// The plot point about 3/5 through the outline: past the setup, at the
/// height of the trouble, before it is solved.
fn key_moment(plan: &StoryPlan) -> Option<&PlotPoint> {
    let n = plan.outline.len();
    plan.outline.get((n * 3 / 5).min(n.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::plan::tests::{inventory, valid_plan};

    #[test]
    fn prompt_describes_the_cast_by_their_figures() {
        let collection = Collection::new(vec![inventory()]);
        let plan = valid_plan();
        let text = prompt(Some(&plan), &collection, &StoryOptions::default());
        assert!(text.contains(&format!("Story: \"{}\"", plan.title)));
        let moment = key_moment(&plan).unwrap();
        assert!(text.contains(&format!("The moment in the shot: {}", moment.event)));
        // "leon" in the outline is Leon, with his portrait.
        assert!(text.contains("- Rose (toy figure: "));
        assert!(text.contains("- Leon (toy figure: "));
        assert!(text.contains(": cheerful trumpeter"));
        assert!(text.contains("Camera: low angle"));
        assert!(text.contains("No text"));
    }

    #[test]
    fn key_moment_is_past_the_middle() {
        let mut plan = valid_plan();
        let step = plan.outline[0].clone();
        for (n, expected) in [(1, 0), (2, 1), (3, 1), (4, 2), (5, 3)] {
            plan.outline = (0..n)
                .map(|i| PlotPoint {
                    event: i.to_string(),
                    ..step.clone()
                })
                .collect();
            assert_eq!(key_moment(&plan).unwrap().event, expected.to_string());
        }
        plan.outline.clear();
        assert!(key_moment(&plan).is_none());
    }

    #[test]
    fn prompt_without_plan_lists_the_figures() {
        let collection = Collection::new(vec![inventory()]);
        let text = prompt(None, &collection, &StoryOptions::default());
        for figure in collection.figures() {
            assert!(text.contains(&part_label(figure)));
        }
    }

    #[test]
    fn replay_reads_the_picture_next_to_its_prompt() {
        let dir = std::env::temp_dir().join(format!("rag-mobil-illu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let missing = replay(&dir, "a castle").unwrap_err();
        assert!(matches!(missing, RagError::ReplayMissing { .. }));
        assert_eq!(
            std::fs::read_to_string(dir.join("illustration.prompt.txt")).unwrap(),
            "a castle"
        );
        std::fs::write(dir.join("illustration.svg"), "<svg/>").unwrap();
        let picture = replay(&dir, "a castle").unwrap();
        assert_eq!(picture.mime, "image/svg+xml");
        assert_eq!(picture.data_url(), "data:image/svg+xml;base64,PHN2Zy8+");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_model_is_refused() {
        assert!(matches!(
            Illustrator::new("claude-opus-5-5"),
            Err(RagError::Image(_))
        ));
    }
}
