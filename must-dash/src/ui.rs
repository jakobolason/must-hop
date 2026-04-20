use crate::app::{App, AppView, DashFocus, LandingFocus};
use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem, Paragraph},
};

pub fn draw(f: &mut Frame, app: &App) {
    match app.view {
        AppView::Landing => draw_landing(f, app),
        AppView::Dashboard => draw_dash(f, app),
    }

    if app.shutting_down {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area); // Clears the background logs behind the popup
        let popup = Paragraph::new("\nShutting down background processes gracefully...\n\nDisconnecting SSH and Debug Probe.")
            .block(Block::default().title(" Exiting ").borders(Borders::ALL).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)))
            .alignment(Alignment::Center);
        f.render_widget(popup, area);
    }
}

fn draw_landing(f: &mut Frame, app: &App) {
    let area = centered_rect(40, 60, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Configuration Setup ")
        .borders(Borders::ALL);
    f.render_widget(block, area);

    let inner_area = area.inner(Margin {
        vertical: 2,
        horizontal: 2,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // KP
            Constraint::Length(3), // KI
            Constraint::Length(3), // SOURCEID
            Constraint::Length(2), // Spacer
            Constraint::Length(3), // Start Button
        ])
        .split(inner_area);

    let active_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(Color::DarkGray);

    let kp_style = if app.landing_focus == LandingFocus::Kp {
        active_style
    } else {
        inactive_style
    };
    let ki_style = if app.landing_focus == LandingFocus::Ki {
        active_style
    } else {
        inactive_style
    };
    let src_style = if app.landing_focus == LandingFocus::SourceId {
        active_style
    } else {
        inactive_style
    };
    let start_style = if app.landing_focus == LandingFocus::Start {
        active_style
    } else {
        inactive_style
    };

    let kp_p = Paragraph::new(app.env_vars.kp.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" KP (Environment Variable) "),
        )
        .style(kp_style);
    f.render_widget(kp_p, chunks[0]);

    let ki_p = Paragraph::new(app.env_vars.ki.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" KI (Environment Variable) "),
        )
        .style(ki_style);
    f.render_widget(ki_p, chunks[1]);

    let src_p = Paragraph::new(app.env_vars.source_id.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SOURCEID (Environment Variable) "),
        )
        .style(src_style);
    f.render_widget(src_p, chunks[2]);

    let start_text = if app.landing_focus == LandingFocus::Start {
        " [ START PROGRAM ] "
    } else {
        "   START PROGRAM   "
    };
    let start_p = Paragraph::new(start_text)
        .block(Block::default().borders(Borders::ALL))
        .style(start_style)
        .alignment(Alignment::Center);
    f.render_widget(start_p, chunks[4]);
}

// NOTE: Dashboard
fn draw_dash(f: &mut Frame, app: &App) {
    // Determine heights based on what is in focus
    let (data_constraint, log_constraint) = match app.dash_focus {
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
    draw_dash_data(f, app, main_chunks[1]);
    draw_dash_logs(f, app, main_chunks[2]);
}

fn draw_dash_header(f: &mut Frame, app: &App, area: Rect) {
    let header_text = format!(
        "  Medians (last 10) | Error: {}ms | Err: {}ms | Prev Speed: {} | New speed: {}  [TAB: Focus | ESC: Back | Q: Quit] ",
        format_opt(app.dash_stats.delay.median()),
        format_opt(app.dash_stats.err.median()),
        format_opt(app.dash_stats.prev_speed.median()),
        format_opt(app.dash_stats.new_speed.median()),
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

fn draw_dash_data(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_area = chunks[0];
    let right_area = chunks[1];

    let data_block = Block::default()
        .borders(Borders::ALL)
        .title(" Data History")
        .style(if app.dash_focus == DashFocus::Data {
            Style::default().fg(Color::LightCyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    // Ask app.rs for exactly enough formatted rows to fill our vertical height
    let entries_that_can_be_seen = left_area.height.saturating_sub(2) as usize;
    let history_lines = app.dash_stats.get_history_lines(entries_that_can_be_seen);

    let history_items: Vec<ListItem> = history_lines.into_iter().map(ListItem::new).collect();

    let history_list = List::new(history_items).block(data_block);
    f.render_widget(history_list, left_area);

    if right_area.height > 6 && right_area.width > 2 {
        draw_dash_charts(f, app, right_area);
    }
}

fn draw_dash_charts(f: &mut Frame, app: &App, area: Rect) {
    // The chart boundary box takes 2 chars horizontally. Ask app.rs for exactly
    // enough data points to fit the exact width of our terminal chunk.
    let plot_width = area.width.saturating_sub(2) as usize;
    let chart_data = app.dash_stats.get_chart_data(plot_width);

    let datasets = vec![
        Dataset::default()
            .name("Delay")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Cyan))
            .data(&chart_data.delay),
        Dataset::default()
            .name("Δ Up")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Magenta))
            .data(&chart_data.up),
        Dataset::default()
            .name("Δ Down")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Yellow))
            .data(&chart_data.down),
        Dataset::default()
            .name("HW Scope")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Green))
            .data(&chart_data.hw),
    ];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(" Timing & Deltas (ms) ")
                .borders(Borders::ALL),
        )
        .x_axis(Axis::default().bounds(chart_data.x_bounds))
        .y_axis(Axis::default().bounds(chart_data.y_bounds).labels(vec![
            Span::raw(format!("{:.3}", chart_data.y_bounds[0])),
            Span::raw(format!("{:.3}", chart_data.y_bounds[1])),
        ]));

    f.render_widget(chart, area);
}

fn draw_dash_logs(f: &mut Frame, app: &App, area: Rect) {
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let log_title_style = if app.dash_focus == DashFocus::Logs {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // --- Node Logs (Left) ---
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

    // --- GW Logs (Right) ---
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
