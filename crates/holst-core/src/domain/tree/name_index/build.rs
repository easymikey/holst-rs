//! Compact search index of a workspace tree: flat arrays and two string arenas.

use std::borrow::Cow;

use icu_normalizer::ComposingNormalizerBorrowed;
use itertools::Itertools as _;
use serde_json::Value;

use crate::{
    domain::{
        node::NodeKind,
        tree,
        tree::{TreeFallback, TreeNode, TreeReply, name_index::query::NameGap},
    },
    error::Result,
};

/// A node number; only an [`NameIndex`] hands them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIndex(pub(crate) u32);

impl From<NodeIndex> for usize {
    fn from(node: NodeIndex) -> Self {
        widen(node.0)
    }
}

/// Widens a node number; saturates on a target where `usize` is narrower.
pub(crate) fn widen(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A node whose id is not a canonical uuid, kept as the server sent it.
#[derive(Debug)]
struct OddId {
    node: u32,
    id: Box<str>,
}

/// One node on its way into the index.
#[derive(Debug)]
struct IndexedNode<'a> {
    id: &'a str,
    kind: NodeKind,
    name: &'a str,
    in_trash: bool,
    order: f64,
}

const NO_PARENT: u32 = u32::MAX;

const SEPARATOR: char = '\n';

/// The compact search index of one workspace tree.
#[derive(Debug, Default)]
pub struct NameIndex {
    pub(crate) ids: Vec<u128>,
    odd_ids: Vec<OddId>,
    pub(crate) kinds: Vec<NodeKind>,
    pub(crate) in_trash: Vec<bool>,
    parents: Vec<u32>,
    order: Vec<f64>,
    name_ends: Vec<u32>,
    pub(crate) normalized_ends: Vec<u32>,
    names: String,
    pub(crate) normalized_names: String,
}

impl NameIndex {
    /// The index from a `workspace/{id}/tree` reply.
    ///
    /// # Errors
    ///
    /// [`crate::error::http::HttpError::Json`] when the reply is not JSON.
    ///
    /// ```
    /// use holst_core::domain::{
    ///     limit::Limit,
    ///     tree::name_index::{build::NameIndex, query::NodeFilter},
    /// };
    ///
    /// let index = NameIndex::from_tree_json(
    ///     r#"{"channels":[{"id":"b1","type":"board","name":"Plans Q4"}]}"#,
    /// )?;
    /// let (hits, total) = index.search("plans", NodeFilter::default(), Limit::from(10));
    /// assert_eq!((index.name(hits[0].node), total), ("Plans Q4", 1));
    ///
    /// # Ok::<(), holst_core::Error>(())
    /// ```
    pub fn from_tree_json(text: &str) -> Result<Self> {
        Self::parse(text).map(|(index, _)| index)
    }

    /// Like [`NameIndex::from_tree_json`], plus why the heuristic was used.
    ///
    /// # Errors
    ///
    /// [`crate::error::http::HttpError::Json`] when the reply is not JSON.
    pub fn parse(text: &str) -> Result<(Self, Option<TreeFallback>)> {
        TreeReply::parse(text)
            .map(|tree| {
                (
                    Self::build(
                        &tree.channels,
                        |c| IndexedNode {
                            id: &c.id,
                            kind: c.kind,
                            name: c.name.as_deref().unwrap_or(""),
                            in_trash: tree::is_trashed(c.in_trash.as_ref()),
                            order: c.sidebar_order.unwrap_or(f64::INFINITY),
                        },
                        |c| c.parent_id.as_deref(),
                    ),
                    None,
                )
            })
            .or_else(|_| {
                tree::parse(text)
                    .map(|parsed| (Self::from_nodes(&parsed.nodes), parsed.fallback))
            })
    }

