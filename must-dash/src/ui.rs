use crate::app::{App, AppView};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::pages::{dashboard::draw_dash, landing::draw_landing};

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

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

pub fn format_opt(lst: &[f32]) -> String {
    if !lst.is_empty() {
        let mean = lst.iter().sum::<f32>() / lst.len() as f32;
        format!("{:.2}", mean)
    } else {
        "--".to_string()
    }
}
