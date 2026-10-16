//! Module root of the IR.

use crate::item::Item;

/// A compiled Python module: the root IR node both backends consume.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Top-level items (defs and bare statements), in source order.
    pub items: Vec<Item>,
}

impl Module {
    /// An empty module.
    pub fn new() -> Self {
        Module { items: Vec::new() }
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}
