//! Answers as text for the model: compacted data, tree pages, list lines.

use serde_json::Value;

use crate::memory::McpMemory;

impl McpMemory {
    pub(crate) fn render(&self, value: Value) -> String {
        crate::reply::compact::render(value, &|id| self.alias(id))
    }
}
