use super::*;
use runner_terminal::terminal::{TerminalBridge as Registry, TerminalMirror};
use std::ops::Deref;

#[derive(Clone, Copy)]
pub enum UserInputMode {
    Queued,
}

pub struct TerminalSession {
    pub mirror: Arc<TerminalMirror>,
    pub model: Arc<TerminalModel>,
}
impl Deref for TerminalSession {
    type Target = TerminalMirror;
    fn deref(&self) -> &Self::Target {
        &self.mirror
    }
}
impl TerminalSession {
    pub fn attach(
        core: crate::AppCore,
        id: String,
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> runner_terminal::terminal::HostResult<Arc<Self>> {
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal(&id, (cols, rows), &core.db, &events)?;
        let model = core.sessions.terminal_model(&id)?;
        // This facade tests the pre-existing terminal behaviours synchronously; seam tests use the real frame queues.
        let mirror =
            TerminalMirror::attach(crate::daemon::InProcessTransport::client(core), id, waker)?;
        Ok(Arc::new(Self { mirror, model }))
    }
    pub fn attach_with_input_mode(
        core: crate::AppCore,
        id: String,
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
        _: UserInputMode,
    ) -> runner_terminal::terminal::HostResult<Arc<Self>> {
        Self::attach(core, id, cols, rows, waker)
    }
    pub fn feed_output(&self, event: &OutputEvent) -> runner_terminal::terminal::HostResult<()> {
        self.model
            .publish_input(self.model.feed_output(event.seq, &event.bytes));
        self.mirror.test_feed(event.seq, &event.bytes);
        self.mirror.refresh_metadata();
        Ok(())
    }
    pub fn title(&self) -> String {
        self.model.title()
    }
    pub fn live_cwd(&self) -> Option<PathBuf> {
        self.model.live_cwd().filter(|cwd| cwd.is_dir())
    }
}

pub struct TerminalBridge {
    core: crate::AppCore,
    registry: Arc<Registry>,
}
impl TerminalBridge {
    pub fn new(
        core: crate::AppCore,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> runner_terminal::terminal::HostResult<Arc<Self>> {
        let registry = Registry::new(
            crate::daemon::InProcessTransport::client(core.clone()),
            waker,
        )?;
        Ok(Arc::new(Self { core, registry }))
    }
    pub fn session(&self, id: &str) -> Option<Arc<TerminalSession>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.registry.session(id).is_none()
            && self.core.sessions.terminal_model(id).is_ok()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
        }
        let mirror = self.registry.session(id)?;
        let model = self.core.sessions.terminal_model(id).ok()?;
        Some(Arc::new(TerminalSession { mirror, model }))
    }
    pub fn set_palette(&self, palette: runner_terminal::palette::TerminalPalette) {
        self.registry.set_palette(palette);
    }
    pub fn live_session_count(&self) -> usize {
        self.registry.live_session_count()
    }
    pub fn output(&self, event: &OutputEvent) {
        if self
            .core
            .sessions
            .terminal_model(&event.session_id)
            .is_err()
        {
            let events: Arc<dyn SessionEvents> = Arc::new(self.core.session_events());
            self.core
                .sessions
                .prepare_unlisted_terminal(&event.session_id, (80, 24), &self.core.db, &events)
                .unwrap();
        }
        self.registry.attach(&event.session_id).unwrap();
        self.session(&event.session_id)
            .unwrap()
            .feed_output(event)
            .unwrap();
    }
}

pub struct TestEvents {
    core: crate::AppCore,
    bridge: Arc<TerminalBridge>,
}
impl TestEvents {
    pub fn new(core: crate::AppCore, bridge: Arc<TerminalBridge>) -> Self {
        Self { core, bridge }
    }
}
impl SessionEvents for TestEvents {
    fn spawned(&self, event: &SessionSpawnedEvent) {
        let events: Arc<dyn SessionEvents> = Arc::new(self.core.session_events());
        self.core
            .sessions
            .prepare_unlisted_terminal(
                &event.session_id,
                (event.cols, event.rows),
                &self.core.db,
                &events,
            )
            .unwrap();
        self.bridge
            .registry
            .handle_event(&runner_core::protocol::ClientEvent {
                name: "session/spawned".into(),
                payload: serde_json::to_value(event).unwrap(),
            });
    }
    fn exit(&self, event: &ExitEvent) {
        self.bridge
            .registry
            .handle_event(&runner_core::protocol::ClientEvent {
                name: "session/exit".into(),
                payload: serde_json::to_value(event).unwrap(),
            });
    }
    fn archived(&self, event: &SessionUpdatedEvent) {
        self.bridge
            .registry
            .handle_event(&runner_core::protocol::ClientEvent {
                name: "session/archived".into(),
                payload: serde_json::to_value(event).unwrap(),
            });
    }
}
impl TestEvents {
    pub fn output(&self, event: &OutputEvent) {
        self.bridge.output(event);
    }
}
