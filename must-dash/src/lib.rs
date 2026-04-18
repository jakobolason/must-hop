// src/lib.rs
pub mod app;
pub mod ui;

use crate::mpsc::Sender;
use app::{App, AppEvent, AppView, LandingFocus};
use crossterm::event::{self, Event as CEvent, KeyCode};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::time::Duration;
use tokio::sync::mpsc;

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
type ChildProcess = Box<dyn portable_pty::Child + Send + Sync>;
fn spawn_children(app: &App, tx: Sender<AppEvent>) -> (Option<ChildProcess>, Option<ChildProcess>) {
    let node_child = spawn_pty_reader(
        "just",
        &["remote-run", "7"],
        &[
            ("CARGO_TERM_COLOR", "always"),
            ("CLICOLOR_FORCE", "1"),
            ("DEFMT_LOG_STYLE", "always"),
            ("KP", &app.env_vars.kp),
            ("KI", &app.env_vars.ki),
            ("SOURCEID", &app.env_vars.source_id),
        ],
        tx.clone(),
        |text, overwrite| AppEvent::NodeLog { text, overwrite },
    );

    let gw_child = spawn_pty_reader(
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
    (Some(node_child), Some(gw_child))
}

pub async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
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

    let mut app = App::new();

    // Hold our process handles in Options, they start as None
    let mut node_child: Option<ChildProcess> = None;
    let mut gw_child: Option<ChildProcess> = None;

    let quit_fn = |app: &mut App, terminal: &mut Terminal<_>| {
        app.shutting_down = true;
        let _ = terminal.draw(|f| ui::draw(f, app));
    };

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Input(key_code) => match app.view {
                    AppView::Landing => match key_code {
                        KeyCode::Esc => {
                            // Shut them down if they exist
                            shutdown_processes(node_child.take(), gw_child.take()).await;
                            quit_fn(&mut app, terminal);
                            match app.view {
                                AppView::Landing => break,
                                AppView::Dashboard => app.view = AppView::Landing,
                            }
                        }
                        KeyCode::Up => app.prev_landing_focus(),
                        KeyCode::Down | KeyCode::Tab => app.next_landing_focus(),
                        KeyCode::Enter => {
                            if app.landing_focus == LandingFocus::Start {
                                app.view = AppView::Dashboard;
                                (node_child, gw_child) = spawn_children(&app, tx.clone());
                            }
                        }
                        KeyCode::Backspace => app.backspace(),
                        KeyCode::Char(c) => app.type_char(c),
                        _ => {}
                    },
                    AppView::Dashboard => match key_code {
                        KeyCode::Char('q') => {
                            quit_fn(&mut app, terminal);
                            break;
                        }
                        KeyCode::Esc => {
                            shutdown_processes(node_child.take(), gw_child.take()).await;
                            app.view = AppView::Landing;
                        }
                        KeyCode::Tab => app.toggle_dash_focus(),
                        _ => {}
                    },
                },
                AppEvent::NodeLog { text, overwrite } => app.add_node_log(text, overwrite),
                AppEvent::GwLog { text, overwrite } => app.add_gw_log(text, overwrite),
                AppEvent::Tick => {}
            }
        }
    }

    // Graceful Shutdown (Only attempt to kill if they were spawned)
    shutdown_processes(node_child, gw_child).await;

    Ok(())
}

async fn shutdown_processes(node_child: Option<ChildProcess>, gw_child: Option<ChildProcess>) {
    if let Some(mut child) = node_child {
        if let Some(pid) = child.process_id() {
            let _ = signal::kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT);
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                let _ = child.wait();
            }),
        )
        .await;
    }

    if let Some(mut child) = gw_child {
        if let Some(pid) = child.process_id() {
            let _ = signal::kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT);
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                let _ = child.wait();
            }),
        )
        .await;
    }
}

