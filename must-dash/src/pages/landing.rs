use crate::{app::App, navigator::LandingFocus};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::composables::formatting::centered_rect;

pub fn draw_landing(f: &mut Frame, app: &App, landing_focus: LandingFocus) {
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
            Constraint::Length(3), // Save button
        ])
        .split(inner_area);

    let active_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(Color::DarkGray);
    let chosen_style = |cmp: LandingFocus| {
        if landing_focus == cmp {
            active_style
        } else {
            inactive_style
        }
    };

    let kp_p = Paragraph::new(app.env_vars.kp.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Kp  ").left_aligned())
                .title(Line::from("[Note: Only 1 decimal used]").right_aligned()),
        )
        .style(chosen_style(LandingFocus::Kp));
    f.render_widget(kp_p, chunks[0]);

    let ki_p = Paragraph::new(app.env_vars.ki.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title_position(ratatui::widgets::TitlePosition::Top)
                .title(Line::from(" Ki  ").left_aligned())
                .title(Line::from("[Note: Only 1 decimal used]").right_aligned()),
        )
        .style(chosen_style(LandingFocus::Ki));
    f.render_widget(ki_p, chunks[1]);

    let src_p = Paragraph::new(app.env_vars.source_id.as_str())
        .block(Block::default().borders(Borders::ALL).title(" source id "))
        .style(chosen_style(LandingFocus::SourceId));
    f.render_widget(src_p, chunks[2]);

    let start_text = if landing_focus == LandingFocus::Start {
        " [ START PROGRAM ] "
    } else {
        "   START PROGRAM   "
    };
    let start_p = Paragraph::new(start_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title_bottom("{ enter }"),
        )
        .style(chosen_style(LandingFocus::Start))
        .alignment(Alignment::Center);
    f.render_widget(start_p, chunks[4]);

    // Show a save button if a run was made before
    if !app.node_logs.is_empty() {
        let save_button = Paragraph::new("Save data")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title_bottom("{ s/S }"),
            )
            .style(chosen_style(LandingFocus::Save))
            .alignment(Alignment::Center);
        f.render_widget(save_button, chunks[5]);
    }
}
