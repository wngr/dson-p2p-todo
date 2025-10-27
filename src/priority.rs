// ABOUTME: Priority array management using OrArray.
// ABOUTME: Maintains ordered list of todo dots for display.

use dson::crdts::{mvreg::MvRegValue, snapshot::ToValue};
use dson::transaction::MapTransaction;
use dson::{Dot, OrMap};

const PRIORITY_KEY: &str = "priority";

/// Unique identifier for a todo, encoded as "{replica_id}:{counter}".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DotKey(String);

impl DotKey {
    /// Create a DotKey from a Dot.
    pub fn new(dot: &Dot) -> Self {
        Self(format!(
            "{}:{}",
            dot.actor().node().value(),
            dot.sequence().get()
        ))
    }

    /// Parse a DotKey string back into a Dot.
    ///
    /// # Errors
    /// Returns `None` if the format is not "node_id:counter" or if
    /// either component is not a valid u64.
    pub fn parse(&self) -> Option<Dot> {
        let parts: Vec<&str> = self.0.split(':').collect();
        if parts.len() != 2 {
            return None;
        }
        let node_id = parts[0].parse().ok()?;
        let counter = parts[1].parse().ok()?;
        Some(Dot::mint(dson::Identifier::new(node_id, 0), counter))
    }

    /// Get the string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the DotKey and return the inner String.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for DotKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Read the priority array, returning dots in order.
pub fn read_priority(store: &OrMap<String>) -> Vec<Dot> {
    let priority_field = match store.get(PRIORITY_KEY) {
        Some(field) => &field.array,
        None => return Vec::new(),
    };

    let mut dots = Vec::new();
    for idx in 0..priority_field.len() {
        if let Some(item) = priority_field.get(idx) {
            // Handle both single value and multi-value cases
            if let Ok(MvRegValue::String(dot_str)) = item.reg.value() {
                if let Some(dot) = parse_dot(dot_str) {
                    dots.push(dot);
                }
            } else {
                // Multi-value - take first
                for val in item.reg.values() {
                    if let MvRegValue::String(dot_str) = val
                        && let Some(dot) = parse_dot(dot_str)
                    {
                        dots.push(dot);
                        break; // Only take first
                    }
                }
            }
        }
    }
    dots
}

/// Insert a todo at the given position in priority array.
pub fn insert_at_priority(tx: &mut MapTransaction<String>, dot: &Dot, position: usize) {
    let dot_key = DotKey::new(dot);
    tx.in_array(PRIORITY_KEY, |arr_tx| {
        arr_tx.insert_register(position, MvRegValue::String(dot_key.into_inner()));
    });
}

/// Remove todo at specific index from priority array.
pub fn remove_at_index(tx: &mut MapTransaction<String>, index: usize) {
    tx.in_array(PRIORITY_KEY, |arr_tx| {
        arr_tx.remove(index);
    });
}

/// Find index of a dot in the priority list.
///
/// # Errors
/// Returns `None` if the dot is not found in the priority array.
pub fn find_priority_index(store: &OrMap<String>, dot: &Dot) -> Option<usize> {
    let priority = read_priority(store);
    priority.iter().position(|d| d == dot)
}

/// Parse dot from "node_id:counter" format.
fn parse_dot(s: &str) -> Option<Dot> {
    DotKey(s.to_string()).parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dson::{CausalDotStore, Identifier, OrMap};

    type TodoStore = CausalDotStore<OrMap<String>>;

    #[test]
    fn test_read_empty_priority() {
        let store = TodoStore::default();
        assert_eq!(read_priority(&store.store), Vec::<Dot>::new());
    }

    #[test]
    fn test_insert_and_read_priority() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);

        let dot1 = Dot::mint(id, 1);
        let dot2 = Dot::mint(id, 2);

        {
            let mut tx = store.transact(id);
            insert_at_priority(&mut tx, &dot1, 0);
            insert_at_priority(&mut tx, &dot2, 1);
            let _ = tx.commit();
        }

        let priority = read_priority(&store.store);
        assert_eq!(priority, vec![dot1, dot2]);
    }

    #[test]
    fn test_remove_at_index() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);

        let dot1 = Dot::mint(id, 1);
        let dot2 = Dot::mint(id, 2);
        let dot3 = Dot::mint(id, 3);

        {
            let mut tx = store.transact(id);
            insert_at_priority(&mut tx, &dot1, 0);
            insert_at_priority(&mut tx, &dot2, 1);
            insert_at_priority(&mut tx, &dot3, 2);
            let _ = tx.commit();
        }

        // Verify we have all three items
        {
            let priority = read_priority(&store.store);
            assert_eq!(priority.len(), 3);
        }

        {
            let mut tx = store.transact(id);
            remove_at_index(&mut tx, 1); // Remove middle item
            let _ = tx.commit();
        }

        let priority = read_priority(&store.store);
        // After removing index 1, we should have 2 items
        assert_eq!(priority.len(), 2);
        // First and last should remain
        assert_eq!(priority[0], dot1);
        assert_eq!(priority[1], dot3);
    }

    #[test]
    fn test_find_priority_index() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);

        let dot1 = Dot::mint(id, 1);
        let dot2 = Dot::mint(id, 2);

        {
            let mut tx = store.transact(id);
            insert_at_priority(&mut tx, &dot1, 0);
            insert_at_priority(&mut tx, &dot2, 1);
            let _ = tx.commit();
        }

        assert_eq!(find_priority_index(&store.store, &dot1), Some(0));
        assert_eq!(find_priority_index(&store.store, &dot2), Some(1));
        assert_eq!(
            find_priority_index(&store.store, &Dot::mint(Identifier::new(99, 0), 99)),
            None
        );
    }
}
