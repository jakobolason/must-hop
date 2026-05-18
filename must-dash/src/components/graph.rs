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

/// Maximum number of heartbeats (packets) visible at one time in the graph.
/// Once more packets than this have been collected the scrollbar appears and
/// older data can be browsed by scrolling left.
const MAX_VISIBLE_HBS: usize = 20;

pub fn draw_dash_charts(f: &mut Frame, app: &App, area: Rect, graph_scroll: usize) {
    let total_packets = app.dash_stats.packets.len();
    let max_valid_scroll = total_packets.saturating_sub(MAX_VISIBLE_HBS);
    let clamped_graph_scroll = graph_scroll.min(max_valid_scroll);
    let chart_data = app
        .dash_stats
        .get_chart_data(MAX_VISIBLE_HBS, clamped_graph_scroll);

    let zero_ref_line = [(0.0, 0.0), (chart_data.x_bounds[1], 0.0)];

    let datasets = vec![
        Dataset::default()
            .name("error")
            .marker(symbols::Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::Cyan))
            .data(&chart_data.err),
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
        Dataset::default()
            .name("HW Live")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::LightGreen))
            .data(&chart_data.hw_live),
        Dataset::default()
            .name("0 ms ref")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Gray))
            .data(&zero_ref_line),
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

    if total_packets > MAX_VISIBLE_HBS {
        // scroll=0 means rightmost (newest); scrollbar position is inverted so the
        // thumb sits at the right when viewing live data and moves left as you scroll back.
        // Use clamped_graph_scroll so the thumb never misrepresents an over-scrolled position.
        let thumb_pos = max_valid_scroll.saturating_sub(clamped_graph_scroll);
        let mut scrollbar_state = ScrollbarState::new(total_packets)
            // .viewport_content_length(MAX_VISIBLE_HBS)
            .position(thumb_pos);
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