    /// The index from already parsed nodes.
    #[must_use]
    pub fn from_nodes(nodes: &[TreeNode]) -> Self {
        Self::build(
            nodes,
            |n| IndexedNode {
                id: &n.id,
                kind: n.kind,
                name: &n.name,
                in_trash: n.in_trash,
                order: f64::INFINITY,
            },
            |n| n.parent_id.as_deref(),
        )
    }

    /// The index of a `workspace/{id}/trash` reply: everything in it counts as
    /// trashed.
    #[must_use]
    pub fn of_trash(reply: &Value) -> Self {
        let nodes: Vec<TreeNode> = tree::flatten(reply)
            .into_iter()
            .map(|node| TreeNode {
                in_trash: true,
                ..node
            })
            .collect();
        Self::from_nodes(&nodes)
    }

    fn build<'a, T>(
        items: &'a [T],
        new_node: impl Fn(&'a T) -> IndexedNode<'a>,
        parent_id: impl Fn(&'a T) -> Option<&'a str>,
    ) -> Self {
        items
            .iter()
            .fold(Self::with_capacity(items.len()), |mut index, item| {
                index.push(&new_node(item));
                index
            })
            .link_parents(items.iter().map(parent_id))
            .shrunk()
    }

    fn with_capacity(node_count: usize) -> Self {
        Self {
            ids: Vec::with_capacity(node_count),
            odd_ids: Vec::new(),
            kinds: Vec::with_capacity(node_count),
            in_trash: Vec::with_capacity(node_count),
            parents: Vec::with_capacity(node_count),
            order: Vec::with_capacity(node_count),
            name_ends: Vec::with_capacity(node_count),
            normalized_ends: Vec::with_capacity(node_count),
            names: String::with_capacity(node_count * 24),
            normalized_names: String::with_capacity(node_count * 24),
        }
    }

    fn push(&mut self, new: &IndexedNode<'_>) {
        let &IndexedNode {
            id,
            kind,
            name,
            in_trash,
            order,
        } = new;
        let node = saturating_u32(self.ids.len());
        let uuid = canonical_uuid(id);
        self.odd_ids.extend(uuid.is_none().then(|| OddId {
            node,
            id: id.into(),
        }));
        self.ids.push(uuid.unwrap_or(0));
        self.kinds.push(kind);
        self.in_trash.push(in_trash);
        self.order.push(order);
        self.names.push_str(name);
        self.name_ends.push(saturating_u32(self.names.len()));
        // Keyed on the node count: "🚀" normalizes to nothing.
        self.normalized_names
            .extend(self.normalized_ends.last().map(|_| SEPARATOR));
        normalize_into(name, &mut self.normalized_names);
        self.normalized_ends
            .push(saturating_u32(self.normalized_names.len()));
    }

    fn shrunk(self) -> Self {
        Self {
            names: self.names.into_boxed_str().into_string(),
            normalized_names: self.normalized_names.into_boxed_str().into_string(),
            name_ends: self.name_ends.into_boxed_slice().into_vec(),
            order: self.order.into_boxed_slice().into_vec(),
            normalized_ends: self.normalized_ends.into_boxed_slice().into_vec(),
            ..self
        }
    }

    fn link_parents<'a>(
        self,
        parent_ids: impl Iterator<Item = Option<&'a str>>,
    ) -> Self {
        let by_id: Vec<(u128, u32)> = self
            .ids
            .iter()
            .zip(0..)
            .filter(|&(&id, _)| id != 0)
            .map(|(&id, node)| (id, node))
            .sorted_unstable()
            .collect();
        let parents = parent_ids
            .map(|parent| {
                parent
                    .and_then(|parent| {
                        canonical_uuid(parent).map_or_else(
                            || self.odd_node(parent),
                            |key| {
                                by_id
                                    .binary_search_by_key(&key, |&(id, _)| id)
                                    .ok()
                                    .and_then(|position| by_id.get(position))
                                    .map(|&(_, node)| node)
                            },
                        )
                    })
                    .unwrap_or(NO_PARENT)
            })
            .collect();
        Self { parents, ..self }
    }

