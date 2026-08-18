use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use anyhow::{Context as _, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use runner_backend::events::AppEvent;
use runner_backend::session::manager::OutputEvent;
use runner_backend::AppCore;
use tokio::sync::broadcast::error::RecvError;

use crate::palette;

pub struct EventProxy {
    tx: Sender<Event>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.waker)();
    }
}

pub fn query_color<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
) -> alacritty_terminal::vte::ansi::Rgb {
    query_color_for(term, index, base, palette::RUNNER)
}

pub fn query_color_for<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
    theme: palette::TerminalPalette,
) -> alacritty_terminal::vte::ansi::Rgb {
    let stored = if index < alacritty_terminal::term::color::COUNT {
        term.colors()[index]
    } else {
        None
    };
    stored.unwrap_or_else(|| palette::resolve_index_for(index, base, theme))
}

#[derive(Default)]
struct SequenceState {
    last: u64,
    synthetic_prefix_seen: bool,
}

#[derive(Clone, Copy)]
struct PaletteState {
    theme: palette::TerminalPalette,
    base: [alacritty_terminal::vte::ansi::Rgb; 256],
}

impl PaletteState {
    fn new(theme: palette::TerminalPalette) -> Self {
        Self {
            theme,
            base: palette::base_palette_for(theme),
        }
    }
}

