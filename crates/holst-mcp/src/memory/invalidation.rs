//! Invalidation: what a write reaches, and guards that invalidate on drop.

use std::sync::atomic::Ordering;

use crate::memory::{McpMemory, lock};

pub(crate) struct FullInvalidation<'a>(pub(crate) &'a McpMemory);

impl Drop for FullInvalidation<'_> {
    fn drop(&mut self) {
        self.0.invalidate_all();
    }
}

impl McpMemory {
    pub(crate) fn invalidate_all(&self) {
        let generation = self.next_generation();
        self.workspaces_floor.store(generation, Ordering::Release);
        self.workspaces.store(None);
        lock(&self.trees).forget_all(generation);
        lock(&self.boards).forget_all(generation);
    }
}