    pub(crate) fn odd_node(&self, id: &str) -> Option<u32> {
        self.odd_ids
            .iter()
            .find(|odd| &*odd.id == id)
            .map(|odd| odd.node)
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// The index has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Every node, in reply order.
    pub fn nodes(
        &self,
    ) -> impl DoubleEndedIterator<Item = NodeIndex> + ExactSizeIterator {
        (0..saturating_u32(self.len())).map(NodeIndex)
    }

    /// Id of a node as the server sent it; empty for a foreign number.
    #[must_use]
    pub fn id(&self, NodeIndex(node): NodeIndex) -> String {
        match self.ids.get(widen(node)) {
            Some(0) => self
                .odd_ids
                .iter()
                .find_map(|odd| (odd.node == node).then(|| odd.id.to_string()))
                .unwrap_or_default(),
            Some(&uuid) => uuid::Uuid::from_u128(uuid).hyphenated().to_string(),
            None => String::new(),
        }
    }

    /// Kind of a node; [`NodeKind::Other`] for a foreign number.
    #[must_use]
    pub fn kind(&self, NodeIndex(node): NodeIndex) -> NodeKind {
        at(&self.kinds, node, NodeKind::Other)
    }

    /// The node is in the trash; `false` for a foreign number.
    #[must_use]
    pub fn in_trash(&self, NodeIndex(node): NodeIndex) -> bool {
        at(&self.in_trash, node, false)
    }

    /// Sidebar order (`order_index`); `INFINITY` when unknown.
    #[must_use]
    pub fn order(&self, NodeIndex(node): NodeIndex) -> f64 {
        at(&self.order, node, f64::INFINITY)
    }

    /// Parent of a node; `None` for a root or a foreign number.
    #[must_use]
    pub fn parent(&self, NodeIndex(node): NodeIndex) -> Option<NodeIndex> {
        Some(at(&self.parents, node, NO_PARENT))
            .filter(|&parent| parent != NO_PARENT)
            .map(NodeIndex)
    }

    /// Name of a node as sent; empty for a foreign number.
    #[must_use]
    pub fn name(&self, NodeIndex(node): NodeIndex) -> &str {
        let node = widen(node);
        let start = node
            .checked_sub(1)
            .and_then(|prev| offset(&self.name_ends, prev));
        let end = offset(&self.name_ends, node);
        self.names
            .get(start.unwrap_or(0)..end.unwrap_or(0))
            .unwrap_or("")
    }

    pub(crate) fn normalized_name(&self, node: u32) -> &str {
        let end = offset(&self.normalized_ends, widen(node)).unwrap_or(0);
        self.normalized_names
            .get(self.normalized_start(node)..end)
            .unwrap_or("")
    }

    pub(crate) fn normalized_start(&self, node: u32) -> usize {
        widen(node)
            .checked_sub(1)
            .and_then(|prev| offset(&self.normalized_ends, prev))
            .map_or(0, |end| end + 1)
    }

    pub(crate) fn node_at(&self, position: usize) -> u32 {
        saturating_u32(
            self.normalized_ends
                .partition_point(|&end| widen(end) < position),
        )
    }
}

fn at<T: Copy>(values: &[T], node: u32, default: T) -> T {
    values.get(widen(node)).copied().unwrap_or(default)
}

pub(crate) fn offset(ends: &[u32], position: usize) -> Option<usize> {
    ends.get(position).map(|&end| widen(end))
}

pub(crate) fn saturating_u32(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

pub(crate) fn canonical_uuid(id: &str) -> Option<u128> {
    Some(id)
        .filter(|id| {
            crate::domain::id::is_uuid(id)
                && !id.bytes().any(|b| b.is_ascii_uppercase())
        })
        .and_then(|id| uuid::Uuid::try_parse(id).ok())
        .map(|uuid| uuid.as_u128())
        .filter(|&value| value != 0)
}

/// Appends the idempotent search form: lowercase, `ё`→`е`, single spaces.
///
/// ```
/// use holst_core::domain::tree::name_index::build::normalize_into;
///
/// let mut out = String::new();
/// normalize_into("  Ёжик:  План-Б!! ", &mut out);
/// assert_eq!(out, "ежик план б");
/// ```
pub fn normalize_into(raw: &str, out: &mut String) {
    let start = out.len();
    // One pass for text of inert chars, the usual case: NFC leaves it as it is.
    let nfc = fold_into(raw, out).then(|| folded_nfc(raw));
    out.truncate(nfc.as_ref().map_or(out.len(), |_| start));
    out.extend(nfc);
}

/// The search form of `raw` as a new string; see [`normalize_into`].
#[must_use]
pub fn normalized(raw: &str) -> String {
    normalized_onto(String::with_capacity(raw.len()), raw)
}

fn normalized_onto(mut out: String, raw: &str) -> String {
    normalize_into(raw, &mut out);
    out
}

fn folded_onto(mut out: String, raw: &str) -> String {
    fold_into(raw, &mut out);
    out
}

fn folded_nfc(raw: &str) -> String {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    let folded = folded_onto(String::with_capacity(raw.len()), &nfc.normalize(raw));
    // A mark dropped between two letters may let them compose: compose again.
    match nfc.normalize(&folded) {
        Cow::Owned(again) => again,
        Cow::Borrowed(_) => folded,
    }
}

fn fold_into(raw: &str, out: &mut String) -> bool {
    // Fast arms for ASCII and Cyrillic: Unicode tables are slow.
    let (_, marked) =
        raw.chars()
            .fold((NameGap::Leading, false), |(gap, marked), ch| match ch {
                'a'..='z' | '0'..='9' | 'а'..='я' => (emit(out, gap, ch), marked),
                'A'..='Z' => (emit(out, gap, ch.to_ascii_lowercase()), marked),
                'А'..='Я' => (
                    emit(out, gap, char::from_u32(u32::from(ch) + 0x20).unwrap_or(ch)),
                    marked,
                ),
                'ё' | 'Ё' => (emit(out, gap, 'е'), marked),
                _ => (fold_other(out, gap, ch), marked || !is_inert(ch)),
            });
    marked
}

fn fold_other(out: &mut String, gap: NameGap, ch: char) -> NameGap {
    match ch {
        _ if is_invisible(ch) => gap,
        // `İ` lowercases to `i` + a combining dot, which is dropped.
        _ if !ch.is_ascii() && ch.is_alphanumeric() => ch
            .to_lowercase()
            .filter(|lower| lower.is_alphanumeric() && !is_invisible(*lower))
            .fold(gap, |gap, lower| emit(out, gap, lower)),
        _ => gap.separated(),
    }
}

#[inline]
fn emit(out: &mut String, gap: NameGap, ch: char) -> NameGap {
    match gap {
        NameGap::Pending => {
            out.push(' ');
            out.push(ch);
        }
        NameGap::Leading | NameGap::Word => out.push(ch),
    }
    NameGap::Word
}

pub(crate) const fn is_inert(ch: char) -> bool {
    matches!(
        ch,
        '\0'..='\u{2ff}'
            | '\u{400}'..='\u{482}'
            | '\u{48a}'..='\u{4ff}'
            | '\u{2010}'..='\u{205e}'
            | '\u{2600}'..='\u{27bf}'
            | '\u{1f000}'..='\u{1faff}'
    )
}

pub(crate) const fn is_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{ad}'
            | '\u{300}'..='\u{36f}'
            | '\u{483}'..='\u{489}'
            | '\u{61c}'
            | '\u{180e}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{fe20}'..='\u{fe2f}'
            | '\u{feff}'
    )
}
