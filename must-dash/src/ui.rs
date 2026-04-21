use crate::app::{App, AppView};
use ratatui::{
    Frame,
    layout::Alignment,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::composables::formatting::centered_rect;
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
