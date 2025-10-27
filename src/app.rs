// ABOUTME: Application state management and network synchronization.
// ABOUTME: Coordinates CRDT store, network layer, and UI state.

use crate::anti_entropy::{AntiEntropy, SyncNeeded};
use crate::network::{self, NetworkMessage};
use crate::todo::Todo;
use dson::{CausalDotStore, Dot, Identifier, OrMap};
use std::io;
use std::net::UdpSocket;

type TodoStore = CausalDotStore<OrMap<String>>;

/// Unique identifier for a replica, derived from timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ReplicaId(u8);

impl ReplicaId {
    /// Create a new ReplicaId.
    pub fn new(id: u8) -> Self {
        Self(id)
    }

    /// Create a ReplicaId from current timestamp (lower 8 bits).
    pub fn from_timestamp() -> Self {
        let id = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_secs()
            % 256) as u8;
        Self(id)
    }

    /// Get the underlying u8 value.
    pub fn value(self) -> u8 {
        self.0
    }
}

impl std::fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02x}", self.0)
    }
}

/// Maximum number of log messages to keep in the buffer.
const MAX_LOG_MESSAGES: usize = 50;

/// Star Wars themed sample todos
const SAMPLE_TODOS: &[&str] = &[
    "Train with Yoda on Dagobah",
    "Repair the hyperdrive motivator",
    "Negotiate trade agreements with the Hutts",
    "Deliver death star plans to Rebellion",
    "Practice lightsaber forms in training room",
    "Calibrate targeting computer for trench run",
    "Schedule inspection of shield generator",
    "Update astromech droid memory banks",
    "Attend Jedi Council meeting",
    "Investigate disturbance in the Force",
    "Stock up on power converters at Tosche Station",
    "Complete Kessel Run in less than 12 parsecs",
    "Escape from trash compactor on detention level",
    "Rescue princess from cell block 1138",
    "Disable tractor beam on battle station",
    "Navigate asteroid field near Hoth",
    "Establish new base on remote ice planet",
    "Recruit smugglers at Mos Eisley cantina",
    "Investigate Imperial presence on Endor",
    "Sabotage AT-AT walker defense systems",
    "Infiltrate Imperial garrison disguised as scout",
    "Reprogram protocol droid for Binary language",
    "Hunt for bounties in Outer Rim territories",
    "Evade Star Destroyer pursuit through nebula",
    "Restore balance to the Force",
    "Negotiate release of carbonite frozen smuggler",
    "Study ancient Jedi texts in temple archives",
    "Upgrade X-wing fighter with proton torpedoes",
    "Attend diplomatic mission to Cloud City",
    "Investigate Sith artifact on Moraband",
];

/// UI state for navigation and interaction.
pub struct UiState {
    pub selected_index: usize,
    pub mode: Mode,
    pub input_buffer: String,
    pub editing_dot: Option<dson::Dot>,
    pub log_scroll: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected_index: 0,
            mode: Mode::Normal,
            input_buffer: String::new(),
            editing_dot: None,
            log_scroll: 0,
        }
    }
}

/// UI modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Normal,
    Insert,
}

/// Main application state.
pub struct App {
    pub replica_id: ReplicaId,
    pub store: TodoStore,
    pub socket: UdpSocket,
    pub network_isolated: bool,
    pub ui_state: UiState,
    pub counter: u16,
    pub port: u16,
    pub log_buffer: Vec<String>,
    pub anti_entropy: AntiEntropy,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("replica_id", &self.replica_id)
            .field("network_isolated", &self.network_isolated)
            .field("counter", &self.counter)
            .field("port", &self.port)
            .field("log_buffer_len", &self.log_buffer.len())
            .finish_non_exhaustive()
    }
}

impl App {
    /// Create a new app instance.
    pub fn new(port: u16) -> io::Result<Self> {
        let replica_id = ReplicaId::from_timestamp();
        let socket = network::create_broadcast_socket(port)?;

        Ok(Self {
            replica_id,
            store: TodoStore::default(),
            socket,
            network_isolated: false,
            ui_state: UiState::default(),
            counter: 0,
            port,
            log_buffer: Vec::new(),
            anti_entropy: AntiEntropy::default(),
        })
    }

    /// Add a log message to the buffer.
    pub fn log(&mut self, msg: String) {
        self.log_buffer.push(msg);
        if self.log_buffer.len() > MAX_LOG_MESSAGES {
            self.log_buffer.remove(0);
        }
    }

    /// Toggle network isolation state.
    /// When switching from isolated to connected, broadcasts current state.
    pub fn toggle_isolation(&mut self) -> io::Result<()> {
        let was_isolated = self.network_isolated;
        self.network_isolated = !self.network_isolated;

        // If we just reconnected, broadcast our entire state
        if was_isolated && !self.network_isolated {
            self.log(format!(
                "[Replica {}] Reconnecting - broadcasting full state",
                self.replica_id
            ));

            // Serialize the entire store and broadcast it
            let msg = NetworkMessage::Delta {
                sender_id: self.replica_id,
                delta: dson::Delta(self.store.clone()),
            };

            let data = network::serialize_message(&msg)?;
            self.log(format!(
                "[Replica {}] Broadcasting full state: {} bytes",
                self.replica_id,
                data.len()
            ));
            network::broadcast(&self.socket, &data, self.port, self.network_isolated)?;
        }

        Ok(())
    }

    /// Get current identifier for transactions.
    /// Uses a fixed application ID (0) - the CRDT handles sequence numbering internally.
    pub fn identifier(&self) -> Identifier {
        Identifier::new(self.replica_id.value(), 0)
    }

