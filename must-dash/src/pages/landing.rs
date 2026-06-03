use crate::app::{App, ProbeConfigFocus};
use crate::navigator::{LandingSection, LandingSubView, Navigator};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::composables::formatting::centered_rect;

pub fn draw_landing(f: &mut Frame, app: &App, navigator: &Navigator) {
    match navigator.landing_sub_view {
        LandingSubView::ProbeList => draw_probe_list(f, app, navigator),
        LandingSubView::ProbeConfig => draw_probe_config(f, app, navigator),
    }
}

fn draw_probe_list(f: &mut Frame, app: &App, navigator: &Navigator) {
    let area = centered_rect(70, 80, f.area());
    f.render_widget(Clear, area);

    let outer = Block::default()
        .title(" Probe Selection ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(outer, area);

    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    let help_height = 2u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Min(0),
            Constraint::Length(help_height),
        ])
        .split(inner);

    let probe_items: Vec<ListItem> = if let Some(err) = &app.probe_fetch_error {
        vec![ListItem::new(Span::styled(
            err.as_str(),
            Style::default().fg(Color::Red),
        ))]
    } else {
        app.available_probes
            .iter()
            .map(|p| ListItem::new(format!("[{}]  {}", p.index, p.name)))
            .collect()
    };

    let probes_focused = navigator.landing_section == LandingSection::Probes;
    let nodes_focused = navigator.landing_section == LandingSection::Nodes;

    let probe_list = List::new(probe_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Available Probes ")
                .style(if probes_focused {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    let mut probe_state = ListState::default();
    if !app.available_probes.is_empty() && probes_focused {
        probe_state.select(Some(navigator.probe_list_cursor));
    }
    f.render_stateful_widget(probe_list, chunks[0], &mut probe_state);

    let node_items: Vec<ListItem> = if app.configured_nodes.is_empty() {
        vec![ListItem::new(Span::styled(
            "  No nodes configured yet",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.configured_nodes
            .iter()
            .map(|n| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  node-{}  ", n.source_id),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("[probe {}] ", n.probe_index),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(format!(
                        "KP={} KI={}  SF={} BW={}kHz",
                        n.kp, n.ki, n.sf, n.bw
                    )),
                ]))
            })
            .collect()
    };

    let node_block_style = if nodes_focused {
        Style::default().fg(Color::Cyan)
    } else if app.configured_nodes.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Green)
    };

    let node_list = List::new(node_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Configured Nodes ")
                .style(node_block_style),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("► ");

    let mut node_state = ListState::default();
    if nodes_focused && !app.configured_nodes.is_empty() {
        node_state.select(Some(navigator.node_list_cursor));
    }
    f.render_stateful_widget(node_list, chunks[1], &mut node_state);

    let mut help_parts = vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Configure / Edit   "),
        Span::styled(
            "D",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Remove node   "),
        Span::styled(
            "R",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Refresh   "),
    ];

    if !app.configured_nodes.is_empty() {
        help_parts.extend([
            Span::styled(
                "S",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": Start   "),
        ]);
    }
    if app.has_data() {
        help_parts.extend([
            Span::styled(
                "W",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": Save run data   "),
        ]);
    }
    help_parts.extend([
        Span::styled(
            "Esc",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Quit"),
    ]);

    let help = Paragraph::new(Line::from(help_parts)).alignment(Alignment::Center);
    f.render_widget(help, chunks[2]);
}

fn draw_probe_config(f: &mut Frame, app: &App, navigator: &Navigator) {
    let area = centered_rect(55, 88, f.area());
    f.render_widget(Clear, area);

    let title = app.pending_node.as_ref().map_or_else(
        || " Configure Node ".to_string(),
        |n| {
            let verb = if app.editing_node_index.is_some() {
                "Edit"
            } else {
                "Configure"
            };
            format!(
                " {} Node — probe [{}]: {} ",
                verb,
                n.probe_index,
                n.name_short()
            )
        },
    );

    let outer = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(outer, area);

    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 3,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // KP
            Constraint::Length(3), // KI
            Constraint::Length(3), // Source ID
            Constraint::Length(3), // SF
            Constraint::Length(3), // BW
            Constraint::Length(3), // TAU
            Constraint::Length(1), // spacer
            Constraint::Length(3), // Confirm button
            Constraint::Min(0),    // rest
            Constraint::Length(1), // help
        ])
        .split(inner);

    let focus = navigator.probe_config_focus;

    let active = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive = Style::default().fg(Color::DarkGray);
    let style_for = |f: ProbeConfigFocus| if focus == f { active } else { inactive };

    let node = app.pending_node.as_ref();

    // KP field
    let kp_val = node.map_or("", |n| n.kp.as_str());
    let kp = Paragraph::new(kp_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Kp ").left_aligned())
                .title(Line::from("[no decimals allowed]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::Kp));
    f.render_widget(kp, chunks[0]);

    // KI field
    let ki_val = node.map_or("", |n| n.ki.as_str());
    let ki = Paragraph::new(ki_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Ki ").left_aligned())
                .title(Line::from("[no decimals allowed]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::Ki));
    f.render_widget(ki, chunks[1]);

    // Source ID field
    let sid_val = node.map_or("", |n| n.source_id.as_str());
    let sid = Paragraph::new(sid_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Source ID ").left_aligned())
                .title(Line::from("[also used as remote-run arg]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::SourceId));
    f.render_widget(sid, chunks[2]);

    // SF field
    let sf_val = node.map_or("", |n| n.sf.as_str());
    let sf = Paragraph::new(sf_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Spreading Factor ").left_aligned())
                .title(Line::from("[5–12]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::Sf));
    f.render_widget(sf, chunks[3]);

    // BW field
    let bw_val = node.map_or("", |n| n.bw.as_str());
    let bw = Paragraph::new(bw_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" Bandwidth (kHz) ").left_aligned())
                .title(Line::from("[125 / 250 / 500]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::Bw));
    f.render_widget(bw, chunks[4]);

    // TAU field
    let tau_val = node.map_or("", |n| n.tau.as_str());
    let tau = Paragraph::new(tau_val)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Line::from(" heartbeat duration (tau) [s] ").left_aligned())
                .title(Line::from("[10-100]").right_aligned()),
        )
        .style(style_for(ProbeConfigFocus::Tau));
    f.render_widget(tau, chunks[5]);

    // Confirm button
    let confirm_label = if focus == ProbeConfigFocus::Confirm {
        "[ CONFIRM ]"
    } else {
        "  CONFIRM  "
    };
    let confirm = Paragraph::new(confirm_label)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title_bottom("{ enter }"),
        )
        .style(style_for(ProbeConfigFocus::Confirm))
        .alignment(Alignment::Center);
    f.render_widget(confirm, chunks[7]);

    // Help line
    let help = Paragraph::new(
        "Tab/↓: Next field   Shift-Tab/↑: Prev   Enter on Confirm: Save   Esc: Cancel",
    )
    .style(Style::default().fg(Color::DarkGray))
    .alignment(Alignment::Center);
    f.render_widget(help, chunks[8]);
}

impl crate::app::NodeConfig {
    fn name_short(&self) -> &str {
        // Show the first ~40 chars of the probe name to keep the title compact
        let s = self.probe_name.as_str();
        s.char_indices().nth(40).map(|(i, _)| &s[..i]).unwrap_or(s)
    }
}
