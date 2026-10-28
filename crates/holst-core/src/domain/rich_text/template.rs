//! The five notes the web editor offers to start a board's note from.

use serde::{Deserialize, Serialize};

use super::{Block, BlockKind, plain_runs};

/// A built-in note (web bundle: English texts, lines 238214-238297).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    strum::Display,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum NoteTemplate {
    Todo,
    Agenda,
    Summary,
    Workshop,
    Annotation,
}

fn line(kind: BlockKind, text: &str) -> Block {
    Block::new(kind, plain_runs(text))
}

fn under(mut block: Block, nested: Vec<Block>) -> Block {
    block.nested = nested;
    block
}

impl NoteTemplate {
    /// The blocks of the note.
    #[must_use]
    pub fn blocks(self) -> Vec<Block> {
        use BlockKind::{Bullet, Check, HeadingOne, HeadingTwo, Numbered, Paragraph};
        let blocks = |pairs: &[(BlockKind, &str)]| -> Vec<Block> {
            pairs
                .iter()
                .map(|(kind, text)| line(kind.clone(), text))
                .collect()
        };
        match self {
            NoteTemplate::Todo => blocks(&[
                (HeadingOne, "To do list"),
                (Paragraph, "[Describe the project you are working on here]"),
                (Check(true), "Add an action item here"),
                (Check(false), ""),
            ]),
            NoteTemplate::Agenda => vec![
                line(HeadingOne, "Meeting agenda"),
                line(Paragraph, "Date: [Meeting date]"),
                line(HeadingTwo, "Agenda details"),
                under(
                    line(Numbered(None), "Add agenda item"),
                    vec![line(Bullet, "Add agenda details")],
                ),
                line(Numbered(None), ""),
            ],
            NoteTemplate::Summary => blocks(&[
                (HeadingOne, "Project summary"),
                (HeadingTwo, "Product Description"),
                (
                    Paragraph,
                    "[Give a high-level overview of the product / project you're working on, its goals, etc]",
                ),
                (HeadingTwo, "Target Audience"),
                (
                    Paragraph,
                    "[Elaborate on the target audience of your project/product, link out to additional resources]",
                ),
                (HeadingTwo, "Competition"),
                (
                    Paragraph,
                    "[List your competitors here and recommendations how to position your product against the competition & handle objections]",
                ),
                (Bullet, "Competitor 1"),
                (HeadingTwo, "Risk / Opportunity"),
                (
                    Paragraph,
                    "[List the challenges and opportunities your product / project can encounter]",
                ),
                (Numbered(None), "Add an opportunity here"),
                (Numbered(None), "Add a risk here"),
                (HeadingTwo, "Conclusions"),
                (
                    Paragraph,
                    "[Add your recommendations, suggestions, and next steps here]",
                ),
                (Check(false), "Add an action item here"),
            ]),
            NoteTemplate::Workshop => blocks(&[
                (HeadingOne, "Workshop notes"),
                (Paragraph, "[Add date & time here]"),
                (Paragraph, "[Add presenter here]"),
                (Paragraph, "[Add topic here]"),
                (HeadingTwo, "Notes"),
                (Paragraph, "Start typing your notes here"),
                (HeadingTwo, "Key takeaways"),
                (Numbered(None), "Add your takeaways here"),
                (HeadingTwo, "Action items"),
                (Check(false), "Add an action item here"),
            ]),
            NoteTemplate::Annotation => blocks(&[
                (HeadingOne, "Board annotation"),
                (
                    Paragraph,
                    "[Add text here to tell your team about the purpose of the board and explain how they should interact with it]",
                ),
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::VariantNames as _;

    use super::NoteTemplate;

    #[test]
    fn every_template_starts_with_its_heading() {
        let firsts: Vec<String> = [
            NoteTemplate::Todo,
            NoteTemplate::Agenda,
            NoteTemplate::Summary,
            NoteTemplate::Workshop,
            NoteTemplate::Annotation,
        ]
        .map(|template| {
            let first = template.blocks().remove(0);
            first.runs.iter().map(|run| run.text.as_str()).collect()
        })
        .into();
        assert_eq!(
            firsts,
            [
                "To do list",
                "Meeting agenda",
                "Project summary",
                "Workshop notes",
                "Board annotation"
            ]
        );
        assert_eq!(
            NoteTemplate::VARIANTS,
            ["todo", "agenda", "summary", "workshop", "annotation"]
        );
    }
}
