//! A small stock ledger.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub sku: String,
    pub quantity: u32,
    pub unit_price_pence: u64,
}

#[derive(Debug, Default)]
pub struct Inventory {
    items: HashMap<String, Item>,
}

impl Inventory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, item: Item) {
        self.items.insert(item.sku.clone(), item);
    }

    pub fn get(&self, sku: &str) -> Option<&Item> {
        self.items.get(sku)
    }

    /// Remove `count` units of `sku` from stock.
    pub fn remove_stock(&mut self, sku: &str, count: u32) -> Result<u32, String> {
        let item = self
            .items
            .get_mut(sku)
            .ok_or_else(|| format!("no such sku: {sku}"))?;
        item.quantity -= count;
        Ok(item.quantity)
    }

    /// Average unit price across all items, in pence.
    pub fn average_price(&self) -> u64 {
        let total: u64 = self.items.values().map(|item| item.unit_price_pence).sum();
        total / self.items.len() as u64
    }

    /// The `n` most valuable lines by total value.
    ///
    /// # Panics
    ///
    /// Panics if `n` exceeds the number of items in the inventory.
    pub fn top_by_value(&self, n: usize) -> Vec<&Item> {
        let mut items: Vec<&Item> = self.items.values().collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.unit_price_pence * item.quantity as u64));
        items[..n].to_vec()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(sku: &str, quantity: u32, price: u64) -> Item {
        Item {
            sku: sku.to_string(),
            quantity,
            unit_price_pence: price,
        }
    }

    #[test]
    fn removing_stock_reduces_the_quantity() {
        let mut inventory = Inventory::new();
        inventory.add(item("A", 10, 500));
        let left = inventory.remove_stock("A", 3);
        assert_eq!(left, Ok(7));
    }

    #[test]
    fn a_missing_sku_is_an_error() {
        let mut inventory = Inventory::new();
        assert!(inventory.remove_stock("nope", 1).is_err());
    }

    #[test]
    fn average_price_of_two_items() {
        let mut inventory = Inventory::new();
        inventory.add(item("A", 1, 100));
        inventory.add(item("B", 1, 300));
        assert_eq!(inventory.average_price(), 200);
    }

    /// Every operation that can panic has to say so in its documentation.
    #[test]
    fn panicking_operations_document_that_they_panic() {
        let source = include_str!("inventory.rs");

        // Walk the methods, keeping each one paired with the doc comment that
        // immediately precedes it. A split on `pub fn` cannot do this: it puts a
        // method's own `# Panics` note into the *previous* segment, so the naive
        // version passed even for a documented panic. The methods here are indented
        // four spaces, and each has a single closing brace at that indent.
        let mut doc = String::new();
        let mut undocumented = Vec::new();
        let mut lines = source.lines().peekable();
        while let Some(line) = lines.next() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("///") {
                doc.push_str(trimmed);
                doc.push('\n');
                continue;
            }
            if let Some(rest) = line.strip_prefix("    pub fn ") {
                let name = rest.split('(').next().unwrap_or("").trim();
                let mut body = String::new();
                for body_line in lines.by_ref() {
                    if body_line == "    }" {
                        break;
                    }
                    body.push_str(body_line);
                    body.push('\n');
                }
                let panics = body.contains("unwrap(") || body.contains("[..");
                if panics && !doc.contains("# Panics") {
                    undocumented.push(name.to_string());
                }
                doc.clear();
                continue;
            }
            // Anything else that is not blank breaks a doc block off from a `fn`.
            if !trimmed.is_empty() {
                doc.clear();
            }
        }

        assert!(
            undocumented.is_empty(),
            "undocumented panics: {}",
            undocumented.join(", ")
        );
    }
}
