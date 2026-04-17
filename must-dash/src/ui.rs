use crate::app::{App, Focus};
use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

pub fn draw(f: &mut Frame, app: &App) {
    // Determine heights based on what is in focus
    let (data_constraint, log_constraint) = match app.focus {
        Focus::Data => (Constraint::Percentage(50), Constraint::Percentage(50)),
        Focus::Logs => (Constraint::Length(3), Constraint::Min(0)), // Data collapses to 3 lines
    };

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top Bar (Metrics)
            data_constraint,       // Middle Row (Data)
            log_constraint,        // Bottom Row (Logs)
        ])
        .split(f.area());

    // --- 1. Analytics Header (Medians) ---
    let header_text = format!(
        " 📡 Medians (last 10) | Drift: {}ms | Err: {}ms | Ratio: {} | Self Ratio: {}  [Press TAB to switch focus] ",
        format_opt(app.drift.median()),
        format_opt(app.err.median()),
        format_opt(app.ratio.median()),
        format_opt(app.self_ratio.median()),
    );

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title(" Metrics "))
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(header, main_chunks[0]);

    // --- 2. Middle Row (Data View) ---
    let data_title = if app.focus == Focus::Data {
        " Data History (FOCUSED) "
    } else {
        " Data History "
    };
    let data_block = Block::default()
        .borders(Borders::ALL)
        .title(data_title)
        .style(if app.focus == Focus::Data {
            Style::default().fg(Color::LightCyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let mut history_items = Vec::new();

    // We base the loop on the longest list we might have.
    // Usually drift and delta_up will be the exact same length.
    let max_len = app.drift.values.len().max(app.delta_up.values.len());

    for i in 0..max_len {
        // Safely extract from parallel arrays, formatting if present
        let drift_str = app
            .drift
            .values
            .get(i)
            .map(|&v| format!("{:.3}ms", v))
            .unwrap_or_else(|| "--".to_string());

        let ratio_str = app
            .ratio
            .values
            .get(i)
            .map(|&v| format!("{:.0}", v))
            .unwrap_or_else(|| "--".to_string());

        let up_str = app
            .delta_up
            .values
            .get(i)
            .map(|&v| format!("{:.3}", v))
            .unwrap_or_else(|| "--".to_string());

        let down_str = app
            .delta_down
            .values
            .get(i)
            .map(|&v| format!("{:.3}", v))
            .unwrap_or_else(|| "--".to_string());

        // Format the expanded row
        history_items.push(ListItem::new(format!(
            "Entry {i:02}: Drift = {drift:<10} | Ratio = {ratio:<10} | Δ Up = {up:<8} | Δ Down = {down}",
            i = i + 1,
            drift = drift_str,
            ratio = ratio_str,
            up = up_str,
            down = down_str
        )));
    }

    let history_list = List::new(history_items).block(data_block);
    f.render_widget(history_list, main_chunks[1]);

    // --- 3. Bottom Row (Logs) ---
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[2]);

    let log_title_style = if app.focus == Focus::Logs {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Node Logs (Left)
    let node_raw = app.node_logs.iter().cloned().collect::<Vec<_>>().join("\n");
    let node_text = node_raw
        .into_text()
        .unwrap_or_else(|_| ratatui::text::Text::raw(&node_raw));
    let node_panel = Paragraph::new(node_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Node (remote-run) ")
            .style(log_title_style),
    );
    let node_scroll = app
        .node_logs
        .len()
        .saturating_sub(log_chunks[0].height as usize - 2);
    f.render_widget(node_panel.scroll((node_scroll as u16, 0)), log_chunks[0]);

    // GW Logs (Right)
    let gw_raw = app.gw_logs.iter().cloned().collect::<Vec<_>>().join("\n");
    let gw_text = gw_raw
        .into_text()
        .unwrap_or_else(|_| ratatui::text::Text::raw(&gw_raw));
    let gw_panel = Paragraph::new(gw_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Gateway (run-gw) ")
            .style(log_title_style),
    );
    let gw_scroll = app
        .gw_logs
        .len()
        .saturating_sub(log_chunks[1].height as usize - 2);
    f.render_widget(gw_panel.scroll((gw_scroll as u16, 0)), log_chunks[1]);

    if app.shutting_down {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area); // Clears the background logs behind the popup
        let popup = Paragraph::new("\nShutting down background processes gracefully...\n\nDisconnecting SSH and Debug Probe.")
            .block(Block::default().title(" Exiting ").borders(Borders::ALL).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)))
            .alignment(Alignment::Center);
        f.render_widget(popup, area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn format_opt(opt: Option<f32>) -> String {
    match opt {
        Some(val) => format!("{:.2}", val),
        None => "--".to_string(),
    }
}
