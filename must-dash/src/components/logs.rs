use crate::{app::App, navigator::DashFocus};
use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

pub fn draw_dash_logs(
    f: &mut Frame,
    app: &App,
    area: Rect,
    dash_focus: &DashFocus,
    logs_scroll: usize,
) {
    let log_title_style = if *dash_focus == DashFocus::Logs {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let n = app.sources.len();
    let n_hw = app
        .node_stats
        .iter()
        .filter(|ns| ns.din_index.is_some())
        .count();
    let hw_pct_each = 5u16;
    let total_hw_pct = hw_pct_each * n_hw as u16;
    let source_pct = if n != 0 {
        (100u16 - total_hw_pct) / n as u16
    } else {
        0
    };
    let leftover = if n != 0 {
        (100u16 - total_hw_pct) % n as u16
    } else {
        0
    };

    let mut constraints: Vec<Constraint> = (0..n)
        .map(|i| Constraint::Percentage(source_pct + if i == 0 { leftover } else { 0 }))
        .collect();
    for _ in 0..n_hw {
        constraints.push(Constraint::Percentage(hw_pct_each));
    }

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

        let visible_lines = chunks[i].height as usize - 2;
        let total_lines = source.logs.len();
        let max_scroll = total_lines.saturating_sub(visible_lines);
        let clamped = logs_scroll.min(max_scroll);
        // scroll=0 → bottom (newest); higher → further back
        let scroll_row = max_scroll.saturating_sub(clamped);

        f.render_widget(panel.scroll((scroll_row as u16, 0)), chunks[i]);

        if total_lines > visible_lines {
            let thumb_pos = max_scroll.saturating_sub(clamped);
            let mut scrollbar_state = ScrollbarState::new(max_scroll + 1).position(thumb_pos);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                chunks[i].inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }

    // One HW delay panel per node that has a DIN index assigned.
    let hw_nodes: Vec<_> = app
        .node_stats
        .iter()
        .filter(|ns| ns.din_index.is_some())
        .collect();

    for (hw_i, ns) in hw_nodes.iter().enumerate() {
        let panel_idx = n + hw_i;
        let raw = ns
            .hardware_delay
            .iter()
            .map(|v: &f32| {
                if v.is_finite() {
                    format!("{:.4}", v)
                } else {
                    "--".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = raw
            .into_text()
            .unwrap_or_else(|_| ratatui::text::Text::raw(&raw));
        let title = format!(" hw din{} ", ns.din_index.unwrap_or(0));
        let panel = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(log_title_style),
        );
        let visible_lines = chunks[panel_idx].height as usize - 2;
        let total_lines = ns.hardware_delay.len();
        let max_scroll = total_lines.saturating_sub(visible_lines);
        let clamped = logs_scroll.min(max_scroll);
        let scroll_row = max_scroll.saturating_sub(clamped);
        f.render_widget(panel.scroll((scroll_row as u16, 0)), chunks[panel_idx]);

        if total_lines > visible_lines {
            let thumb_pos = max_scroll.saturating_sub(clamped);
            let mut scrollbar_state = ScrollbarState::new(max_scroll + 1).position(thumb_pos);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                chunks[panel_idx].inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }
}
