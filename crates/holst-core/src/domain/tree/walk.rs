//! A workspace tree walked depth first in sidebar order; no I/O.

use itertools::Itertools;

use crate::domain::tree::name_index::build::{NameIndex, NodeIndex};

/// Whether a walk goes through nodes in the trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrashWalk {
    #[default]
    Hidden,
    Shown,
}

struct Child {
    parent: usize,
    node: NodeIndex,
}

struct Siblings<'a> {
    index: &'a NameIndex,
    by_parent: Vec<Child>,
}

impl<'a> Siblings<'a> {
    fn new(index: &'a NameIndex, trashed: TrashWalk) -> Self {
        let top = index.len();
        let by_parent = index
            .nodes()
            .filter(|&node| trashed == TrashWalk::Shown || !index.in_trash(node))
            .map(|node| Child {
                parent: index.parent(node).map_or(top, usize::from),
                node,
            })
            .sorted_by(|left, right| {
                left.parent
                    .cmp(&right.parent)
                    .then_with(|| {
                        index.order(left.node).total_cmp(&index.order(right.node))
                    })
                    .then_with(|| index.name(left.node).cmp(index.name(right.node)))
            })
            .collect();
        Self { index, by_parent }
    }

    fn of(&self, node: usize) -> impl DoubleEndedIterator<Item = NodeIndex> + '_ {
        let from = self.by_parent.partition_point(|child| child.parent < node);
        let to = self.by_parent.partition_point(|child| child.parent <= node);
        self.by_parent
            .get(from..to)
            .unwrap_or_default()
            .iter()
            .map(|child| child.node)
    }

    fn visits(&self, root: Option<NodeIndex>) -> Vec<(NodeIndex, usize)> {
        let start = root.map_or(self.index.len(), usize::from);
        // imperative: the stack is the state of the walk.
        let mut stack: Vec<(NodeIndex, usize)> =
            self.of(start).rev().map(|node| (node, 0)).collect();
        // imperative: `seen` is the state of the walk.
        let mut seen = vec![false; self.index.len()];
        std::iter::from_fn(|| {
            let (node, depth) = stack.pop()?;
            let fresh = seen
                .get_mut(usize::from(node))
                .is_some_and(|seen| !std::mem::replace(seen, true));
            let deeper = fresh.then(|| {
                self.of(usize::from(node))
                    .rev()
                    .map(move |child| (child, depth + 1))
            });
            stack.extend(deeper.into_iter().flatten());
            Some(fresh.then_some((node, depth)))
        })
        .flatten()
        .collect()
    }
}

impl NameIndex {
    /// The nodes under `root` (the top when `None`) with depth, as the sidebar.
    #[must_use]
    pub fn walk(
        &self,
        root: Option<NodeIndex>,
        trashed: TrashWalk,
    ) -> Vec<(NodeIndex, usize)> {
        Siblings::new(self, trashed).visits(root)
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

    use serde_json::{Value, json};

    use crate::domain::tree::{
        name_index::build::{NameIndex, NodeIndex},
        walk::TrashWalk,
    };

    fn id(ordinal: u32) -> String {
        format!("00000001-0000-4000-8000-{ordinal:012x}")
    }

    fn index(channels: &Value) -> crate::Result<NameIndex> {
        NameIndex::from_tree_json(&json!({ "channels": channels }).to_string())
    }

    fn shown(
        index: &NameIndex,
        root: Option<NodeIndex>,
        trashed: TrashWalk,
    ) -> Vec<String> {
        index
            .walk(root, trashed)
            .into_iter()
            .map(|(node, depth)| format!("{}{}", "  ".repeat(depth), index.name(node)))
            .collect()
    }

    #[test]
    fn children_follow_their_folder_in_sidebar_order() -> crate::Result<()> {
        let index = index(&json!([
            { "id": id(1), "type": "folder", "name": "B folder", "order_index": 2.0 },
            { "id": id(2), "type": "board", "name": "Inside", "parent_id": id(1), "order_index": 1.0 },
            { "id": id(3), "type": "board", "name": "A board", "order_index": 1.0 },
            { "id": id(4), "type": "board", "name": "Deleted", "in_trash": true },
            { "id": id(5), "type": "folder", "name": "Old", "in_trash": true },
            { "id": id(6), "type": "board", "name": "Under old", "parent_id": id(5) },
        ]))?;
        assert_eq!(
            shown(&index, None, TrashWalk::Hidden),
            ["A board", "B folder", "  Inside"]
        );
        assert_eq!(
            shown(&index, None, TrashWalk::Shown),
            [
                "A board",
                "B folder",
                "  Inside",
                "Deleted",
                "Old",
                "  Under old"
            ]
        );
        assert_eq!(
            shown(&index, index.find_id(&id(1)), TrashWalk::Hidden),
            ["Inside"]
        );
        Ok(())
    }

    #[test]
    fn a_cycle_is_cut() -> crate::Result<()> {
        let index = index(&json!([
            { "id": id(1), "type": "folder", "name": "A", "parent_id": id(2) },
            { "id": id(2), "type": "folder", "name": "B", "parent_id": id(1) },
        ]))?;
        assert_eq!(shown(&index, None, TrashWalk::Shown), Vec::<String>::new());
        assert_eq!(
            shown(&index, index.find_id(&id(1)), TrashWalk::Shown),
            ["B", "  A"]
        );
        Ok(())
    }
}
