use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Duration};
use tokio::sync::mpsc;

use must_dash::app::{App, AppEvent};
use must_dash::ui;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

fn spawn_pty_reader(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    tx: mpsc::Sender<AppEvent>,
    event_mapper: impl Fn(String, bool) -> AppEvent + Send + 'static,
) -> Box<dyn portable_pty::Child + Send + Sync> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 220,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("Failed to open PTY");

    let mut cmd = CommandBuilder::new(program);
    for arg in args {
        cmd.arg(arg);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.cwd(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));

    let child = pair.slave.spawn_command(cmd).expect("Failed to spawn");

    // The master side is a synchronous Read — run it on a blocking thread
    let mut master_reader = pair.master.try_clone_reader().unwrap();
    tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        let mut last_was_cr = false;
        loop {
            match master_reader.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let b = byte[0];
                    if b == b'\r' {
                        let line = String::from_utf8_lossy(&buf).into_owned();
                        let _ = tx.blocking_send(event_mapper(line, true));
                        buf.clear();
                        last_was_cr = true;
                    } else if b == b'\n' {
                        if last_was_cr {
                            // \r\n pair — \r already sent the line content,
                            // but we still need to prime a new line slot
                            let _ = tx.blocking_send(event_mapper("".to_string(), false));
                            last_was_cr = false;
                        } else {
                            let line = String::from_utf8_lossy(&buf).into_owned();
                            if !line.is_empty() {
                                let _ = tx.blocking_send(event_mapper(line, true));
                            }
                            let _ = tx.blocking_send(event_mapper("".to_string(), false));
                            buf.clear();
                        }
                    } else {
                        last_was_cr = false;
                        buf.push(b);
                    }
                }
            }
        }
    });

    child
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tx, mut rx) = mpsc::channel(100);

    let tx_input = tx.clone();
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(250)).unwrap()
                && let CEvent::Key(key) = event::read().unwrap()
                && tx_input.send(AppEvent::Input(key.code)).await.is_err()
            {
                break;
            }
            if tx_input.send(AppEvent::Tick).await.is_err() {
                break;
            }
        }
    });

    // Node process
    let mut node_child = spawn_pty_reader(
        "just",
        &["remote-run", "7"],
        &[
            ("CARGO_TERM_COLOR", "always"),
            ("CLICOLOR_FORCE", "1"),
            ("DEFMT_LOG_STYLE", "always"),
        ],
        tx.clone(),
        |text, overwrite| AppEvent::NodeLog { text, overwrite },
    );

    // GW process
    let mut gw_child = spawn_pty_reader(
        "just",
        &["run-gw"],
        &[
            ("CARGO_TERM_COLOR", "always"),
            ("CLICOLOR_FORCE", "1"),
            ("DEFMT_LOG_STYLE", "always"),
        ],
        tx.clone(),
        |text, overwrite| AppEvent::GwLog { text, overwrite },
    );

    let mut app = App::new();
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Input(KeyCode::Char('q')) | AppEvent::Input(KeyCode::Esc) => {
                    app.shutting_down = true;
                    let _ = terminal.draw(|f| ui::draw(f, &app));
                    break;
                }
                AppEvent::Input(KeyCode::Tab) => app.toggle_focus(), // <--- NEW TOGGLE LOGIC
                AppEvent::Input(_) => {}
                AppEvent::NodeLog { text, overwrite } => app.add_node_log(text, overwrite),
                AppEvent::GwLog { text, overwrite } => app.add_gw_log(text, overwrite),
                AppEvent::Tick => {}
            }
        }
    }

    // Graceful Shutdown
    if let Some(pid) = node_child.process_id() {
        let _ = signal::kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT);
    }
    if let Some(pid) = gw_child.process_id() {
        let _ = signal::kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT);
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            let _ = node_child.wait();
        }),
    )
    .await;
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            let _ = gw_child.wait();
        }),
    )
    .await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
