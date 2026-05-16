use crate::{app::App, navigator::DashFocus};
use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

pub fn draw_dash_logs(f: &mut Frame, app: &App, area: Rect, dash_focus: &DashFocus) {
    let log_title_style = if *dash_focus == DashFocus::Logs {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let n = app.sources.len();
    let delay_pct = 5u16;
    let source_pct = (100u16 - delay_pct) / n as u16;
    let leftover = (100u16 - delay_pct) % n as u16;

    let mut constraints: Vec<Constraint> = (0..n)
        .map(|i| Constraint::Percentage(source_pct + if i == 0 { leftover } else { 0 }))
        .collect();
    constraints.push(Constraint::Percentage(delay_pct));

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, source) in app.sources.iter().enumerate() {
        let raw = source.logs.join("\n");
        let text = raw
            .into_text()
            .unwrap_or_else(|_| ratatui::text::Text::raw(&raw));

        let panel = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", source.id))
                .style(log_title_style),
        );

        let scroll = source.logs.len().saturating_sub(chunks[i].height as usize - 2);
        f.render_widget(panel.scroll((scroll as u16, 0)), chunks[i]);
    }

    // Delay panel is always last, driven by DashStats rather than a log buffer
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
            .title(" hw delay ")
            .style(log_title_style),
    );
    let delay_scroll = app
        .dash_stats
        .hardware_delay
        .len()
        .saturating_sub(chunks[n].height as usize - 2);
    f.render_widget(delay_panel.scroll((delay_scroll as u16, 0)), chunks[n]);
}
