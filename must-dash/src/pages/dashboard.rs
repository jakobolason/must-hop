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

pub fn draw_dash(
    f: &mut Frame,
    app: &App,
    dash_focus: &DashFocus,
    history_scroll: usize,
    graph_scroll: usize,
    logs_scroll: usize,
) {
    let (data_constraint, log_constraint) = match dash_focus {
        DashFocus::Data => (Constraint::Percentage(50), Constraint::Percentage(50)),
        DashFocus::Logs => (Constraint::Length(3), Constraint::Min(0)),
    };

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), data_constraint, log_constraint])
        .split(f.area());

    draw_dash_header(f, app, main_chunks[0]);
    draw_dash_data(
        f,
        app,
        main_chunks[1],
        dash_focus,
        history_scroll,
        graph_scroll,
    );
    draw_dash_logs(f, app, main_chunks[2], dash_focus, logs_scroll);
}

fn draw_dash_header(f: &mut Frame, app: &App, area: Rect) {
    let mut param_text = String::new();
    let mut first_node = true;
    for ns in &app.configured_nodes {
        param_text.push_str(&format!(
            " [{}] SF:{} BW:{} tau:{} kp,ki:{},{}  ",
            ns.source_id, ns.sf, ns.bw, ns.tau, ns.kp, ns.ki
        ));
        if !first_node {
            param_text.push('|');
        } else {
            first_node = false;
        }
    }
    param_text.push_str("[TAB: Focus | ESC: Back | Q: Quit]");

    let header = ratatui::widgets::Paragraph::new(param_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Experiment Parameters "),
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(header, area);
}

fn draw_dash_data(
    f: &mut Frame,
    app: &App,
    area: Rect,
    dash_focus: &DashFocus,
    history_scroll: usize,
    graph_scroll: usize,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_area = chunks[0];
    let right_area = chunks[1];

    draw_history_panels(f, app, left_area, dash_focus, history_scroll);

    if right_area.height > 6 && right_area.width > 2 {
        draw_dash_charts(f, app, right_area, graph_scroll);
    }
}

fn draw_history_panels(
    f: &mut Frame,
    app: &App,
    area: Rect,
    dash_focus: &DashFocus,
    history_scroll: usize,
) {
    let n = app.node_stats.len();
    if n == 0 {
        let placeholder = Block::default()
            .borders(Borders::ALL)
            .title(" Data History ")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(placeholder, area);
        return;
    }

    let panel_constraints: Vec<Constraint> =
        (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect();

    let panels = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(panel_constraints)
        .split(area);

    let block_style = if *dash_focus == DashFocus::Data {
        Style::default().fg(Color::LightCyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let header = Row::new(vec![
        "Pkt", "HW Avg", "Err", "Delay", "Δ Up", "Δ Down", "Speed", "τ_hb",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

    let widths = [
        Constraint::Length(4),      // Pkt
        Constraint::Percentage(13), // HW Avg
        Constraint::Percentage(13), // Err
        Constraint::Percentage(13), // Delay
        Constraint::Percentage(14), // Δ Up
        Constraint::Percentage(14), // Δ Down
        Constraint::Percentage(14), // Speed
        Constraint::Length(4),      // τ Hi/Lo
    ];

    for (ns, &panel_area) in app.node_stats.iter().zip(panels.iter()) {
        let entries_visible = panel_area.height.saturating_sub(3) as usize;
        let total = ns.stats.packets.len();
        let clamped = history_scroll.min(total.saturating_sub(entries_visible));

        let lines = ns.stats.get_history_lines(entries_visible, clamped);
        let rows = lines.into_iter().map(Row::new);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ({}) ", ns.node_label, ns.probe_id))
            .style(block_style);

        let table = Table::new(rows, widths)
            .header(header.clone())
            .block(block)
            .column_spacing(1);

        f.render_widget(table, panel_area);

        if total > entries_visible {
            let scroll_pos = total
                .saturating_sub(entries_visible)
                .saturating_sub(clamped);
            let mut scrollbar_state = ScrollbarState::new(total).position(scroll_pos);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                panel_area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar_state,
            );
        }
    }
}
