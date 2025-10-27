// ABOUTME: Keyboard input handling and action execution.
// ABOUTME: Maps key events to app state changes and CRDT operations.

use std::io;
use crossterm::event::{KeyCode, KeyEvent};
use crate::app::{App, Mode};

/// User actions triggered by keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    MovePriorityUp,
    MovePriorityDown,
    ToggleDone,
    Delete,
    EnterInsertMode,
    EnterEditMode,
    ToggleIsolation,
    AddRandomTodos,
    ScrollLogsUp,
    ScrollLogsDown,
}

/// Handle a key event and return the corresponding action.
pub fn handle_key(key: KeyEvent, app: &App) -> Option<Action> {
    match app.ui_state.mode {
        Mode::Normal => handle_normal_mode(key),
        Mode::Insert => None, // Insert mode handled differently
    }
}

/// Handle keys in normal mode.
fn handle_normal_mode(key: KeyEvent) -> Option<Action> {
    use crossterm::event::KeyModifiers;

    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => Some(Action::Quit),
        (KeyCode::Char('j'), KeyModifiers::NONE) => Some(Action::MoveDown),
        (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Action::MoveUp),
        (KeyCode::Char('J'), _) => Some(Action::MovePriorityDown),
        (KeyCode::Char('K'), _) => Some(Action::MovePriorityUp),
        (KeyCode::Char(' '), _) => Some(Action::ToggleDone),
        (KeyCode::Char('d'), _) => Some(Action::Delete),
        (KeyCode::Char('i'), _) => Some(Action::EnterInsertMode),
        (KeyCode::Char('p'), _) => Some(Action::ToggleIsolation),
        (KeyCode::Char('r'), _) => Some(Action::AddRandomTodos),
        (KeyCode::Up, _) => Some(Action::ScrollLogsUp),
        (KeyCode::Down, _) => Some(Action::ScrollLogsDown),
        (KeyCode::Enter, _) => Some(Action::EnterEditMode),
        _ => None,
    }
}

/// Handle keys in insert mode.
pub fn handle_insert_key(key: KeyEvent, app: &mut App) -> io::Result<bool> {
    match key.code {
        KeyCode::Enter => {
            let text = app.ui_state.input_buffer.clone();
            if !text.is_empty() {
                if let Some(editing_dot) = app.ui_state.editing_dot.take() {
                    // Editing existing todo
                    let dot_key = crate::priority::DotKey::new(&editing_dot);
                    let mut tx = app.store.transact(app.identifier());
                    crate::todo::update_text(&mut tx, &dot_key, text);
                    let delta = tx.commit();
                    app.broadcast_delta(delta)?;
                } else {
                    // Creating new todo
                    let (dot_key, dot) = app.next_dot_key();
                    let mut tx = app.store.transact(app.identifier());
                    crate::todo::create_todo(&mut tx, &dot_key, text);
                    crate::priority::insert_at_priority(&mut tx, &dot, 0);
                    let delta = tx.commit();
                    app.broadcast_delta(delta)?;
                }
            }

            app.ui_state.input_buffer.clear();
            app.ui_state.editing_dot = None;
            app.ui_state.mode = Mode::Normal;
            Ok(true)
        }
        KeyCode::Esc => {
            app.ui_state.input_buffer.clear();
            app.ui_state.editing_dot = None;
            app.ui_state.mode = Mode::Normal;
            Ok(true)
        }
        KeyCode::Char(c) => {
            app.ui_state.input_buffer.push(c);
            Ok(true)
        }
        KeyCode::Backspace => {
            app.ui_state.input_buffer.pop();
            Ok(true)
        }
        _ => Ok(true),
    }
}

