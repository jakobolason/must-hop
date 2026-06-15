pub mod app;
pub mod components;
pub mod composables;
pub mod navigator;
pub mod pages;
pub mod ui;

use crate::mpsc::Sender;
use crate::navigator::Navigator;
use app::GatewayConfigFocus;
use app::{App, AppEvent, ProcessDescriptor};
use crossterm::event::{self, Event as CEvent, KeyCode};
use navigator::{DashFocus, LandingSection, LandingSubView, NavigatorView};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver};

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

pub type ChildProcess = Box<dyn portable_pty::Child + Send + Sync>;
pub fn spawn_pty_reader(
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
    // let program = args[0].to_string().clone();

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

pub fn spawn_log_processes(
    descriptors: &[ProcessDescriptor],
    tx: Sender<AppEvent>,
) -> Vec<ChildProcess> {
    descriptors
        .iter()
        .map(|d| {
            let args: Vec<&str> = d.args.iter().map(String::as_str).collect();
            let envs: Vec<(&str, &str)> = d
                .envs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let source_id = d.source_id.clone();
            spawn_pty_reader(
                &d.command,
                &args,
                &envs,
                tx.clone(),
                move |text, overwrite| AppEvent::ProcessLog {
                    source: source_id.clone(),
                    text,
                    overwrite,
                },
            )
        })
        .collect()
}

pub fn initialize(
    interactive: bool,
    din_map: &'static [(&'static str, u8)],
) -> (App, Sender<AppEvent>, Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel(100);
    init_logger();

    if interactive {
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
    }
    let app = App::new(interactive, din_map);

    (app, tx, rx)
}