pub struct TerminalSession {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    core: AppCore,
    session_id: String,
    feed_gate: Mutex<()>,
    parser: Mutex<Processor>,
    sequence: Mutex<SequenceState>,
    size: Arc<Mutex<(u16, u16)>>,
    title: Arc<Mutex<String>>,
    palette: Arc<Mutex<PaletteState>>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl TerminalSession {
    pub fn attach(
        core: AppCore,
        session_id: String,
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        let (tx, rx) = mpsc::channel::<Event>();
        let proxy = EventProxy {
            tx,
            waker: Arc::clone(&waker),
        };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &TermSize::new(cols as usize, rows as usize),
            proxy,
        )));
        let size = Arc::new(Mutex::new((cols, rows)));
        let title = Arc::new(Mutex::new(String::new()));
        let terminal_palette = Arc::new(Mutex::new(PaletteState::new(palette::RUNNER)));
        let session = Arc::new(Self {
            term: Arc::clone(&term),
            core: core.clone(),
            session_id: session_id.clone(),
            feed_gate: Mutex::new(()),
            parser: Mutex::new(Processor::new()),
            sequence: Mutex::new(SequenceState::default()),
            size: Arc::clone(&size),
            title: Arc::clone(&title),
            palette: Arc::clone(&terminal_palette),
            waker,
        });

        let term_for_events = Arc::downgrade(&term);
        thread::Builder::new()
            .name(format!("native-term-events-{session_id}"))
            .spawn(move || {
                let write = |bytes: &[u8]| {
                    let _ = core.sessions.inject_stdin(&session_id, bytes);
                };
                while let Ok(event) = rx.recv() {
                    match event {
                        Event::PtyWrite(text) => write(text.as_bytes()),
                        Event::ColorRequest(index, format) => {
                            let palette = *terminal_palette.lock().unwrap();
                            let rgb = term_for_events
                                .upgrade()
                                .map(|term| {
                                    query_color_for(
                                        &*term.lock_unfair(),
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    crate::palette::resolve_index_for(
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                });
                            write(format(rgb).as_bytes());
                        }
                        Event::TextAreaSizeRequest(format) => {
                            let (cols, rows) = *size.lock().unwrap();
                            let reply = format(WindowSize {
                                num_lines: rows,
                                num_cols: cols,
                                cell_width: 0,
                                cell_height: 0,
                            });
                            write(reply.as_bytes());
                        }
                        Event::Title(new_title) => {
                            *title.lock().unwrap() = new_title;
                        }
                        Event::ResetTitle => {
                            title.lock().unwrap().clear();
                        }
                        _ => {}
                    }
                }
            })
            .context("spawn terminal event thread")?;

        Ok(session)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn title(&self) -> String {
        self.title.lock().unwrap().clone()
    }

    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        let mut current = self.palette.lock().unwrap();
        if current.theme != palette {
            *current = PaletteState::new(palette);
            drop(current);
            (self.waker)();
        }
    }

    pub fn palette(&self) -> palette::TerminalPalette {
        self.palette.lock().unwrap().theme
    }

    pub fn feed_output(&self, event: &OutputEvent) -> Result<()> {
        self.feed_encoded(event.seq, &event.data)
    }

    pub fn feed_snapshot(&self, events: &[OutputEvent]) -> Result<()> {
        let _feed = self.feed_gate.lock().unwrap();
        self.feed_snapshot_locked(events)
    }

    #[cfg(test)]
    fn feed_snapshot_with_hook(&self, events: &[OutputEvent], hook: impl FnOnce()) -> Result<()> {
        let _feed = self.feed_gate.lock().unwrap();
        hook();
        self.feed_snapshot_locked(events)
    }

    fn feed_snapshot_locked(&self, events: &[OutputEvent]) -> Result<()> {
        for event in events {
            self.feed_encoded_locked(event.seq, &event.data)?;
        }
        Ok(())
    }

    fn feed_encoded(&self, seq: u64, data: &str) -> Result<()> {
        let _feed = self.feed_gate.lock().unwrap();
        self.feed_encoded_locked(seq, data)
    }

    fn feed_encoded_locked(&self, seq: u64, data: &str) -> Result<()> {
        let bytes = B64.decode(data).context("decode session output")?;
        let mut sequence = self.sequence.lock().unwrap();
        if seq == 0 {
            if sequence.synthetic_prefix_seen {
                return Ok(());
            }
            sequence.synthetic_prefix_seen = true;
        } else {
            if seq <= sequence.last {
                return Ok(());
            }
            sequence.last = seq;
        }
        let mut parser = self.parser.lock().unwrap();
        let mut term = self.term.lock();
        parser.advance(&mut *term, &bytes);
        drop(term);
        drop(parser);
        drop(sequence);
        (self.waker)();
        Ok(())
    }

    pub fn submit_text(&self, text: &str) -> runner_backend::error::Result<()> {
        self.write_user_bytes(text.as_bytes())?;
        self.write_user_bytes(b"\r")
    }

    pub fn write_user_bytes(&self, bytes: &[u8]) -> runner_backend::error::Result<()> {
        self.core
            .sessions
            .inject_direct_stdin(&self.session_id, bytes, &self.core.session_events())
    }

    pub fn send_key(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        key_char: Option<&str>,
    ) -> runner_backend::error::Result<bool> {
        let app_cursor = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::APP_CURSOR);
        match crate::mappings::encode_key(key, ctrl, alt, key_char, app_cursor) {
            Some(bytes) => {
                self.write_user_bytes(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn paste(&self, text: &str) -> runner_backend::error::Result<()> {
        let bracketed = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        self.write_user_bytes(&crate::mappings::encode_paste(text, bracketed))
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        {
            let mut size = self.size.lock().unwrap();
            if *size == (cols, rows) {
                return;
            }
            *size = (cols, rows);
        }
        let _ =
            runner_backend::ops::session::session_resize(&self.core, &self.session_id, cols, rows);
        self.term
            .lock()
            .resize(TermSize::new(cols as usize, rows as usize));
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    pub fn scroll(&self, delta_lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(delta_lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }
}

pub struct TerminalBridge {
    core: AppCore,
    attached: Mutex<HashMap<String, Weak<TerminalSession>>>,
    refresh_sessions: AtomicBool,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl TerminalBridge {
    pub fn new(core: AppCore, waker: Arc<dyn Fn() + Send + Sync>) -> Result<Arc<Self>> {
        let mut receiver = core.events.subscribe();
        let bridge = Arc::new(Self {
            core,
            attached: Mutex::new(HashMap::new()),
            refresh_sessions: AtomicBool::new(false),
            waker,
        });
        let weak: Weak<Self> = Arc::downgrade(&bridge);
        thread::Builder::new()
            .name("native-app-events".into())
            .spawn(move || loop {
                match receiver.blocking_recv() {
                    Ok(event) => {
                        let Some(bridge) = weak.upgrade() else {
                            break;
                        };
                        bridge.handle_event(event);
                    }
                    Err(RecvError::Lagged(_)) => {
                        let Some(bridge) = weak.upgrade() else {
                            break;
                        };
                        bridge.resync_attached();
                        bridge.refresh_sessions.store(true, Ordering::Release);
                        (bridge.waker)();
                    }
                    Err(RecvError::Closed) => break,
                }
            })
            .context("spawn app event thread")?;
        Ok(bridge)
    }

    pub fn attach(&self, session: Arc<TerminalSession>) -> Result<()> {
        let session_id = session.session_id().to_owned();
        let _feed = session.feed_gate.lock().unwrap();
        self.attached
            .lock()
            .unwrap()
            .insert(session_id.clone(), Arc::downgrade(&session));
        let snapshot =
            runner_backend::ops::session::session_output_snapshot(&self.core, &session_id)
                .context("load terminal output snapshot")?;
        session.feed_snapshot_locked(&snapshot)?;
        Ok(())
    }

    pub fn take_session_refresh(&self) -> bool {
        self.refresh_sessions.swap(false, Ordering::AcqRel)
    }

    fn handle_event(&self, event: AppEvent) {
        match event.name {
            "session/output" => {
                let Some(session_id) = event.payload.get("session_id").and_then(|v| v.as_str())
                else {
                    return;
                };
                let Some(seq) = event.payload.get("seq").and_then(|v| v.as_u64()) else {
                    return;
                };
                let Some(data) = event.payload.get("data").and_then(|v| v.as_str()) else {
                    return;
                };
                let session = self
                    .attached
                    .lock()
                    .unwrap()
                    .get(session_id)
                    .and_then(Weak::upgrade);
                if let Some(session) = session {
                    let _ = session.feed_encoded(seq, data);
                }
            }
            "session/exit" | "session/updated" | "session/archived" => {
                self.refresh_sessions.store(true, Ordering::Release);
                (self.waker)();
            }
            _ => {}
        }
    }

    fn resync_attached(&self) {
        let sessions = {
            let mut attached = self.attached.lock().unwrap();
            attached.retain(|_, session| session.strong_count() > 0);
            attached
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        for session in sessions {
            let _feed = session.feed_gate.lock().unwrap();
            if let Ok(snapshot) = runner_backend::ops::session::session_output_snapshot(
                &self.core,
                session.session_id(),
            ) {
                let _ = session.feed_snapshot_locked(&snapshot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::TermMode;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;
    use runner_backend::session::manager::OutputEvent;

    use super::TerminalSession;
    use crate::replay::visible_lines;
    use runner_backend::AppCore;

    /// Minimal `AppCore` over a temp dir — the pieces `boot_core` wires
    /// in runner-app, minus login-shell discovery and startup cleanup.
    fn test_core(root: &std::path::Path) -> AppCore {
        let app_data_dir = root.join("app-data");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let pool =
            Arc::new(runner_backend::db::open_pool(&app_data_dir.join("runner.db")).unwrap());
        let runtime: Arc<dyn runner_backend::session::runtime::SessionRuntime> =
            Arc::new(runner_backend::session::pty_runtime::PtyRuntime::new());
        let windows = Arc::new(runner_backend::windows::WindowRegistry::new());
        windows.register("main");
        let runtime_shell_env = Arc::new(std::sync::RwLock::new(
            runner_backend::shell_path::LoginShellEnv::default(),
        ));
        let runtime_discovery = Arc::new(std::sync::RwLock::new(
            runner_backend::shell_path::DiscoveryState::startup(None, None),
        ));
        AppCore {
            db: pool,
            app_data_dir,
            sessions: runner_backend::session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                runtime,
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: runner_backend::event_bus::BusRegistry::new(),
            routers: runner_backend::router::RouterRegistry::new(),
            mcp: Arc::new(runner_backend::mcp::McpHandle::new()),
            windows,
            events: runner_backend::events::EventChannel::new(),
            app_version: "0.0.0-test".into(),
        }
    }

    fn output(seq: u64, text: &str) -> OutputEvent {
        OutputEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            seq,
            data: B64.encode(text),
        }
    }

    #[test]
    fn snapshot_batch_blocks_newer_live_output_until_replay_finishes() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();
        let gate_held = Arc::new(Barrier::new(2));
        let live_returned = Arc::new(AtomicBool::new(false));

        let live_terminal = Arc::clone(&terminal);
        let live_barrier = Arc::clone(&gate_held);
        let live_done = Arc::clone(&live_returned);
        let live = thread::spawn(move || {
            live_barrier.wait();
            live_terminal
                .feed_output(&output(2, "live-marker"))
                .unwrap();
            live_done.store(true, Ordering::Release);
        });
        let replay_terminal = Arc::clone(&terminal);
        let replay_barrier = Arc::clone(&gate_held);
        let replay_live_returned = Arc::clone(&live_returned);
        let replay = thread::spawn(move || {
            replay_terminal
                .feed_snapshot_with_hook(&[output(1, "snapshot-marker")], || {
                    replay_barrier.wait();
                    thread::sleep(Duration::from_millis(30));
                    assert!(!replay_live_returned.load(Ordering::Acquire));
                })
                .unwrap();
        });
        replay.join().unwrap();
        live.join().unwrap();

        let rendered = {
            let term = terminal.term.lock();
            visible_lines(&*term).join("\n")
        };
        assert!(rendered.contains("snapshot-markerlive-marker"));
    }

    #[test]
    fn native_terminal_applies_resume_seam_and_reset_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();

        terminal
            .feed_output(&output(
                1,
                "history-marker\x1b[?2004h\x1b[?1000h\x1b[?1006h",
            ))
            .unwrap();
        {
            let term = terminal.term.lock();
            assert!(term.mode().contains(TermMode::BRACKETED_PASTE));
            assert!(term.mode().intersects(TermMode::MOUSE_MODE));
            assert!(term.mode().contains(TermMode::SGR_MOUSE));
        }

        terminal
            .feed_output(&output(
                2,
                "\x1b[0m\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\r\n",
            ))
            .unwrap();
        {
            let term = terminal.term.lock();
            assert!(visible_lines(&*term).join("\n").contains("history-marker"));
            assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
            assert!(!term.mode().intersects(TermMode::MOUSE_MODE));
            assert!(!term.mode().contains(TermMode::SGR_MOUSE));
        }

        terminal.feed_output(&output(3, "\x1bc")).unwrap();
        let term = terminal.term.lock();
        assert!(!visible_lines(&*term).join("\n").contains("history-marker"));
        assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
        assert!(!term.mode().intersects(TermMode::MOUSE_MODE));
        assert!(!term.mode().contains(TermMode::SGR_MOUSE));
    }
}