/// Execute an action on the app state.
pub fn execute_action(app: &mut App, action: Action) -> io::Result<()> {
    match action {
        Action::Quit => {
            // Handled by caller
            Ok(())
        }
        Action::MoveUp => {
            if app.ui_state.selected_index > 0 {
                app.ui_state.selected_index -= 1;
            }
            Ok(())
        }
        Action::MoveDown => {
            let todos = app.get_todos_ordered();
            if app.ui_state.selected_index + 1 < todos.len() {
                app.ui_state.selected_index += 1;
            }
            Ok(())
        }
        Action::ToggleDone => {
            let todos = app.get_todos_ordered();
            if let Some((dot, todo)) = todos.get(app.ui_state.selected_index) {
                let new_done = !todo.primary_done();
                let dot_key = crate::priority::DotKey::new(dot);

                let mut tx = app.store.transact(app.identifier());
                crate::todo::set_done(&mut tx, &dot_key, new_done);
                let delta = tx.commit();

                app.broadcast_delta(delta)?;
            }
            Ok(())
        }
        Action::Delete => {
            let todos = app.get_todos_ordered();
            if let Some((dot, _)) = todos.get(app.ui_state.selected_index)
                && let Some(index) = crate::priority::find_priority_index(&app.store.store, dot) {
                    let mut tx = app.store.transact(app.identifier());
                    crate::priority::remove_at_index(&mut tx, index);
                    let delta = tx.commit();

                    app.broadcast_delta(delta)?;

                    // Adjust selection if needed
                    let todos_after = app.get_todos_ordered();
                    if app.ui_state.selected_index >= todos_after.len() && !todos_after.is_empty() {
                        app.ui_state.selected_index = todos_after.len() - 1;
                    }
                }
            Ok(())
        }
        Action::EnterInsertMode => {
            app.ui_state.mode = Mode::Insert;
            app.ui_state.input_buffer.clear();
            app.ui_state.editing_dot = None;
            Ok(())
        }
        Action::ToggleIsolation => {
            app.toggle_isolation()?;
            Ok(())
        }
        Action::AddRandomTodos => {
            app.add_random_todos()?;
            Ok(())
        }
        Action::ScrollLogsUp => {
            app.ui_state.log_scroll = app.ui_state.log_scroll.saturating_add(3);
            Ok(())
        }
        Action::ScrollLogsDown => {
            app.ui_state.log_scroll = app.ui_state.log_scroll.saturating_sub(3);
            Ok(())
        }
        Action::EnterEditMode => {
            let todos = app.get_todos_ordered();
            if let Some((dot, todo)) = todos.get(app.ui_state.selected_index) {
                app.ui_state.mode = Mode::Insert;
                // Show all text values if there's a conflict, same as in the list view
                app.ui_state.input_buffer = if todo.text.len() > 1 {
                    format!("[{}]", todo.text.join(", "))
                } else {
                    todo.primary_text().to_string()
                };
                app.ui_state.editing_dot = Some(*dot);
            }
            Ok(())
        }
        Action::MovePriorityUp => {
            let todos = app.get_todos_ordered();
            let idx = app.ui_state.selected_index;
            if idx > 0 && idx < todos.len() {
                let (dot, _) = &todos[idx];

                // Read current position
                if let Some(current_pos) = crate::priority::find_priority_index(&app.store.store, dot)
                    && current_pos > 0 {
                        // Move up in priority (lower index)
                        let mut tx = app.store.transact(app.identifier());
                        crate::priority::remove_at_index(&mut tx, current_pos);
                        crate::priority::insert_at_priority(&mut tx, dot, current_pos - 1);
                        let delta = tx.commit();
                        app.broadcast_delta(delta)?;

                        // Update UI selection
                        app.ui_state.selected_index -= 1;
                    }
            }
            Ok(())
        }
        Action::MovePriorityDown => {
            let todos = app.get_todos_ordered();
            let idx = app.ui_state.selected_index;
            if idx < todos.len() {
                let (dot, _) = &todos[idx];

                // Read current position
                if let Some(current_pos) = crate::priority::find_priority_index(&app.store.store, dot) {
                    let priority_len = crate::priority::read_priority(&app.store.store).len();
                    if current_pos + 1 < priority_len {
                        // Move down in priority (higher index)
                        let mut tx = app.store.transact(app.identifier());
                        crate::priority::remove_at_index(&mut tx, current_pos);
                        crate::priority::insert_at_priority(&mut tx, dot, current_pos + 1);
                        let delta = tx.commit();
                        app.broadcast_delta(delta)?;

                        // Update UI selection
                        app.ui_state.selected_index += 1;
                    }
                }
            }
            Ok(())
        }
    }
}
