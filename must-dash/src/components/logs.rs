use crate::app::{App, DashFocus};
use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

pub fn draw_dash_logs(f: &mut Frame, app: &App, area: Rect) {
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(45),
            Constraint::Percentage(5),
        ])
        .split(area);

    let log_title_style = if app.dash_focus == DashFocus::Logs {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // --- Node Logs (Left) ---
    let node_raw = app.node_logs.to_vec().join("\n");
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

    // --- GW Logs (Right) ---
    let gw_raw = app.gw_logs.to_vec().join("\n");
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

    // --- Delay Logs
    let delay_raw = app
        .dash_stats
        .hardware_delay
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let delay_text = delay_raw
        .into_text()
        .unwrap_or_else(|_| ratatui::text::Text::raw(&delay_raw));
    let delay_panel = Paragraph::new(delay_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Measured delay between GW and Node")
            .style(log_title_style),
    );
    let delay_scroll = app
        .dash_stats
        .hardware_delay
        .len()
        .saturating_sub(log_chunks[2].height as usize - 2);
    f.render_widget(delay_panel.scroll((delay_scroll as u16, 0)), log_chunks[2]);
}
