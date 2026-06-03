use crate::app::App;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{
        Axis, Block, Borders, Chart, Dataset, GraphType, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};

const MAX_VISIBLE_HBS: usize = 20;

/// One distinct color per node for the error series overlay.
const NODE_ERR_COLORS: [Color; 4] = [Color::Cyan, Color::Yellow, Color::Magenta, Color::LightBlue];

pub fn draw_dash_charts(f: &mut Frame, app: &App, area: Rect, graph_scroll: usize) {
    let Some(primary) = app.node_stats.first() else {
        f.render_widget(
            Block::default()
                .title(" Timing & Deltas (ms) ")
                .borders(Borders::ALL),
            area,
        );
        return;
    };

    let total_packets = primary.stats.packets.len();
    let max_valid_scroll = total_packets.saturating_sub(MAX_VISIBLE_HBS);
    let clamped_scroll = graph_scroll.min(max_valid_scroll);

    // Build chart data for every node so we can overlay error series and merge y-bounds.
    let all_chart_data: Vec<_> = app
        .node_stats
        .iter()
        .map(|ns| {
            let safe_idx = ns.last_hw_idx.min(ns.hardware_delay.len());
            let hw_pending = &ns.hardware_delay[safe_idx..];
            ns.stats.get_chart_data(MAX_VISIBLE_HBS, clamped_scroll, hw_pending)
        })
        .collect();

    let primary_data = &all_chart_data[0];
    let zero_ref_line = [(0.0, 0.0), (primary_data.x_bounds[1], 0.0)];

    // Merge y-bounds across all node error/delta/hw series.
    let (y_min, y_max) = all_chart_data
        .iter()
        .flat_map(|cd| {
            cd.err
                .iter()
                .chain(cd.up.iter())
                .chain(cd.down.iter())
                .chain(cd.hw.iter())
                .chain(cd.hw_live.iter())
        })
        .map(|&(_, y)| y)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), y| {
            (mn.min(y), mx.max(y))
        });

    let y_bounds = if y_min.is_infinite() {
        [0.0, 10.0]
    } else if (y_max - y_min).abs() < f64::EPSILON {
        [y_min - 1.0, y_max + 1.0]
    } else {
        let padding = (y_max - y_min) * 0.1;
        [y_min - padding, y_max + padding]
    };

    let mut datasets: Vec<Dataset> = Vec::new();

    // Per-node error series in distinct colors.
    for (i, (ns, cd)) in app.node_stats.iter().zip(all_chart_data.iter()).enumerate() {
        let color = NODE_ERR_COLORS.get(i).copied().unwrap_or(Color::White);
        datasets.push(
            Dataset::default()
                .name(format!("err {}", ns.node_label))
                .marker(symbols::Marker::Dot)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(color))
                .data(&cd.err),
        );
    }

    // Primary node delta up/down and HW scope (shared channel — same for all nodes).
    datasets.extend([
        Dataset::default()
            .name("Δ Up")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::LightMagenta))
            .data(&primary_data.up),
        Dataset::default()
            .name("Δ Down")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::LightRed))
            .data(&primary_data.down),
        Dataset::default()
            .name("HW Scope")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Green))
            .data(&primary_data.hw),
        Dataset::default()
            .name("HW Live")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::LightGreen))
            .data(&primary_data.hw_live),
        Dataset::default()
            .name("0 ms ref")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Gray))
            .data(&zero_ref_line),
    ]);

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(" Timing & Deltas (ms) ")
                .borders(Borders::ALL),
        )
        .x_axis(Axis::default().bounds(primary_data.x_bounds))
        .y_axis(Axis::default().bounds(y_bounds).labels(vec![
            Span::raw(format!("{:.3}", y_bounds[0])),
            Span::raw(format!("{:.3}", y_bounds[1])),
        ]));

    f.render_widget(chart, area);

    if total_packets > MAX_VISIBLE_HBS {
        let thumb_pos = max_valid_scroll.saturating_sub(clamped_scroll);
        let mut scrollbar_state = ScrollbarState::new(total_packets).position(thumb_pos);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::HorizontalBottom),
            area.inner(Margin {
                vertical: 0,
                horizontal: 1,
            }),
            &mut scrollbar_state,
        );
    }
}
