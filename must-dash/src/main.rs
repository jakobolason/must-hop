use ansi_to_tui::IntoText;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};
use std::{collections::VecDeque, io, process::Stdio, sync::OnceLock, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::mpsc,
};

/// Pre-compile the regex so we don't recompile it on every log line
static DRIFT_REGEX: OnceLock<regex::Regex> = OnceLock::new();

fn get_regex() -> &'static regex::Regex {
    DRIFT_REGEX.get_or_init(|| {
        // Matches: "Measured drift: 12.3 | err: -0.4 | ratio: 100 | self ratio: 98"
        regex::Regex::new(
            r"Measured drift:\s*([-\d.]+)\s*\|\s*err:\s*([-\d.]+)\s*\|\s*ratio:\s*([-\d]+)\s*\|\s*self ratio:\s*([-\d]+)"
        ).expect("Failed to compile regex")
    })
}

/// Events that our main loop will handle
enum AppEvent {
    Input(KeyCode),
    NodeLog(String),
    GwLog(String),
    Tick,
}

/// The state of our application
struct App {
    node_logs: VecDeque<String>,
    gw_logs: VecDeque<String>,

    // Parsed Metrics from TDMA Controller
    measured_drift: Option<f32>,
    err: Option<f32>,
    ratio: Option<i64>,
    self_ratio: Option<i64>,
}

impl App {
    fn new() -> Self {
        Self {
            node_logs: VecDeque::with_capacity(500),
            gw_logs: VecDeque::with_capacity(500),
            measured_drift: None,
            err: None,
            ratio: None,
            self_ratio: None,
        }
    }

    fn add_node_log(&mut self, log: String) {
        if self.node_logs.len() == 500 {
            self.node_logs.pop_front();
        }

        // Real-time analysis!
        if log.contains("Measured drift:")
            && let Some(caps) = get_regex().captures(&log)
        {
            // Ignore parse errors if the string is somehow malformed
            if let Ok(drift) = caps[1].parse::<f32>() {
                self.measured_drift = Some(drift);
            }
            if let Ok(err) = caps[2].parse::<f32>() {
                self.err = Some(err);
            }
            if let Ok(ratio) = caps[3].parse::<i64>() {
                self.ratio = Some(ratio);
            }
            if let Ok(self_ratio) = caps[4].parse::<i64>() {
                self.self_ratio = Some(self_ratio);
            }
        }

        self.node_logs.push_back(log);
    }

    fn add_gw_log(&mut self, log: String) {
        if self.gw_logs.len() == 500 {
            self.gw_logs.pop_front();
        }
        self.gw_logs.push_back(log);
    }
}

/// Helper function to read an async stream (stdout or stderr) and forward it to our app
fn spawn_log_reader<R, F>(stream: R, tx: mpsc::Sender<AppEvent>, event_mapper: F)
where
    R: AsyncRead + Unpin + Send + 'static,
    F: Fn(String) -> AppEvent + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            // Ignore channel send errors (happens when the app shuts down)
            let _ = tx.send(event_mapper(line)).await;
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, mut rx) = mpsc::channel(100);

    // 1. Keyboard Input
    let tx_input = tx.clone();
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(250)).unwrap()
                && let CEvent::Key(key) = event::read().unwrap()
            {
                // If the channel is closed (main thread exited), break the loop
                if tx_input.send(AppEvent::Input(key.code)).await.is_err() {
                    break;
                }
            }
            // If the channel is closed, break the loop
            if tx_input.send(AppEvent::Tick).await.is_err() {
                break;
            }
        }
    });

    let mut node_child = Command::new("just")
        .args(["remote-run", "7"])
        // --- ADD THESE THREE LINES ---
        .env("CARGO_TERM_COLOR", "always")
        .env("CLICOLOR_FORCE", "1")
        .env("DEFMT_LOG_STYLE", "always")
        // -----------------------------
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn Node command");

    spawn_log_reader(
        node_child.stdout.take().unwrap(),
        tx.clone(),
        AppEvent::NodeLog,
    );
    spawn_log_reader(
        node_child.stderr.take().unwrap(),
        tx.clone(),
        AppEvent::NodeLog,
    );

    // 3. Gateway process (`just run-gw`)
    let mut gw_child = Command::new("just")
        .args(["run-gw"])
        .env("CARGO_TERM_COLOR", "always")
        .env("CLICOLOR_FORCE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn GW command");

    spawn_log_reader(gw_child.stdout.take().unwrap(), tx.clone(), AppEvent::GwLog);
    spawn_log_reader(gw_child.stderr.take().unwrap(), tx.clone(), AppEvent::GwLog);

    // Main UI Loop
    let mut app = App::new();
    loop {
        terminal.draw(|f| ui(f, &app))?;

        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Input(KeyCode::Char('q')) | AppEvent::Input(KeyCode::Esc) => {
                    break;
                }
                AppEvent::Input(_) => {}
                AppEvent::NodeLog(line) => app.add_node_log(line),
                AppEvent::GwLog(line) => app.add_gw_log(line),
                AppEvent::Tick => {}
            }
        }
    }

    // Cleanup logic
    if let Some(pid) = node_child.id() {
        let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT);
    }

    // Gracefully ask Gateway (ssh) to shut down (sends Ctrl+C)
    if let Some(pid) = gw_child.id() {
        let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT);
    }

    // Killing child processes upon exit
    let _ = tokio::time::timeout(Duration::from_secs(2), node_child.wait()).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), gw_child.wait()).await;
    let _ = node_child.kill().await;
    let _ = gw_child.kill().await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut ratatui::Frame, app: &App) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(f.area());

    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    // --- Analytics Header ---
    let drift_text = if let (Some(drift), Some(err), Some(ratio), Some(self_ratio)) =
        (app.measured_drift, app.err, app.ratio, app.self_ratio)
    {
        format!(
            " 📡 Network Status | Drift: {:.2}ms | Err: {:.2}ms | Ratio: {} | Self Ratio: {} ",
            drift, err, ratio, self_ratio
        )
    } else {
        " 📡 Network Status | Waiting for TDMA sync data... ".to_string()
    };

    let header = Paragraph::new(drift_text)
        .block(Block::default().borders(Borders::ALL).title(" Metrics "))
        .style(Style::default().fg(Color::Yellow));
    f.render_widget(header, main_chunks[0]);

    // --- Node Logs (Left) ---
    // Instead of forcing a specific color, we convert ANSI sequences from `RUST_LOG_STYLE=always` to TUI format
    let node_raw_text = app.node_logs.iter().cloned().collect::<Vec<_>>().join("\n");
    let node_text = node_raw_text
        .into_text()
        .unwrap_or_else(|_| ratatui::text::Text::raw(&node_raw_text));

    let node_panel = Paragraph::new(node_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Node (remote-run 7) "),
    );

    let node_scroll = app
        .node_logs
        .len()
        .saturating_sub(log_chunks[0].height as usize - 2);
    f.render_widget(node_panel.scroll((node_scroll as u16, 0)), log_chunks[0]);

    // --- GW Logs (Right) ---
    let gw_raw_text = app.gw_logs.iter().cloned().collect::<Vec<_>>().join("\n");
    let gw_text = gw_raw_text
        .into_text()
        .unwrap_or_else(|_| ratatui::text::Text::raw(&gw_raw_text));

    let gw_panel = Paragraph::new(gw_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Gateway (run-gw) "),
    );

    let gw_scroll = app
        .gw_logs
        .len()
        .saturating_sub(log_chunks[1].height as usize - 2);
    f.render_widget(gw_panel.scroll((gw_scroll as u16, 0)), log_chunks[1]);
}

