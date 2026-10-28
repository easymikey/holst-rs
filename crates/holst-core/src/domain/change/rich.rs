//! Rich text laid out as the web editor does: a `wrapper` per block.

use std::sync::Arc;

use yrs::{
    Any,
    Text as _,
    TransactionMut,
    Xml as _,
    XmlTextPrelim,
    XmlTextRef,
    types::Attrs,
};

use crate::domain::{
    id_source::IdSource,
    rich_text::{Block, BlockKind, Run},
};

/// The colour token the editor sets on a link's text (bundle line 234938).
const LINK_COLOR: f64 = 1_671_390.0;

/// `blocks` as the children of `root`, one `wrapper` each; a nested block is a
/// `wrapper` after its parent's element inside the parent's `wrapper`.
pub(super) fn fill_blocks(
    transaction: &mut TransactionMut<'_>,
    root: &XmlTextRef,
    blocks: &[Block],
    ids: &dyn IdSource,
) {
    blocks
        .iter()
        .zip(0_u32..)
        .for_each(|(block, at)| wrap(transaction, root, at, block, ids));
}

fn wrap(
    transaction: &mut TransactionMut<'_>,
    parent: &XmlTextRef,
    at: u32,
    block: &Block,
    ids: &dyn IdSource,
) {
    let wrapper = parent.insert_embed(transaction, at, XmlTextPrelim::new(""));
    wrapper.insert_attribute(transaction, "type", "wrapper");
    wrapper.insert_attribute(transaction, "key", ids.new_id());
    let element = wrapper.insert_embed(transaction, 0, XmlTextPrelim::new(""));
    element.insert_attribute(transaction, "type", block.kind.element());
    element.insert_attribute(transaction, "key", ids.new_id());
    let extra: Option<(&str, Any)> = match &block.kind {
        BlockKind::Numbered(Some(number)) => {
            Some(("counter", Any::from(f64::from(*number))))
        }
        BlockKind::Check(done) => Some(("checked", Any::Bool(*done))),
        BlockKind::Code { language } => {
            Some(("language", Any::from(language.as_str())))
        }
        BlockKind::Numbered(None)
        | BlockKind::Paragraph
        | BlockKind::HeadingOne
        | BlockKind::HeadingTwo
        | BlockKind::HeadingThree
        | BlockKind::Bullet
        | BlockKind::Divider => None,
    };
    if let Some((name, value)) = extra {
        element.insert_attribute(transaction, name, value);
    }
    write_runs(transaction, &element, &block.runs, ids);
    block
        .nested
        .iter()
        .zip(1_u32..)
        .for_each(|(nested, slot)| wrap(transaction, &wrapper, slot, nested, ids));
}

/// The runs after one another; a stretch of one link is a `link` element.
fn write_runs(
    transaction: &mut TransactionMut<'_>,
    element: &XmlTextRef,
    runs: &[Run],
    ids: &dyn IdSource,
) {
    runs.chunk_by(|a, b| a.link == b.link).for_each(|group| {
        let link = group.first().and_then(|run| run.link.as_deref());
        let target = link.map_or_else(
            || element.clone(),
            |url| {
                let at = element.len(transaction);
                let linked = element.insert_embed_with_attributes(
                    transaction,
                    at,
                    XmlTextPrelim::new(""),
                    Attrs::new(),
                );
                linked.insert_attribute(transaction, "type", "link");
                linked.insert_attribute(transaction, "key", ids.new_id());
                linked.insert_attribute(transaction, "link", url);
                linked
            },
        );
        // imperative: each run is appended after the last, so the writes are ordered
        for run in group {
            let at = target.len(transaction);
            let marks = marks_of(run, link.is_some());
            target.insert_with_attributes(transaction, at, &run.text, marks);
        }
    });
}

/// The attributes of a run's text: its marks, and a link's colour.
fn marks_of(run: &Run, linked: bool) -> Attrs {
    run.marks
        .iter()
        .map(|mark| (Arc::from(mark.name()), Any::Bool(true)))
        .chain(linked.then(|| (Arc::from("color"), Any::from(LINK_COLOR))))
        .collect()
}