pub async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    din_map: &'static [(&'static str, u8)],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut navigator = Navigator::new();

    let (mut app, tx, mut rx) = initialize(true, din_map);
    let mut log_children = Vec::new();
    let mut delay_child = None;

    let quit_fn = |app: &mut App, ngtr: &mut Navigator, terminal: &mut Terminal<_>| {
        ngtr.reset_scrolls();
        app.reset_data();
        let _ = terminal.draw(|f| ui::draw(f, app, ngtr));
    };

    loop {
        terminal.draw(|f| ui::draw(f, &app, &navigator))?;

        if let Some(event) = rx.recv().await {
            match event {
                AppEvent::Input(key_code) => match navigator.view {
                    NavigatorView::Landing => match navigator.landing_sub_view {
                        LandingSubView::ProbeList => match key_code {
                            KeyCode::Esc => {
                                shutdown_processes(
                                    log_children
                                        .drain(..)
                                        .map(Some)
                                        .chain([delay_child.take()])
                                        .collect(),
                                )
                                .await;
                                quit_fn(&mut app, &mut navigator, terminal);
                                break;
                            }
                            KeyCode::Up => {
                                navigator.landing_up(app.available_probes.len());
                            }
                            KeyCode::Down => {
                                navigator.landing_down(
                                    app.available_probes.len(),
                                    app.configured_nodes.len(),
                                );
                            }
                            KeyCode::Enter => match navigator.landing_section {
                                LandingSection::Probes => {
                                    let cursor = navigator.probe_list_cursor;
                                    if cursor < app.available_probes.len() {
                                        app.start_configuring_probe(cursor);
                                        navigator.probe_config_focus = app::ProbeConfigFocus::Kp;
                                        navigator.landing_sub_view = LandingSubView::ProbeConfig;
                                    }
                                }
                                LandingSection::Gateway => {
                                    navigator.gateway_config_focus = GatewayConfigFocus::Sf;
                                    navigator.landing_sub_view = LandingSubView::GatewayConfig;
                                }
                                LandingSection::Nodes => {
                                    let cursor = navigator.node_list_cursor;
                                    if cursor < app.configured_nodes.len() {
                                        app.start_editing_node(cursor);
                                        navigator.probe_config_focus = app::ProbeConfigFocus::Kp;
                                        navigator.landing_sub_view = LandingSubView::ProbeConfig;
                                    }
                                }
                            },
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                use navigator::LandingSection;
                                let idx = match navigator.landing_section {
                                    LandingSection::Nodes => navigator.node_list_cursor,
                                    LandingSection::Probes | LandingSection::Gateway => {
                                        app.configured_nodes.len().saturating_sub(1)
                                    }
                                };
                                app.remove_configured_node(idx);
                                navigator.clamp_node_cursor(app.configured_nodes.len());
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                app.fetch_probes();
                                navigator.probe_list_cursor = 0;
                            }
                            KeyCode::Char('s') | KeyCode::Char('S')
                                if !app.configured_nodes.is_empty() =>
                            {
                                app.reset_data();
                                app.save_to_env_file();
                                navigator.view = NavigatorView::Dashboard;
                                let descriptors = app.build_descriptors();
                                log::info!("Built descriptors!");
                                app.init_sources(&descriptors);
                                log_children = spawn_log_processes(&descriptors, tx.clone());
                                delay_child = Some(spawn_pty_reader(
                                    "just",
                                    &["run-delay"],
                                    &[],
                                    tx.clone(),
                                    |delay_ms, _| AppEvent::HardwareLog { delay_ms },
                                ));
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') if app.has_data() => {
                                app.save_data();
                                app.reset_data();
                            }
                            _ => {}
                        },
                        LandingSubView::ProbeConfig => match key_code {
                            KeyCode::Esc => {
                                app.pending_node = None;
                                app.editing_node_index = None;
                                navigator.landing_sub_view = LandingSubView::ProbeList;
                            }
                            KeyCode::Tab | KeyCode::Down => navigator.next_config_focus(),
                            KeyCode::BackTab | KeyCode::Up => navigator.prev_config_focus(),
                            KeyCode::Enter => {
                                // if navigator.probe_config_focus == app::ProbeConfigFocus::Confirm {
                                app.confirm_pending_node();
                                navigator.landing_sub_view = LandingSubView::ProbeList;
                                // } else {
                                //     navigator.next_config_focus();
                                // }
                            }
                            KeyCode::Backspace => {
                                app.backspace_pending(navigator.probe_config_focus);
                            }
                            KeyCode::Char(c) => {
                                app.type_char_pending(c, navigator.probe_config_focus);
                            }
                            _ => {}
                        },
                        LandingSubView::GatewayConfig => match key_code {
                            KeyCode::Esc => {
                                navigator.landing_sub_view = LandingSubView::ProbeList;
                            }
                            KeyCode::Tab | KeyCode::Down => navigator.next_gateway_focus(),
                            KeyCode::BackTab | KeyCode::Up => navigator.prev_gateway_focus(),
                            KeyCode::Enter => {
                                navigator.landing_sub_view = LandingSubView::ProbeList;
                                navigator.landing_section = LandingSection::Gateway;
                            }
                            KeyCode::Backspace => {
                                app.backspace_gateway(navigator.gateway_config_focus);
                            }
                            KeyCode::Char(c) => {
                                app.type_char_gateway(c, navigator.gateway_config_focus);
                            }
                            _ => {}
                        },
                    },
                    NavigatorView::Dashboard => match key_code {
                        KeyCode::Char('q') => {
                            quit_fn(&mut app, &mut navigator, terminal);
                            break;
                        }
                        KeyCode::Esc => {
                            shutdown_processes(
                                log_children
                                    .drain(..)
                                    .map(Some)
                                    .chain([delay_child.take()])
                                    .collect(),
                            )
                            .await;
                            navigator.view = NavigatorView::Landing;
                        }
                        KeyCode::Tab => navigator.toggle_dash_focus(),
                        KeyCode::Up if navigator.dash_focus == DashFocus::Data => {
                            navigator.scroll_history_up()
                        }
                        KeyCode::Down if navigator.dash_focus == DashFocus::Data => {
                            navigator.scroll_history_down()
                        }
                        KeyCode::Up if navigator.dash_focus == DashFocus::Logs => {
                            navigator.scroll_logs_up()
                        }
                        KeyCode::Down if navigator.dash_focus == DashFocus::Logs => {
                            navigator.scroll_logs_down()
                        }
                        KeyCode::Left => navigator.scroll_graph_back(
                            app.node_stats
                                .first()
                                .map_or(0, |ns| ns.stats.packets.len()),
                        ),
                        KeyCode::Right => navigator.scroll_graph_forward(),
                        KeyCode::Char('p') => {
                            // Stops the processes, but doesn't go back to landing. They can still
                            // press esc to go back
                            shutdown_processes(
                                log_children
                                    .drain(..)
                                    .map(Some)
                                    .chain([delay_child.take()])
                                    .collect(),
                            )
                            .await;
                        }
                        _ => {}
                    },
                },
                AppEvent::ProcessLog {
                    source,
                    text,
                    overwrite,
                } => app.add_log(&source, text, overwrite),
                AppEvent::HardwareLog { delay_ms } => app.add_hw_delay(delay_ms),
                AppEvent::Tick => {}
            }
        }
    }

    // Graceful Shutdown (Only attempt to kill if they were spawned)
    shutdown_processes(
        log_children
            .into_iter()
            .map(Some)
            .chain([delay_child])
            .collect(),
    )
    .await;

    Ok(())
}

pub async fn shutdown_processes(children: Vec<Option<ChildProcess>>) {
    for mut child in children.into_iter().flatten() {
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
