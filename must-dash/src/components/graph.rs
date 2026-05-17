use crate::app::App;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

pub fn draw_dash_charts(f: &mut Frame, app: &App, area: Rect, graph_scroll: usize) {
    // The chart boundary box takes 2 chars horizontally.
    let plot_width = area.width.saturating_sub(2) as usize;
    let chart_data = app.dash_stats.get_chart_data(plot_width, graph_scroll);
    let total_packets = app.dash_stats.packets.len();

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

    if total_packets > plot_width {
        // scroll=0 means rightmost (newest); scrollbar position is inverted so the
        // thumb sits at the right when viewing live data and moves left as you scroll back.
        let max_scroll = total_packets.saturating_sub(1);
        let thumb_pos = max_scroll.saturating_sub(graph_scroll);
        let mut scrollbar_state = ScrollbarState::new(max_scroll)
            .viewport_content_length(plot_width)
            .position(thumb_pos);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                .begin_symbol(Some("←"))
                .end_symbol(Some("→")),
            area.inner(Margin { vertical: 0, horizontal: 1 }),
            &mut scrollbar_state,
        );
    }
}
