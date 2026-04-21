use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};

pub fn draw_dash_charts(f: &mut Frame, app: &App, area: Rect) {
    // The chart boundary box takes 2 chars horizontally. Ask app.rs for exactly
    // enough data points to fit the exact width of our terminal chunk.
    let plot_width = area.width.saturating_sub(2) as usize;
    let chart_data = app.dash_stats.get_chart_data(plot_width);

    let zero_ref_line = [(0.0, 0.0), (chart_data.x_bounds[1], 0.0)];

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
}
