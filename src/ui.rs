// ABOUTME: Terminal UI rendering using ratatui.
// ABOUTME: Displays todos, status bar, and help text.

use crate::app::{App, Mode};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

/// Draw the entire UI.
pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Status bar
            Constraint::Min(0),    // Todo list
            Constraint::Length(8), // Log window + context
            Constraint::Length(3), // Help text
        ])
        .split(f.area());

    draw_status(f, app, chunks[0]);
    draw_list(f, app, chunks[1]);

    // Split the log area into logs (2/3) and context (1/3)
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(67), // Logs
            Constraint::Percentage(33), // Context
        ])
        .split(chunks[2]);

    draw_logs(f, app, log_chunks[0]);
    draw_context(f, app, log_chunks[1]);
    draw_help(f, app, chunks[3]);
}

/// Draw the status bar.
fn draw_status(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let isolation_status = if app.network_isolated { "YES" } else { "NO" };

    let text = format!(
        "Replica: {} | Port: {} | Isolated: {}",
        app.replica_id, app.port, isolation_status
    );

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Status"));

    f.render_widget(paragraph, area);
}

/// Draw the todo list.
fn draw_list(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let todos = app.get_todos_ordered();

    let items: Vec<ListItem> = todos
        .iter()
        .enumerate()
        .map(|(i, (_dot, todo))| {
            let checkbox = if todo.primary_done() { "[✓]" } else { "[ ]" };
            let conflict_indicator = if todo.has_conflicts() { " ⚠ " } else { "   " };

            // Show all text values if there's a conflict
            let text = if todo.text.len() > 1 {
                format!("[{}]", todo.text.join(", "))
            } else {
                todo.primary_text().to_string()
            };

            let content = format!("{checkbox} {conflict_indicator}{text}");

            let style = if i == app.ui_state.selected_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(content).style(style)
        })
        .collect();

    // Show input mode if inserting
    let title = match app.ui_state.mode {
        Mode::Normal => "Todos",
        Mode::Insert => {
            let input = &app.ui_state.input_buffer;
            let edit_mode = if app.ui_state.editing_dot.is_some() {
                "Edit"
            } else {
                "Add"
            };
            return draw_insert_mode(f, area, input, edit_mode);
        }
    };

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(list, area);
}

/// Draw the insert mode UI.
fn draw_insert_mode(f: &mut Frame, area: ratatui::layout::Rect, input: &str, mode: &str) {
    let text = vec![Line::from(vec![
        Span::styled(
            format!("{mode} Todo: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(input),
        Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ])];

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Insert Mode"));

    f.render_widget(paragraph, area);
}

/// Draw the log window.
fn draw_logs(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let total_logs = app.log_buffer.len();
    let visible_lines = area.height.saturating_sub(2) as usize;

    // Calculate the range of logs to display based on scroll position
    let scroll_offset = app
        .ui_state
        .log_scroll
        .min(total_logs.saturating_sub(visible_lines));

    let log_lines: Vec<Line> = app
        .log_buffer
        .iter()
        .rev()
        .skip(scroll_offset)
        .take(visible_lines)
        .rev()
        .map(|s| {
            // Color code by replica ID
            // Extract replica ID from log message like "[Replica 3a]"
            let color = if s.contains("Replica") {
                if let Some(start) = s.find("Replica ") {
                    if let Some(end) = s[start..].find(']') {
                        let replica_str = &s[start + 8..start + end];
                        if let Ok(replica_id) = u8::from_str_radix(replica_str, 16) {
                            // Assign colors based on replica ID
                            match replica_id % 6 {
                                0 => Color::Cyan,
                                1 => Color::Green,
                                2 => Color::Yellow,
                                3 => Color::Magenta,
                                4 => Color::Blue,
                                _ => Color::Red,
                            }
                        } else {
                            Color::White
                        }
                    } else {
                        Color::White
                    }
                } else {
                    Color::White
                }
            } else {
                Color::White
            };

            Line::from(Span::styled(s.as_str(), Style::default().fg(color)))
        })
        .collect();

    // Add scroll indicator to title
    let title = if total_logs > visible_lines {
        format!(
            "Network Logs (↑↓ scroll {}/{})",
            scroll_offset,
            total_logs.saturating_sub(visible_lines)
        )
    } else {
        "Network Logs".to_string()
    };

    let paragraph =
        Paragraph::new(log_lines).block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(paragraph, area);
}

/// Draw the causal context window.
fn draw_context(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    use std::collections::BTreeMap;

    // Build a map of node_id -> highest_seq from the causal context
    let mut node_seqs: BTreeMap<u8, u64> = BTreeMap::new();

    for dot in app.store.context.dots() {
        let node = dot.actor().node().value();
        let seq = dot.sequence().get();
        node_seqs
            .entry(node)
            .and_modify(|max| {
                if seq > *max {
                    *max = seq;
                }
            })
            .or_insert(seq);
    }

    // Build the display lines
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Node → Seq",
        Style::default().add_modifier(Modifier::BOLD),
    )));

    for (node, seq) in node_seqs.iter() {
        let line_str = format!("{node:02x} → {seq}");
        lines.push(Line::from(line_str));
    }

    // TODO: Add missing dots detection if needed
    // For now, just show the version vector

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Causal Context"),
    );

    f.render_widget(paragraph, area);
}

/// Draw the help text.
fn draw_help(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let help_text = match app.ui_state.mode {
        Mode::Normal => {
            "q: quit | i: add | r: random | Enter: edit | j/k: nav | J/K: priority | ↑↓: scroll logs | space: toggle | d: delete | p: isolate"
        }
        Mode::Insert => "Enter: save | Esc: cancel",
    };

    let paragraph =
        Paragraph::new(help_text).block(Block::default().borders(Borders::ALL).title("Help"));

    f.render_widget(paragraph, area);
}
