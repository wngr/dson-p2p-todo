// ABOUTME: Application state management and network synchronization.
// ABOUTME: Coordinates CRDT store, network layer, and UI state.

use crate::{
    anti_entropy::{AntiEntropy, SyncNeeded},
    network::{self, NetworkMessage},
    todo::Todo,
};
use dson::{CausalDotStore, Dot, Identifier, OrMap};
use std::{io, net::UdpSocket};

type TodoStore = CausalDotStore<OrMap<String>>;

/// Unique identifier for a replica, derived from timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ReplicaId(u8);

impl ReplicaId {
    #[allow(unused)]
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

/// Star Wars themed sample todos.
const SAMPLE_TODOS: &[&str] = &[
    "Train with the Jedi master",
    "Fix the spaceship engine",
    "Deliver secret plans to the rebels",
    "Practice with the laser sword",
    "Rescue the princess from the space station",
    "Disable the tractor beam",
    "Navigate through the asteroid field",
    "Escape the trash compactor",
    "Attend the galactic senate meeting",
    "Learn to use the Force",
    "Repair the robot companion",
    "Complete the smuggling run",
    "Establish rebel base on ice planet",
    "Find a good cantina for drinks",
    "Evade the Empire's warships",
    "Study ancient galactic history",
    "Upgrade the starfighter weapons",
    "Negotiate with space gangsters",
    "Investigate the mysterious energy field",
    "Defrost friend from carbonite",
    "Sabotage the giant walking tanks",
    "Recruit pilots for the rebellion",
    "Destroy the moon-sized weapon",
    "Train the new generation of heroes",
    "Explore the desert planet",
    "Meet with the galactic emperor",
    "Hide from the bounty hunters",
    "Build a new lightsaber",
    "Convince the smuggler to help",
    "Stop the evil empire's plans",
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
    pub fn toggle_isolation(&mut self) -> io::Result<()> {
        self.network_isolated = !self.network_isolated;
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
        network::broadcast(&self.socket, &data, self.port, self.network_isolated)?;
        self.log(format!(
            "[Replica {}] Broadcast delta: {} bytes (isolated: {})",
            self.replica_id,
            data.len(),
            self.network_isolated
        ));
        Ok(())
    }

    /// Broadcast our causal context for anti-entropy.
    fn broadcast_context(&mut self) -> io::Result<()> {
        let msg = NetworkMessage::Context {
            sender_id: self.replica_id,
            context: self.store.context.clone(),
        };

        let data = network::serialize_message(&msg)?;
        network::broadcast(&self.socket, &data, self.port, self.network_isolated)?;
        self.log(format!(
            "[Replica {}] Broadcast context: {} bytes",
            self.replica_id,
            data.len()
        ));
        Ok(())
    }

    /// Process all incoming messages from the network.
    /// Returns the number of deltas processed.
    pub fn process_incoming_deltas(&mut self) -> io::Result<usize> {
        let mut count = 0;

        while let Some((data, addr)) = network::try_receive(&self.socket, self.network_isolated)? {
            match network::deserialize_message(&data) {
                Ok(msg) => {
                    if msg.sender_id() == self.replica_id {
                        continue; // Ignore own messages
                    }

                    self.log(format!(
                        "[Replica {}] Received {} bytes from {}",
                        msg.sender_id(),
                        data.len(),
                        addr
                    ));

                    match msg {
                        NetworkMessage::Delta { sender_id, delta } => {
                            self.log(format!(
                                "[Replica {}] Received delta: {} bytes",
                                sender_id,
                                data.len()
                            ));
                            self.store
                                .join_or_replace_with(delta.0.store, &delta.0.context);
                            count += 1;
                            self.log(format!("[Replica {}] Applied delta", sender_id));
                        }
                        NetworkMessage::Context { sender_id, context } => {
                            self.log(format!(
                                "[Replica {}] Received context: {} bytes",
                                sender_id,
                                data.len()
                            ));

                            // Compare contexts and decide what to do
                            let sync_needed =
                                AntiEntropy::compare_contexts(&self.store.context, &context);
                            match sync_needed {
                                SyncNeeded::InSync => {
                                    self.log(format!("[Replica {}] Already in sync", sender_id));
                                }
                                SyncNeeded::RemoteNeedsSync | SyncNeeded::BothNeedSync => {
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
                                    self.log(format!(
                                        "[Replica {}] Needs sync, sent full state: {} bytes",
                                        sender_id,
                                        data.len()
                                    ));
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
        use rand::{seq::SliceRandom, thread_rng};

        // Pick 3 unique random todos
        let mut rng = thread_rng();
        let selected: Vec<_> = SAMPLE_TODOS.choose_multiple(&mut rng, 3).collect();

        // Add the todos
        for text in selected.iter() {
            let (dot_key, _dot) = self.next_dot_key();
            let mut tx = self.store.transact(self.identifier());

            // Create the todo with text and done fields
            tx.in_map(dot_key.as_str(), |todo_tx| {
                todo_tx.write_register(
                    "text",
                    dson::crdts::mvreg::MvRegValue::String(text.to_string()),
                );
                todo_tx.write_register("done", dson::crdts::mvreg::MvRegValue::Bool(false));
            });

            // Add to priority array at the end
            tx.in_array("priority", |arr_tx| {
                arr_tx.insert_register(
                    arr_tx.len(),
                    dson::crdts::mvreg::MvRegValue::String(dot_key.into_inner()),
                );
            });

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
