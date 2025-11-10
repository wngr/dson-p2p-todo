// ABOUTME: Todo item representation and CRDT operations.
// ABOUTME: Handles reading todos from the CRDT store.

use crate::priority::DotKey;
use dson::{
    Dot, OrMap,
    crdts::{mvreg::MvRegValue, snapshot::ToValue},
};

/// Todo item read from CRDT.
/// Fields may have multiple concurrent values due to conflicts.
#[derive(Debug, Clone, PartialEq)]
pub struct Todo {
    pub dot: Dot,
    pub text: Vec<String>,
    pub done: Vec<bool>,
}

impl Todo {
    /// Check if this todo has any conflicts.
    pub fn has_conflicts(&self) -> bool {
        self.text.len() > 1 || self.done.len() > 1
    }

    /// Get primary text value (first one).
    pub fn primary_text(&self) -> &str {
        self.text.first().map(|s| s.as_str()).unwrap_or("")
    }

    /// Get primary done value (first one).
    pub fn primary_done(&self) -> bool {
        self.done.first().copied().unwrap_or(false)
    }
}

/// Read a todo from the store by its dot.
/// Returns None if the todo doesn't exist.
pub fn read_todo(store: &OrMap<String>, dot: &Dot) -> Option<Todo> {
    let dot_key = DotKey::new(dot);

    // Get the nested map for this todo
    let todo_map = &store.get(dot_key.as_str())?.map;

    // Extract text field (handle multi-value)
    let text = extract_string_values(todo_map, "text");

    // Extract done field (handle multi-value)
    let done = extract_bool_values(todo_map, "done");

    Some(Todo {
        dot: *dot,
        text,
        done,
    })
}

// DEMO BEGIN #4: Conflict extraction - DSON's multi-value registers
/// Extract all string values from a register field.
/// Handles both single-value and multi-value (conflict) cases.
fn extract_string_values(map: &dson::OrMap<String>, key: &str) -> Vec<String> {
    let field = match map.get(&key.to_string()) {
        Some(f) => f,
        None => return Vec::new(),
    };

    // Try single value first (common case)
    if let Ok(MvRegValue::String(s)) = field.reg.value() {
        return vec![s.clone()];
    }

    // Multi-value case - DSON preserves ALL concurrent writes
    field
        .reg
        .values()
        .into_iter()
        .filter_map(|v| match v {
            MvRegValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}
// DEMO END #4

/// Extract all bool values from a register field.
fn extract_bool_values(map: &dson::OrMap<String>, key: &str) -> Vec<bool> {
    let field = match map.get(&key.to_string()) {
        Some(f) => f,
        None => return Vec::new(),
    };

    // Try single value first
    if let Ok(MvRegValue::Bool(b)) = field.reg.value() {
        return vec![*b];
    }

    // Multi-value case
    field
        .reg
        .values()
        .into_iter()
        .filter_map(|v| match v {
            MvRegValue::Bool(b) => Some(*b),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dson::crdts::mvreg::MvRegValue;
    use dson::{CausalDotStore, Identifier, OrMap};

    type TodoStore = CausalDotStore<OrMap<String>>;

    #[test]
    fn test_read_nonexistent_todo() {
        let store = TodoStore::default();
        let id = Identifier::new(1, 0);

        let result = read_todo(&store.store, &Dot::mint(id, 1));
        assert!(result.is_none());
    }

    #[test]
    fn test_read_todo_single_values() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);
        let dot = Dot::mint(id, 1);
        let dot_key = DotKey::new(&dot);

        // Write a todo
        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Buy milk".to_string()));
                todo_tx.write_register("done", MvRegValue::Bool(false));
            });
            let _delta = tx.commit();
        }

        // Read it back
        let todo = read_todo(&store.store, &dot).expect("Todo should exist");

        assert_eq!(todo.dot, dot);
        assert_eq!(todo.text, vec!["Buy milk".to_string()]);
        assert_eq!(todo.done, vec![false]);
        assert!(!todo.has_conflicts());
    }

    #[test]
    fn test_read_todo_with_text_conflict() {
        let mut replica_a = TodoStore::default();
        let mut replica_b = TodoStore::default();

        let id_a = Identifier::new(1, 0);
        let id_b = Identifier::new(2, 0);
        let dot = Dot::mint(id_a, 1);
        let dot_key = DotKey::new(&dot);

        // Both replicas start with same todo
        let delta_init = {
            let mut tx = replica_a.transact(id_a);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Buy milk".to_string()));
                todo_tx.write_register("done", MvRegValue::Bool(false));
            });
            tx.commit()
        };

        replica_a.join_or_replace_with(delta_init.0.store.clone(), &delta_init.0.context);
        replica_b.join_or_replace_with(delta_init.0.store, &delta_init.0.context);

        // Replica A edits text to "Buy whole milk"
        let delta_a = {
            let mut tx = replica_a.transact(id_a);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Buy whole milk".to_string()));
            });
            tx.commit()
        };

        // Replica B concurrently edits text to "Buy oat milk"
        let delta_b = {
            let mut tx = replica_b.transact(id_b);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Buy oat milk".to_string()));
            });
            tx.commit()
        };

        // Exchange deltas - both replicas converge
        replica_a.join_or_replace_with(delta_b.0.store.clone(), &delta_b.0.context);
        replica_b.join_or_replace_with(delta_a.0.store, &delta_a.0.context);

        // Both should see the conflict
        let todo_a = read_todo(&replica_a.store, &dot).expect("Todo should exist");

        assert_eq!(todo_a.dot, dot);
        assert_eq!(todo_a.text.len(), 2);
        assert!(todo_a.text.contains(&"Buy whole milk".to_string()));
        assert!(todo_a.text.contains(&"Buy oat milk".to_string()));
        assert_eq!(todo_a.done, vec![false]);
        assert!(todo_a.has_conflicts());

        // Verify convergence
        assert_eq!(replica_a, replica_b);
    }

    #[test]
    fn test_create_todo_inline() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);
        let dot = Dot::mint(id, 1);
        let dot_key = DotKey::new(&dot);

        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Test todo".to_string()));
                todo_tx.write_register("done", MvRegValue::Bool(false));
            });
            let _delta = tx.commit();
        }

        let todo = read_todo(&store.store, &dot).expect("Todo should exist");

        assert_eq!(todo.text, vec!["Test todo".to_string()]);
        assert_eq!(todo.done, vec![false]);
    }

    #[test]
    fn test_update_text_inline() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);
        let dot = Dot::mint(id, 1);
        let dot_key = DotKey::new(&dot);

        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Original".to_string()));
                todo_tx.write_register("done", MvRegValue::Bool(false));
            });
            let _delta = tx.commit();
        }

        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Updated".to_string()));
            });
            let _delta = tx.commit();
        }

        let todo = read_todo(&store.store, &dot).expect("Todo should exist");

        assert_eq!(todo.text, vec!["Updated".to_string()]);
    }

    #[test]
    fn test_set_done_inline() {
        let mut store = TodoStore::default();
        let id = Identifier::new(1, 0);
        let dot = Dot::mint(id, 1);
        let dot_key = DotKey::new(&dot);

        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("text", MvRegValue::String("Test".to_string()));
                todo_tx.write_register("done", MvRegValue::Bool(false));
            });
            let _delta = tx.commit();
        }

        {
            let mut tx = store.transact(id);
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register("done", MvRegValue::Bool(true));
            });
            let _delta = tx.commit();
        }

        let todo = read_todo(&store.store, &dot).expect("Todo should exist");

        assert_eq!(todo.done, vec![true]);
    }
}