    /// Generate and return the next dot key.
    /// This is just for creating unique string keys for todos, not for CRDT operations.
    pub fn next_dot_key(&mut self) -> (crate::priority::DotKey, Dot) {
        self.counter += 1;
        // Create a unique dot just for the key string (not used by CRDT operations)
        let dot = Dot::mint(self.identifier(), self.counter as u64);
        let key = crate::priority::DotKey::new(&dot);
        (key, dot)
    }

    /// Get all todos in priority order.
    pub fn get_todos_ordered(&self) -> Vec<(Dot, Todo)> {
        let priority = crate::priority::read_priority(&self.store.store);

        priority
            .into_iter()
            .filter_map(|dot| {
                crate::todo::read_todo(&self.store.store, &dot).map(|todo| (dot, todo))
            })
            .collect()
    }

    /// Broadcast a delta to all peers.
    pub fn broadcast_delta(&mut self, delta: dson::Delta<TodoStore>) -> io::Result<()> {
        let msg = NetworkMessage::Delta {
            sender_id: self.replica_id,
            delta,
        };

        let data = network::serialize_message(&msg)?;
        self.log(format!(
            "[Replica {}] Broadcasting delta {} bytes (isolated: {})",
            self.replica_id,
            data.len(),
            self.network_isolated
        ));
        network::broadcast(&self.socket, &data, self.port, self.network_isolated)?;
        Ok(())
    }

    /// Broadcast our causal context for anti-entropy.
    fn broadcast_context(&mut self) -> io::Result<()> {
        let msg = NetworkMessage::Context {
            sender_id: self.replica_id,
            context: self.store.context.clone(),
        };

        let data = network::serialize_message(&msg)?;
        self.log(format!(
            "[Replica {}] Broadcasting context for anti-entropy",
            self.replica_id
        ));
        network::broadcast(&self.socket, &data, self.port, self.network_isolated)?;
        Ok(())
    }

    /// Process all incoming messages from the network.
    /// Returns the number of deltas processed.
    pub fn process_incoming_deltas(&mut self) -> io::Result<usize> {
        let mut count = 0;

        while let Some((data, addr)) = network::try_receive(&self.socket, self.network_isolated)? {
            self.log(format!(
                "[Replica {}] Received {} bytes from {}",
                self.replica_id,
                data.len(),
                addr
            ));
            match network::deserialize_message(&data) {
                Ok(msg) => {
                    if msg.sender_id() == self.replica_id {
                        continue; // Ignore own messages
                    }

                    match msg {
                        NetworkMessage::Delta { sender_id, delta } => {
                            self.log(format!("[Replica {}] Received delta", sender_id));
                            self.store
                                .join_or_replace_with(delta.0.store, &delta.0.context);
                            count += 1;
                            self.log(format!("[Replica {}] Applied delta", sender_id));
                        }
                        NetworkMessage::Context { sender_id, context } => {
                            self.log(format!("[Replica {}] Received context", sender_id));

                            // Compare contexts and decide what to do
                            let sync_needed =
                                AntiEntropy::compare_contexts(&self.store.context, &context);
                            match sync_needed {
                                SyncNeeded::InSync => {
                                    self.log(format!("[Replica {}] Already in sync", sender_id));
                                }
                                SyncNeeded::RemoteNeedsSync | SyncNeeded::BothNeedSync => {
                                    self.log(format!(
                                        "[Replica {}] Needs sync, sending full state",
                                        sender_id
                                    ));
                                    // They're missing operations, send our full state
                                    let msg = NetworkMessage::Delta {
                                        sender_id: self.replica_id,
                                        delta: dson::Delta(self.store.clone()),
                                    };
                                    let data = network::serialize_message(&msg)?;
                                    network::broadcast(
                                        &self.socket,
                                        &data,
                                        self.port,
                                        self.network_isolated,
                                    )?;
                                }
                                SyncNeeded::LocalNeedsSync => {
                                    self.log(format!(
                                        "[Replica {}] Has updates for us (waiting for delta)",
                                        sender_id
                                    ));
                                    // We're missing operations - they'll send us their state when they see our context
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    self.log(format!("Failed to deserialize message: {e}"));
                }
            }
        }

        Ok(count)
    }

    /// Called every frame to process network events.
    pub fn tick(&mut self) -> io::Result<()> {
        // Process incoming messages
        self.process_incoming_deltas()?;

        // Check if it's time for anti-entropy broadcast
        if self.anti_entropy.should_broadcast() && !self.network_isolated {
            self.broadcast_context()?;
        }

        Ok(())
    }

    /// Add 3 random Star Wars themed todos to the bottom of the list.
    pub fn add_random_todos(&mut self) -> io::Result<()> {
        use std::collections::HashSet;

        // Use current time as seed for randomness
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos() as usize;

        // Pick 3 unique random indices
        let mut used = HashSet::new();
        let mut selected = Vec::new();

        for i in 0..3 {
            let mut idx = (seed + i * 7919) % SAMPLE_TODOS.len();
            while used.contains(&idx) {
                idx = (idx + 1) % SAMPLE_TODOS.len();
            }
            used.insert(idx);
            selected.push(SAMPLE_TODOS[idx]);
        }

        // Get current priority list length to add at the end
        let priority = crate::priority::read_priority(&self.store.store);
        let insert_position = priority.len();

        // Add the todos
        for (offset, text) in selected.iter().enumerate() {
            let (dot_key, dot) = self.next_dot_key();
            let mut tx = self.store.transact(self.identifier());
            crate::todo::create_todo(&mut tx, &dot_key, text.to_string());
            crate::priority::insert_at_priority(&mut tx, &dot, insert_position + offset);
            let delta = tx.commit();
            self.broadcast_delta(delta)?;
        }

        self.log(format!(
            "[Replica {}] Added 3 random Star Wars todos",
            self.replica_id
        ));
        Ok(())
    }
}
