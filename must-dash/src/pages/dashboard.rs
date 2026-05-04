use crate::app::App;

use crate::composables::formatting::format_opt;
use crate::navigator::DashFocus;
use ratatui::widgets::{Row, Table};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::components::{graph::draw_dash_charts, logs::draw_dash_logs};

pub fn draw_dash(f: &mut Frame, app: &App, dash_focus: &DashFocus) {
    // Determine heights based on what is in focus
    let (data_constraint, log_constraint) = match dash_focus {
        DashFocus::Data => (Constraint::Percentage(50), Constraint::Percentage(50)),
        DashFocus::Logs => (Constraint::Length(3), Constraint::Min(0)), // Data collapses to 3 lines
    };

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top Bar (Metrics)
            data_constraint,       // Middle Row (Data)
            log_constraint,        // Bottom Row (Logs)
        ])
        .split(f.area());

    // Delegate rendering to smaller focused functions
    draw_dash_header(f, app, main_chunks[0]);
    draw_dash_data(f, app, main_chunks[1], dash_focus);
    draw_dash_logs(f, app, main_chunks[2], dash_focus);
}

fn draw_dash_header(f: &mut Frame, app: &App, area: Rect) {
    let header_text = format!(
        "  Medians (last 10) | Error: {}ms | Err: {}ms | Prev Speed: {} | New speed: {}  [TAB: Focus | ESC: Back | Q: Quit] ",
        format_opt(&app.dash_stats.delay),
        format_opt(&app.dash_stats.err),
        format_opt(&app.dash_stats.prev_speed),
        format_opt(&app.dash_stats.new_speed),
    );

    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title(" Metrics "))
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(header, area);
}

fn draw_dash_data(f: &mut Frame, app: &App, area: Rect, dash_focus: &DashFocus) {
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

    // Ask app.rs for exactly enough formatted rows to fill our vertical height
    let entries_that_can_be_seen = left_area.height.saturating_sub(3) as usize;
    let history_lines = app.dash_stats.get_history_lines(entries_that_can_be_seen);

    let history_items = history_lines.into_iter().map(Row::new);
    let header = Row::new(vec![
        "Packet",
        "Error",
        "Speed",
        "Δ Up",
        "Δ Down",
        "HW Delay",
        "GW delay",
        "Node delay",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0); // Set to 1 for an empty line between header and data
    let widths = [
        Constraint::Length(8),      // Fixed length for "Packet 01"
        Constraint::Percentage(15), // Delay
        Constraint::Percentage(21), // Speed
        Constraint::Percentage(10), // Δ Up
        Constraint::Percentage(10), // Δ Down
        Constraint::Percentage(5),  // HW Delay
        Constraint::Percentage(8),
        Constraint::Percentage(8),
    ];

    // let history_list = List::new(history_items).block(data_block);
    let history_table = Table::new(history_items, widths)
        .header(header)
        .block(data_block)
        .column_spacing(1);
    f.render_widget(history_table, left_area);

    if right_area.height > 6 && right_area.width > 2 {
        draw_dash_charts(f, app, right_area);
    }
}
