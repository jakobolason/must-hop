use crate::app::App;
use crate::navigator::DashFocus;
use ratatui::widgets::{Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders},
};

use crate::components::{graph::draw_dash_charts, logs::draw_dash_logs};

pub fn draw_dash(f: &mut Frame, app: &App, dash_focus: &DashFocus, history_scroll: usize) {
    let (data_constraint, log_constraint) = match dash_focus {
        DashFocus::Data => (Constraint::Percentage(50), Constraint::Percentage(50)),
        DashFocus::Logs => (Constraint::Length(3), Constraint::Min(0)),
    };

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), data_constraint, log_constraint])
        .split(f.area());

    draw_dash_header(f, app, main_chunks[0]);
    draw_dash_data(f, app, main_chunks[1], dash_focus, history_scroll);
    draw_dash_logs(f, app, main_chunks[2], dash_focus);
}

fn draw_dash_header(f: &mut Frame, app: &App, area: Rect) {
    let stats = &app.dash_stats;
    let n = 10;

    let fmt_ms = |v: Option<f32>| v.map_or("--".to_string(), |x| format!("{:.3}ms", x));
    let fmt_ppb = |v: Option<f32>| v.map_or("--".to_string(), |x| format!("{}", x as i64));

    let header_text = format!(
        "  Medians (last {n}) \
         | Measured Δ: {} \
         | Clock Err: {} \
         | Prev Speed: {} ppb \
         | New Speed: {} ppb  \
         [TAB: Focus | ESC: Back | Q: Quit] ",
        fmt_ms(stats.median_n(n, |p| p.delay_ms)),
        fmt_ms(stats.median_n(n, |p| p.err_ms)),
        fmt_ppb(stats.median_n(n, |p| p.prev_speed)),
        fmt_ppb(stats.median_n(n, |p| p.new_speed)),
    );

    let header = ratatui::widgets::Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title(" Metrics "))
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(header, area);
}

fn draw_dash_data(f: &mut Frame, app: &App, area: Rect, dash_focus: &DashFocus, history_scroll: usize) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_area = chunks[0];
    let right_area = chunks[1];

    let data_block = Block::default()
        .borders(Borders::ALL)
        .title(" Data History")
        .style(if *dash_focus == DashFocus::Data {
            Style::default().fg(Color::LightCyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let entries_that_can_be_seen = left_area.height.saturating_sub(3) as usize;
    let total_packets = app.dash_stats.packets.len();
    let clamped_scroll = history_scroll.min(total_packets.saturating_sub(entries_that_can_be_seen));
    let history_lines = app.dash_stats.get_history_lines(entries_that_can_be_seen, clamped_scroll);

    let history_items = history_lines.into_iter().map(Row::new);

    let header = Row::new(vec![
        "Pkt", "HW Avg", "Err", "Delay", "Δ Up", "Δ Down", "Speed", "GW µs", "GW B", "Node µs",
        "Node B",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

    let widths = [
        Constraint::Length(4),      // "Pkt" — "00"
        Constraint::Percentage(7),  // Err
        Constraint::Percentage(7),  // delay
        Constraint::Percentage(7),  // Speed (ppb, no decimal needed)
        Constraint::Percentage(10), // Δ Up
        Constraint::Percentage(10), // Δ Down
        Constraint::Percentage(10), // HW Avg
        Constraint::Percentage(10), // GW µs
        Constraint::Percentage(5),  // GW B
        Constraint::Percentage(10), // Node µs
        Constraint::Percentage(5),  // Node B
    ];

    let history_table = Table::new(history_items, widths)
        .header(header)
        .block(data_block)
        .column_spacing(1);

    f.render_widget(history_table, left_area);

    if total_packets > entries_that_can_be_seen {
        let scroll_pos = total_packets
            .saturating_sub(entries_that_can_be_seen)
            .saturating_sub(clamped_scroll);
        let mut scrollbar_state = ScrollbarState::new(total_packets)
            .viewport_content_length(entries_that_can_be_seen)
            .position(scroll_pos);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            left_area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }

    if right_area.height > 6 && right_area.width > 2 {
        draw_dash_charts(f, app, right_area);
    }
}
