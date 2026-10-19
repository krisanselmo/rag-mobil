//! Retrieve → Augment → Generate, wired together.

use crate::augment::{Prompt, StoryOptions, build_prompt};
use crate::error::Result;
use crate::generator::{Generation, StoryGenerator};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;

pub struct Story {
    pub inventory: SetInventory,
    pub prompt: Prompt,
    pub generation: Generation,
}

pub struct RagPipeline<S, G> {
    source: S,
    generator: G,
}

impl<S: InventorySource, G: StoryGenerator> RagPipeline<S, G> {
    pub fn new(source: S, generator: G) -> Self {
        Self { source, generator }
    }

    pub async fn run<F>(&self, set: &SetNumber, options: &StoryOptions, on_text: F) -> Result<Story>
    where
        F: FnMut(&str) + Send,
    {
        let inventory = self.source.fetch(set).await?;
        let prompt = build_prompt(&inventory, options);
        let generation = self.generator.generate(&prompt, on_text).await?;
        Ok(Story {
            inventory,
            prompt,
            generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Part;

    struct FakeSource;
    impl InventorySource for FakeSource {
        async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
            Ok(SetInventory {
                set_number: set.clone(),
                name: "Pirate Ship".into(),
                theme: Some("Pirates".into()),
                focus: None,
                release_date: None,
                parts: vec![Part {
                    number: "30 00 001".into(),
                    name: "Pirate captain".into(),
                    color: None,
                    quantity: Some(1),
                    category: "Klicky".into(),
                    components: vec![],
                }],
                source_url: "test".into(),
            })
        }
    }

    /// Echoes the user prompt back, so we can assert what reached the LLM.
    struct EchoGenerator;
    impl StoryGenerator for EchoGenerator {
        async fn generate<F>(&self, prompt: &Prompt, mut on_text: F) -> Result<Generation>
        where
            F: FnMut(&str) + Send,
        {
            on_text(&prompt.user);
            Ok(Generation {
                text: prompt.user.clone(),
                model: "echo".into(),
                truncated: false,
                output_tokens: None,
            })
        }
    }

    #[tokio::test]
    async fn inventory_reaches_the_generator() {
        let pipeline = RagPipeline::new(FakeSource, EchoGenerator);
        let set = SetNumber::parse("5135").unwrap();
        let mut streamed = String::new();
        let story = pipeline
            .run(&set, &StoryOptions::default(), |t| streamed.push_str(t))
            .await
            .unwrap();
        assert!(story.generation.text.contains("1× Pirate captain"));
        assert!(
            story
                .generation
                .text
                .contains("set Playmobil 5135 « Pirate Ship »")
        );
        assert_eq!(streamed, story.generation.text);
    }
}
