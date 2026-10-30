//! A workspace tree (`workspace/{id}/tree`) as a list of nodes.

pub mod name_index;
pub mod walk;

use std::{borrow::Cow, fmt};

use serde::{
    Deserialize,
    Deserializer,
    de,
    de::{IgnoredAny, Visitor},
};
use serde_json::{Map, Value};

use crate::{domain::node::NodeKind, error::Result};

/// One board, folder or other node of a workspace tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub parent_id: Option<String>,
    pub in_trash: bool,
}

/// Why the tree was read by the heuristic instead of by the spec.
#[derive(Debug, thiserror::Error)]
pub enum TreeFallback {
    #[error("the reply does not have the shape expected")]
    UnknownShape(#[source] serde_json::Error),
    #[error("channels contain nested nodes")]
    Nested,
}

/// Nodes of a tree and, when the reply was not the spec's shape, why.
#[derive(Debug)]
pub struct ParsedTree {
    pub nodes: Vec<TreeNode>,
    pub fallback: Option<TreeFallback>,
}

/// A tree reply as nodes: spec `channels`, else [`flatten`] with the reason.
///
/// # Errors
///
/// [`crate::error::http::HttpError::Json`] when the text is not JSON at all.
///
/// ```
/// use holst_core::domain::tree;
///
/// let parsed =
///     tree::parse(r#"{"channels":[{"id":"b1","type":"board","name":"Plans"}]}"#)?;
/// assert_eq!(parsed.nodes[0].name, "Plans");
/// assert!(parsed.fallback.is_none());
///
/// # Ok::<(), holst_core::Error>(())
/// ```
pub fn parse(text: &str) -> Result<ParsedTree> {
    // Nodes have no order: skipping `order_index` saves parsing a float.
    TreeReply::<IgnoredAny>::parse(text)
        .map(|tree| ParsedTree {
            nodes: tree.channels.into_iter().map(TreeNode::from).collect(),
            fallback: None,
        })
        .or_else(|why| {
            Ok(ParsedTree {
                nodes: flatten(&serde_json::from_str(text)?),
                fallback: Some(why),
            })
        })
}

fn leaves<T>(
    channels: Vec<T>,
    is_leaf: impl Fn(&T) -> bool,
) -> std::result::Result<Vec<T>, TreeFallback> {
    if channels.iter().all(is_leaf) {
        Ok(channels)
    } else {
        Err(TreeFallback::Nested)
    }
}

impl<Order> From<TreeReplyNode<'_, Order>> for TreeNode {
    fn from(channel: TreeReplyNode<'_, Order>) -> Self {
        Self {
            id: channel.id.into_owned(),
            kind: channel.kind,
            name: channel.name.map(Cow::into_owned).unwrap_or_default(),
            parent_id: channel.parent_id.map(Cow::into_owned),
            in_trash: is_trashed(channel.in_trash.as_ref()),
        }
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "Order: Deserialize<'de>"))]
struct TreeReply<'a, Order = f64> {
    #[serde(borrow)]
    channels: Vec<TreeReplyNode<'a, Order>>,
}

impl<'a, Order: Deserialize<'a>> TreeReply<'a, Order> {
    fn parse(text: &'a str) -> std::result::Result<Self, TreeFallback> {
        serde_json::from_str::<Self>(text)
            .map_err(TreeFallback::UnknownShape)
            .and_then(|tree| leaves(tree.channels, TreeReplyNode::is_leaf))
            .map(|channels| Self { channels })
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "Order: Deserialize<'de>"))]
struct TreeReplyNode<'a, Order> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(rename = "type", default = "other", deserialize_with = "node_kind")]
    kind: NodeKind,
    #[serde(borrow, default)]
    name: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    parent_id: Option<Cow<'a, str>>,
    #[serde(default)]
    in_trash: Option<Value>,
    #[serde(default)]
    #[serde(rename = "order_index")]
    sidebar_order: Option<Order>,
    #[serde(default)]
    children: Option<IgnoredAny>,
    #[serde(default)]
    folders: Option<IgnoredAny>,
    #[serde(default)]
    items: Option<IgnoredAny>,
}

impl<Order> TreeReplyNode<'_, Order> {
    fn is_leaf(&self) -> bool {
        self.children.is_none() && self.folders.is_none() && self.items.is_none()
    }
}

const fn other() -> NodeKind {
    NodeKind::Other
}

fn node_kind<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<NodeKind, D::Error> {
    struct NodeKindVisitor;
    impl Visitor<'_> for NodeKindVisitor {
        type Value = NodeKind;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a node type")
        }
        fn visit_str<E: de::Error>(
            self,
            raw: &str,
        ) -> std::result::Result<NodeKind, E> {
            Ok(NodeKind::from(raw))
        }
        fn visit_unit<E: de::Error>(self) -> std::result::Result<NodeKind, E> {
            Ok(NodeKind::Other)
        }
    }
    deserializer.deserialize_any(NodeKindVisitor)
}

/// `in_trash` means "in the trash" unless it is absent, `null` or `false`.
#[must_use]
pub fn is_trashed(value: Option<&Value>) -> bool {
    !matches!(value, None | Some(Value::Null | Value::Bool(false)))
}

/// Nodes of any tree JSON: objects with an `id`, nested under known keys.
///
/// ```
/// use holst_core::domain::tree::flatten;
/// use serde_json::json;
///
/// let tree = json!([{ "id": "f", "children": [{ "id": "b" }] }]);
/// let nodes = flatten(&tree);
/// assert_eq!(nodes[1].parent_id.as_deref(), Some("f"));
/// ```
#[must_use]
pub fn flatten(tree: &Value) -> Vec<TreeNode> {
    // A `Vec` by value through the recursion was 14–23% slower.
    let mut out = Vec::new();
    collect(tree, None, &mut out);
    out
}

