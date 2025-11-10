// ABOUTME: Anti-entropy protocol for delta CRDT synchronization.
// ABOUTME: Periodically exchanges causal contexts to detect and repair missing deltas.

use dson::CausalContext;
use std::time::{Duration, Instant};

/// Anti-entropy configuration and state.
pub struct AntiEntropy {
    /// How often to broadcast our causal context
    pub interval: Duration,
    /// Last time we sent our context
    last_broadcast: Instant,
}

/// Default anti-entropy broadcast interval.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

impl Default for AntiEntropy {
    fn default() -> Self {
        Self::new(DEFAULT_INTERVAL)
    }
}

impl AntiEntropy {
    /// Create a new anti-entropy instance with the given broadcast interval.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_broadcast: Instant::now(),
        }
    }

    /// Check if it's time to broadcast our causal context.
    /// Returns true if the interval has elapsed since the last broadcast.
    pub fn should_broadcast(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_broadcast) >= self.interval {
            self.last_broadcast = now;
            true
        } else {
            false
        }
    }

    // DEMO BEGIN #5: Anti-entropy via causal context comparison
    /// Compare two causal contexts to determine if one is behind the other.
    /// Returns SyncNeeded indicating what action should be taken.
    pub fn compare_contexts(local: &CausalContext, remote: &CausalContext) -> SyncNeeded {
        use std::cmp::Ordering;

        match local.partial_cmp(remote) {
            Some(Ordering::Equal) => SyncNeeded::InSync,
            Some(Ordering::Greater) => SyncNeeded::RemoteNeedsSync,
            Some(Ordering::Less) => SyncNeeded::LocalNeedsSync,
            None => SyncNeeded::BothNeedSync, // Concurrent divergence
        }
    }
    // DEMO END #5
}

/// Result of comparing two causal contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum SyncNeeded {
    /// Both replicas are in sync (same version vector).
    InSync,
    /// Remote replica is missing operations we have.
    RemoteNeedsSync,
    /// Local replica is missing operations the remote has.
    LocalNeedsSync,
    /// Both replicas have operations the other doesn't (concurrent updates during partition).
    BothNeedSync,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dson::crdts::mvreg::MvRegValue;
    use dson::{CausalDotStore, Identifier, OrMap};

    type TodoStore = CausalDotStore<OrMap<String>>;

    #[test]
    fn test_should_broadcast() {
        let mut ae = AntiEntropy::new(Duration::from_millis(100));

        // Should not broadcast immediately after creation
        assert!(!ae.should_broadcast());

        // Sleep and check again
        std::thread::sleep(Duration::from_millis(150));
        assert!(ae.should_broadcast());

        // Should not broadcast again immediately
        assert!(!ae.should_broadcast());
    }

    #[test]
    fn test_compare_contexts_in_sync() {
        let mut store_a = TodoStore::default();
        let mut store_b = TodoStore::default();

        let id_a = Identifier::new(1, 0);

        // Both create the same operation
        let delta = {
            let mut tx = store_a.transact(id_a);
            tx.write_register("key", MvRegValue::String("value".to_string()));
            tx.commit()
        };

        store_a.join_or_replace_with(delta.0.store.clone(), &delta.0.context);
        store_b.join_or_replace_with(delta.0.store, &delta.0.context);

        let result = AntiEntropy::compare_contexts(&store_a.context, &store_b.context);
        assert_eq!(result, SyncNeeded::InSync);
    }

    #[test]
    fn test_compare_contexts_remote_behind() {
        let mut store_a = TodoStore::default();
        let store_b = TodoStore::default();
        let id_a = Identifier::new(1, 0);

        // Store A has an operation
        let delta = {
            let mut tx = store_a.transact(id_a);
            tx.write_register("key", MvRegValue::String("value".to_string()));
            tx.commit()
        };
        store_a.join_or_replace_with(delta.0.store, &delta.0.context);

        // Store B is empty
        let result = AntiEntropy::compare_contexts(&store_a.context, &store_b.context);
        assert_eq!(result, SyncNeeded::RemoteNeedsSync);
    }

    #[test]
    fn test_compare_contexts_local_behind() {
        let store_a = TodoStore::default();
        let mut store_b = TodoStore::default();
        let id_b = Identifier::new(2, 0);

        // Store B has an operation
        let delta = {
            let mut tx = store_b.transact(id_b);
            tx.write_register("key", MvRegValue::String("value".to_string()));
            tx.commit()
        };
        store_b.join_or_replace_with(delta.0.store, &delta.0.context);

        // Store A is empty
        let result = AntiEntropy::compare_contexts(&store_a.context, &store_b.context);
        assert_eq!(result, SyncNeeded::LocalNeedsSync);
    }

    #[test]
    fn test_compare_contexts_both_need_sync() {
        let mut store_a = TodoStore::default();
        let mut store_b = TodoStore::default();

        let id_a = Identifier::new(1, 0);
        let id_b = Identifier::new(2, 0);

        // Store A has an operation
        let delta_a = {
            let mut tx = store_a.transact(id_a);
            tx.write_register("key_a", MvRegValue::String("value_a".to_string()));
            tx.commit()
        };
        store_a.join_or_replace_with(delta_a.0.store, &delta_a.0.context);

        // Store B has a different operation
        let delta_b = {
            let mut tx = store_b.transact(id_b);
            tx.write_register("key_b", MvRegValue::String("value_b".to_string()));
            tx.commit()
        };
        store_b.join_or_replace_with(delta_b.0.store, &delta_b.0.context);

        // Both have operations the other doesn't
        let result = AntiEntropy::compare_contexts(&store_a.context, &store_b.context);
        assert_eq!(result, SyncNeeded::BothNeedSync);
    }
}
