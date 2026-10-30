//! Invalidation: what a write reaches, and guards that invalidate on drop.

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
        lock(&self.boards).forget_all(generation);
    }
}
