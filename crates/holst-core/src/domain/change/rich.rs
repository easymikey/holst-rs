//! Rich text laid out as the web editor does: a `wrapper` per block.

use std::sync::Arc;

use yrs::{
    Any,
    Out,
    ReadTxn,
    Text as _,
    TransactionMut,
    Xml as _,
    XmlTextPrelim,
    XmlTextRef,
    types::{Attrs, text::YChange},
};

use crate::{
    domain::{
        change::value::{Backdrop, TextLook, Tick},
        id_source::IdSource,
        rich_text::{Block, BlockKind, Mark, Run},
    },
    protocol::document::{BACKGROUND_COLOR_MARK, COLOR},
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

/// The marks and colours of `look` on every leaf of the text under `root`.
pub(super) fn restyle(
    transaction: &mut TransactionMut<'_>,
    root: &XmlTextRef,
    look: &TextLook,
) {
    let marks = look.marks.iter().flat_map(|kept| {
        Mark::ALL.map(|mark| {
            let on = kept.contains(&mark);
            (
                mark.name(),
                Some(if on { Any::Bool(true) } else { Any::Null }),
            )
        })
    });
    let colors = [
        (
            COLOR,
            look.color.as_ref().map(|color| Any::from(color.as_str())),
        ),
        (
            BACKGROUND_COLOR_MARK,
            look.background.as_ref().map(|background| match background {
                Backdrop::Clear => Any::Null,
                Backdrop::Color(color) => Any::from(color.as_str()),
            }),
        ),
    ];
    let attrs: Attrs = marks
        .chain(colors)
        .filter_map(|(name, value)| Some((Arc::from(name), value?)))
        .collect();
    if !attrs.is_empty() {
        format_leaves(transaction, root, &attrs);
    }
}

/// Sets or clears `checked` on a checklist item of the text under `root`.
///
/// # Errors
/// The number of checklist items the text has, when it has not that many.
pub(super) fn tick(
    transaction: &mut TransactionMut<'_>,
    root: Option<&XmlTextRef>,
    tick: Tick,
) -> Result<(), usize> {
    let items = root.map_or_else(Vec::new, |root| checklist(&*transaction, root));
    let at = usize::try_from(tick.item.get().saturating_sub(1)).ok();
    at.and_then(|at| items.get(at))
        .map(|element| {
            element.insert_attribute(transaction, "checked", Any::Bool(tick.done));
        })
        .ok_or(items.len())
}

/// The checklist items under `node` in reading order, the nested ones after
/// their parent.
fn checklist<T: ReadTxn>(transaction: &T, node: &XmlTextRef) -> Vec<XmlTextRef> {
    let is_item = matches!(
        node.get_attribute(transaction, "type"),
        Some(Out::Any(Any::String(kind))) if &*kind == BlockKind::Check(false).element()
    );
    is_item
        .then(|| node.clone())
        .into_iter()
        .chain(
            node.diff(transaction, YChange::identity)
                .into_iter()
                .flat_map(|chunk| match chunk.insert {
                    Out::YXmlText(child) => checklist(transaction, &child),
                    Out::Any(_)
                    | Out::YText(_)
                    | Out::YArray(_)
                    | Out::YMap(_)
                    | Out::YXmlElement(_)
                    | Out::YXmlFragment(_)
                    | Out::YDoc(_)
                    | Out::UndefinedRef(_) => Vec::new(),
                }),
        )
        .collect()
}

/// `attrs` on the text of `element` and of the elements inside it.
fn format_leaves(
    transaction: &mut TransactionMut<'_>,
    element: &XmlTextRef,
    attrs: &Attrs,
) {
    let chunks = element.diff(&*transaction, YChange::identity);
    // a leaf is formatted at the place the earlier chunks end
    chunks.into_iter().fold(0_u32, |at, chunk| {
        at + match chunk.insert {
            Out::YXmlText(child) => {
                format_leaves(transaction, &child, attrs);
                1
            }
            Out::Any(Any::String(text)) => {
                let len = u32::try_from(text.len()).unwrap_or(u32::MAX);
                element.format(transaction, at, len, attrs.clone());
                len
            }
            Out::Any(_)
            | Out::YText(_)
            | Out::YArray(_)
            | Out::YMap(_)
            | Out::YXmlElement(_)
            | Out::YXmlFragment(_)
            | Out::YDoc(_)
            | Out::UndefinedRef(_) => 1,
        }
    });
}
