pub mod app;
pub mod components;
pub mod composables;
pub mod navigator;
pub mod pages;
pub mod ui;

use crate::mpsc::Sender;
use crate::navigator::Navigator;
use app::{App, AppEvent};
use crossterm::event::{self, Event as CEvent, KeyCode};
use navigator::{LandingFocus, NavigatorView};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::time::Duration;
use tokio::sync::mpsc;

pub fn init_logger() {
    let _ = simplelog::WriteLogger::init(
        simplelog::LevelFilter::Debug, // Change to Info or Trace as needed
        simplelog::Config::default(),
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/must-dash-debug.log")
            .expect("Failed to open debug log file"),
    );
}

type ChildProcess = Box<dyn portable_pty::Child + Send + Sync>;
fn spawn_pty_reader(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    tx: mpsc::Sender<AppEvent>,
    event_mapper: impl Fn(String, bool) -> AppEvent + Send + 'static,
) -> ChildProcess {
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
    let program = args[0].to_string().clone();

    tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        let mut pending_cr = false;
        loop {
            if let Ok(0) | Err(_) = master_reader.read(&mut byte) {
                break;
            }
            match byte[0] {
                b'\r' => {
                    // log::debug!(
                    //     "CR (pending_cr was: {}), buf so far: {:?}",
                    //     pending_cr,
                    //     String::from_utf8_lossy(&buf)
                    // );
                    if pending_cr {
                        // Back-to-back \r\r — flush previous as overwrite
                        if !buf.is_empty() {
                            let line = String::from_utf8_lossy(&buf).into_owned();
                            let _ = tx.blocking_send(event_mapper(line, true));
                            buf.clear();
                        }
                    }
                    pending_cr = true;
                }
                b'\n' => {
                    pending_cr = false;
                    if !buf.is_empty() {
                        let line = String::from_utf8_lossy(&buf).into_owned();
                        // log::debug!("NEWLINE: {:?}", line);
                        let is_erase_line = buf.starts_with(b"\x1b[2K");
                        let _ = tx.blocking_send(event_mapper(line, is_erase_line));
                        buf.clear();
                    }
                }
                b'\x08' => {
                    pending_cr = false;
                    buf.pop();
                }
                b => {
                    if pending_cr {
                        // \r was standalone (progress overwrite) — flush buffer now
                        if !buf.is_empty() {
                            let line = String::from_utf8_lossy(&buf).into_owned();
                            let _ = tx.blocking_send(event_mapper(line, true));
                            buf.clear();
                        }
                        pending_cr = false;
                    }
                    // if byte[0] < 0x20 || byte[0] == 0x7f {
                    //     log::debug!("CTRL: 0x{:02x}", byte[0]);
                    // }
                    buf.push(b);
                }
            }
        }
    });

    child
}

fn spawn_children(
    app: &App,
    tx: Sender<AppEvent>,
) -> (
    Option<ChildProcess>,
    Option<ChildProcess>,
    Option<ChildProcess>,
) {
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

    let delay_child = spawn_pty_reader("just", &["run-delay"], &[], tx.clone(), |delay_ms, _| {
        AppEvent::HardwareLog { delay_ms }
    });

    (Some(node_child), Some(gw_child), Some(delay_child))
}

pub async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (tx, mut rx) = mpsc::channel(100);
    init_logger();

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
    let mut navigator = Navigator::new();

    // Hold our process handles in Options, they start as None
    let mut node_child: Option<ChildProcess> = None;
    let mut gw_child: Option<ChildProcess> = None;
    let mut delay_child: Option<ChildProcess> = None;

    let quit_fn = |app: &mut App, ngtr: &mut Navigator, terminal: &mut Terminal<_>| {
        ngtr.shutting_down = true;
        app.reset_data();
        let _ = terminal.draw(|f| ui::draw(f, app, ngtr));
    };

    loop {
        terminal.draw(|f| ui::draw(f, &app, &navigator))?;

        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Input(key_code) => match navigator.view {
                    NavigatorView::Landing => match key_code {
                        KeyCode::Esc => {
                            // Shut them down if they exist
                            shutdown_processes(vec![
                                node_child.take(),
                                gw_child.take(),
                                delay_child.take(),
                            ])
                            .await;
                            quit_fn(&mut app, &mut navigator, terminal);
                            match navigator.view {
                                NavigatorView::Landing => break,
                                NavigatorView::Dashboard => navigator.view = NavigatorView::Landing,
                            }
                        }
                        KeyCode::Up | KeyCode::BackTab => navigator.prev_landing_focus(),
                        KeyCode::Down | KeyCode::Tab => navigator.next_landing_focus(),
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            app.save_data();
                            app.reset_data();
                        }
                        KeyCode::Enter => {
                            if navigator.landing_focus == LandingFocus::Save {
                                app.save_data();
                            } else {
                                // Resets logs and data for new run
                                app.reset_data();
                                navigator.view = NavigatorView::Dashboard;
                                (node_child, gw_child, delay_child) =
                                    spawn_children(&app, tx.clone());
                            }
                        }
                        KeyCode::Backspace => app.backspace(navigator.landing_focus),
                        KeyCode::Char(c) => app.type_char(c, navigator.landing_focus),
                        _ => {}
                    },
                    NavigatorView::Dashboard => match key_code {
                        KeyCode::Char('q') => {
                            quit_fn(&mut app, &mut navigator, terminal);
                            break;
                        }
                        KeyCode::Esc => {
                            shutdown_processes(vec![
                                node_child.take(),
                                gw_child.take(),
                                delay_child.take(),
                            ])
                            .await;
                            navigator.view = NavigatorView::Landing;
                        }
                        KeyCode::Tab => navigator.toggle_dash_focus(),
                        _ => {}
                    },
                },
                AppEvent::NodeLog { text, overwrite } => app.add_node_log(text, overwrite),
                AppEvent::GwLog { text, overwrite } => app.add_gw_log(text, overwrite),
                AppEvent::HardwareLog { delay_ms } => app.add_hw_delay(delay_ms),
                AppEvent::Tick => {}
            }
        }
    }

    // Graceful Shutdown (Only attempt to kill if they were spawned)
    shutdown_processes(vec![node_child, gw_child, delay_child]).await;

    Ok(())
}

async fn shutdown_processes(children: Vec<Option<ChildProcess>>) {
    for child_opt in children {
        if let Some(mut child) = child_opt {
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
}
