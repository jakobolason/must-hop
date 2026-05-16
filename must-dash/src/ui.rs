use crate::{
    app::App,
    navigator::{Navigator, NavigatorView},
};
use ratatui::{
    Frame,
    layout::Alignment,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::composables::formatting::centered_rect;
use crate::pages::{dashboard::draw_dash, landing::draw_landing};

pub fn draw(f: &mut Frame, app: &App, navigator: &Navigator) {
    match navigator.view {
        NavigatorView::Landing => draw_landing(f, app, navigator),
        NavigatorView::Dashboard => draw_dash(f, app, &navigator.dash_focus, navigator.history_scroll),
    }

    if navigator.shutting_down {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area); // Clears the background logs behind the popup
        let popup = Paragraph::new("\nShutting down background processes gracefully...\n\nDisconnecting SSH and Debug Probe.")
            .block(Block::default().title(" Exiting ").borders(Borders::ALL).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)))
            .alignment(Alignment::Center);
        f.render_widget(popup, area);
    }
}
