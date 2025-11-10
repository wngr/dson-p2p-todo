# P2P Todo List - DSON CRDT Demo

Terminal-based collaborative todo list demonstrating delta-state CRDT synchronization.
Multiple instances communicate via UDP broadcast, automatically syncing changes and
preserving concurrent edits as multi-value conflicts.

![Demo](demo.gif)

## Quick Start

All instances must use the same port (default 7878). Run multiple terminals:

```bash
cargo run    # Terminal 1
cargo run    # Terminal 2
cargo run    # Terminal 3
```

## Keyboard Controls

- `q` - Quit
- `i` - Add todo
- `Enter` - Edit todo
- `Space` - Toggle done
- `d` - Delete todo
- `j/k` - Navigate
- `J/K` - Change priority
- `↑/↓` - Scroll logs
- `p` - Toggle isolation
- `r` - Add sample todos

## Architecture

### Data Model

```text
CausalDotStore<OrMap<String>>
  ├─ "{replica_id}:{counter}" → OrMap
  │    ├─ "text" → MvReg<String>
  │    └─ "done" → MvReg<Bool>
  └─ "priority" → OrArray
       └─ ["{replica_id}:{counter}", ...]
```

### CRDT Types

- **OrMap** - Observed-remove map
- **MvReg** - Multi-value register (preserves concurrent writes)
- **OrArray** - Ordered list

### Network

- UDP broadcast to 255.255.255.255
- SO_REUSEPORT enables multiple instances on one port
- Delta-based sync broadcasts minimal changes
- Anti-entropy broadcasts context every 10s

## Observing CRDTs

### Concurrent Edits

1. Add todo: "Buy milk"
2. Edit simultaneously in two instances:
   - Instance 1: "Buy whole milk"
   - Instance 2: "Buy oat milk"
3. Both show: `⚠ [Buy whole milk, Buy oat milk]`

The system preserves conflicts, not resolves them.

### Network Partitions

1. Start two instances
2. Press `p` to isolate instance 1
3. Make changes in both
4. Press `p` to reconnect
5. Anti-entropy merges state automatically

### Priority Conflicts

Concurrent reordering may interleave, but replicas converge.

## Implementation

- Each replica gets an 8-bit ID from the timestamp
- Todos use dot encoding: `"{replica_id}:{counter}"`
- Transactions provide read-committed isolation
- Logs use 6 colors, cycling by replica ID

## File Organization

- `main.rs` - Event loop and terminal setup
- `app.rs` - Application state and sync logic
- `todo.rs` - Todo CRDT operations
- `priority.rs` - Priority array management
- `network.rs` - UDP broadcast and serialization
- `ui.rs` - Terminal rendering (ratatui)
- `input.rs` - Keyboard handling
- `anti_entropy.rs` - Partition recovery protocol