fn collect(value: &Value, parent: Option<&str>, out: &mut Vec<TreeNode>) {
    value
        .as_array()
        .into_iter()
        .flatten()
        .for_each(|item| collect(item, parent, out));
    value
        .as_object()
        .into_iter()
        .for_each(|map| collect_object(map, parent, out));
}

fn collect_object(
    map: &Map<String, Value>,
    parent: Option<&str>,
    out: &mut Vec<TreeNode>,
) {
    let field = |key| map.get(key).and_then(Value::as_str);
    let id = field("id");
    out.extend(id.map(|id| TreeNode {
        id: id.to_owned(),
        kind: field("type").map_or(NodeKind::Other, NodeKind::from),
        name: field("name").unwrap_or_default().to_owned(),
        parent_id: field("parent_id").or(parent).map(str::to_owned),
        in_trash: is_trashed(map.get("in_trash")),
    }));
    let next_parent = id.or(parent);
    map.iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "children" | "channels" | "folders" | "items" | "tree"
            )
        })
        .for_each(|(_, child)| collect(child, next_parent, out));
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use crate::domain::{
        node::NodeKind,
        tree::{Value, flatten, parse},
    };

    #[derive(Debug, PartialEq, Eq)]
    enum Fallback {
        Used,
        Unused,
    }

    #[rstest]
    #[case::spec_shape(
        json!({ "workspace": { "id": "w" }, "spaces": [], "channels": [
            { "id": "f", "type": "folder", "name": "Folder", "parent_id": "w", "in_trash": null },
            { "id": "b", "type": "board", "name": "Board", "parent_id": "f", "extra": [1, 2] },
            { "id": "t", "type": "board", "name": "Old", "in_trash": true },
        ]}),
        &[("f", Some("w"), false), ("b", Some("f"), false), ("t", None, true)],
        Fallback::Unused
    )]
    #[case::nested_children(
        json!({ "channels": [
            { "id": "b1", "type": "board", "name": "Board", "in_trash": null },
            { "id": "f1", "type": "folder", "name": "Folder", "children": [
                { "id": "b2", "type": "board", "name": "Nested", "in_trash": "2026-10-21" },
            ]},
        ]}),
        &[("b1", None, false), ("f1", None, false), ("b2", Some("f1"), true)],
        Fallback::Used
    )]
    #[case::top_level_array(
        json!([{ "id": "b1", "type": "board", "name": "X", "children": [
            { "id": "b2", "type": "board", "name": "Y" },
        ]}]),
        &[("b1", None, false), ("b2", Some("b1"), false)],
        Fallback::Used
    )]
    // A `null` name is still the spec's shape, as in the search index.
    #[case::null_name(
        json!({ "channels": [{ "id": "b", "type": "board", "name": null }] }),
        &[("b", None, false)],
        Fallback::Unused
    )]
    #[case::empty_channels(json!({ "channels": [] }), &[], Fallback::Unused)]
    #[case::scalar(json!(42), &[], Fallback::Used)]
    fn parse_reads_spec_and_falls_back(
        #[case] tree: Value,
        #[case] expected: &[(&str, Option<&str>, bool)],
        #[case] fallback: Fallback,
    ) {
        let parsed = parse(&tree.to_string());
        let summary = parsed.as_ref().ok().map(|parsed| {
            let nodes: Vec<_> = parsed
                .nodes
                .iter()
                .map(|node| {
                    (node.id.as_str(), node.parent_id.as_deref(), node.in_trash)
                })
                .collect();
            (
                nodes,
                parsed
                    .fallback
                    .as_ref()
                    .map_or(Fallback::Unused, |_| Fallback::Used),
            )
        });
        assert_eq!(summary, Some((expected.to_vec(), fallback)));
    }

    #[rstest]
    #[case::absent(json!({ "id": "b" }), false)]
    #[case::null(json!({ "id": "b", "in_trash": null }), false)]
    #[case::explicit_false(json!({ "id": "b", "in_trash": false }), false)]
    #[case::date(json!({ "id": "b", "in_trash": "2026-10-21" }), true)]
    #[case::explicit_true(json!({ "id": "b", "in_trash": true }), true)]
    fn trash_flag_reads_every_form(#[case] node: Value, #[case] trashed: bool) {
        let spec = parse(&json!({ "channels": [&node] }).to_string());
        let heuristic = flatten(&json!([node]));
        assert_eq!(
            spec.ok().map(|parsed| parsed
                .nodes
                .iter()
                .map(|item| item.in_trash)
                .collect::<Vec<_>>()),
            Some(vec![trashed])
        );
        assert_eq!(
            heuristic
                .iter()
                .map(|item| item.in_trash)
                .collect::<Vec<_>>(),
            [trashed]
        );
    }

    #[test]
    fn unknown_or_missing_type_is_other() {
        let parsed = parse(r#"{"channels":[{"id":"a","type":"sticker"},{"id":"b"}]}"#);
        assert_eq!(
            parsed.ok().map(|parsed| parsed
                .nodes
                .iter()
                .map(|node| node.kind)
                .collect::<Vec<_>>()),
            Some(vec![NodeKind::Other, NodeKind::Other])
        );
    }

    #[test]
    fn fallback_says_why() {
        let nested = parse(r#"{"channels":[{"id":"a","children":[]}]}"#);
        assert_eq!(
            nested
                .ok()
                .and_then(|parsed| parsed.fallback)
                .map(|why| why.to_string()),
            Some("channels contain nested nodes".to_owned())
        );
    }

    #[test]
    fn not_json_is_an_error() {
        assert!(parse("{nope").is_err());
    }
}
