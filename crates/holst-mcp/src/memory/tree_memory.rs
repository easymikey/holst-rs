//! Kept workspaces and name trees: trust window, generation floors, eviction.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use holst_core::domain::{account::Workspace, tree::name_index::build::NameIndex};
use itertools::Itertools as _;

pub(super) const TREE_TRUST_WINDOW: Duration = Duration::from_mins(1);
pub(super) const TREE_MEMORY_BYTES: u64 = 64 << 20;

pub(super) struct KeptWorkspaces {
    pub(super) generation: u64,
    pub(super) at: Instant,
    pub(super) list: Arc<[Workspace]>,
}

pub(super) struct KeptTree {
    pub(super) generation: u64,
    pub(super) at: Instant,
    pub(super) used: u64,
    pub(super) weight: u64,
    pub(super) index: Arc<NameIndex>,
}

impl KeptTree {
    fn is_trusted(&self, floor: u64) -> bool {
        self.generation >= floor && self.at.elapsed() < TREE_TRUST_WINDOW
    }
}

pub(super) fn heap_weight(index: &NameIndex) -> u64 {
    let names: usize = index.nodes().map(|node| index.name(node).len() + 1).sum();
    u64::try_from(index.len() * 38 + names * 2)
        .unwrap_or(u64::MAX)
        .max(1)
}

#[derive(Default)]
pub(super) struct TreeMemory {
    pub(super) trees: HashMap<String, KeptTree>,
    pub(super) floor: u64,
    pub(super) tick: u64,
}

impl TreeMemory {
    pub(super) fn get(&mut self, workspace_id: &str) -> Option<Arc<NameIndex>> {
        let floor = self.floor;
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        self.trees
            .get_mut(workspace_id)
            .filter(|tree| tree.is_trusted(floor))
            .map(|tree| {
                tree.used = tick;
                Arc::clone(&tree.index)
            })
    }

    pub(super) fn insert(
        &mut self,
        workspace_id: &str,
        mut tree: KeptTree,
        capacity: u64,
    ) {
        self.tick = self.tick.wrapping_add(1);
        tree.used = self.tick;
        let floor = self.floor;
        if tree.is_trusted(floor) {
            match tree.weight {
                heavy if heavy > capacity => drop(self.trees.remove(workspace_id)),
                _ => {
                    self.trees.retain(|_, old| old.is_trusted(floor));
                    drop(self.trees.insert(workspace_id.to_string(), tree));
                    self.evict(workspace_id, capacity);
                }
            }
        }
    }

    pub(super) fn forget_all(&mut self, generation: u64) {
        self.trees.clear();
        self.floor = generation;
    }

    fn evict(&mut self, keep: &str, capacity: u64) {
        let total: u64 = self.trees.values().map(|tree| tree.weight).sum();
        let oldest_first: Vec<(u64, u64, String)> = self
            .trees
            .iter()
            .filter(|(id, _)| id.as_str() != keep)
            .map(|(id, tree)| (tree.used, tree.weight, id.clone()))
            .sorted_unstable_by_key(|&(used, ..)| used)
            .collect();
        oldest_first
            .into_iter()
            .scan(total, |total, (_, weight, id)| {
                (*total > capacity).then(|| {
                    *total = total.saturating_sub(weight);
                    id
                })
            })
            .for_each(|id| drop(self.trees.remove(&id)));
    }
}
