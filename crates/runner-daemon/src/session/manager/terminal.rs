use std::collections::VecDeque;
use std::sync::mpsc;

use runner_core::protocol::terminal::{
    TerminalAttachment, TerminalFrame, TerminalMetadata, TerminalPalette, TerminalSubscription,
};
use runner_core::protocol::ClientError;
use runner_terminal::terminal::{
    ClipboardTarget, HostResult, ModelOptions, TerminalHost, TerminalModel,
};

use super::*;

const FRAME_CAPACITY: usize = 64;
// A closed stream can detach after its session state was pruned and recreated.
static NEXT_SUBSCRIBER_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Default, PartialEq)]
pub(super) enum Controller {
    #[default]
    Desktop,
    Browser {
        channel: u64,
        name: String,
    },
    None,
}
impl Controller {
    pub(super) fn wire(&self) -> runner_core::protocol::terminal::TerminalController {
        use runner_core::protocol::terminal::TerminalController as Wire;
        match self {
            Self::Desktop => Wire::Desktop,
            Self::Browser { channel, name } => Wire::Browser {
                channel: *channel,
                name: name.clone(),
            },
            Self::None => Wire::None,
        }
    }
}
#[derive(Default)]
pub(super) struct ControlState {
    attachments: HashMap<(u64, String, u64), String>,
    groups: HashMap<String, Controller>,
    compositions: HashMap<String, u64>,
}

pub(super) enum ResizeControl {
    Desktop,
    TakeBack,
    Browser {
        channel: u64,
        name: String,
        take: bool,
        viewport: bool,
    },
}
impl ResizeControl {
    pub(super) fn apply(self, controller: &mut Controller) -> bool {
        match self {
            Self::Desktop => *controller == Controller::Desktop,
            Self::TakeBack => {
                *controller = Controller::Desktop;
                true
            }
            Self::Browser {
                channel,
                name,
                take,
                viewport,
            } => {
                let owns = matches!(controller, Controller::Browser { channel: owner, .. } if *owner == channel);
                if take || owns && viewport || !viewport && *controller == Controller::None {
                    *controller = Controller::Browser { channel, name };
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Default)]
struct QueueState {
    frames: VecDeque<TerminalFrame>,
    closed: bool,
}
#[derive(Default)]
pub(super) struct FrameQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}
impl FrameQueue {
    fn push(&self, frame: TerminalFrame) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return false;
        }
        if state.frames.len() == FRAME_CAPACITY {
            state.frames.clear();
            state.frames.push_back(TerminalFrame::Resync);
            state.closed = true;
        } else {
            state.frames.push_back(frame);
        }
        self.ready.notify_one();
        !state.closed
    }
    #[cfg(test)]
    pub(super) fn drain(&self) -> Vec<TerminalFrame> {
        self.state.lock().unwrap().frames.drain(..).collect()
    }
    pub(super) fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_one();
    }
}
struct Frames(Arc<FrameQueue>);
impl Drop for Frames {
    fn drop(&mut self) {
        self.0.close();
    }
}
impl TerminalSubscription for Frames {
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let queue = self.0.clone();
        Arc::new(move || queue.close())
    }
    fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
        let mut state = self.0.state.lock().unwrap();
        loop {
            if let Some(frame) = state.frames.pop_front() {
                return Ok(frame);
            }
            if state.closed {
                return Err(ClientError::msg("terminal stream closed"));
            }
            state = self.0.ready.wait(state).unwrap();
        }
    }
}

pub struct CompanionTerminal {
    state: Weak<Mutex<SessionState>>,
    history: usize,
    final_snapshot: Mutex<Option<runner_core::protocol::terminal::TerminalSnapshot>>,
}
impl CompanionTerminal {
    pub fn attach(&self) -> Result<TerminalAttachment> {
        let state = self.state.upgrade();
        let mut state = state.as_ref().map(|state| state.lock().unwrap());
        if let Some(snapshot) = self.final_snapshot.lock().unwrap().take() {
            let queue = Arc::new(FrameQueue::default());
            queue.close();
            return Ok(TerminalAttachment {
                subscriber_id: 0,
                snapshot,
                frames: Box::new(Frames(queue)),
            });
        }
        let state = state
            .as_mut()
            .ok_or_else(|| Error::msg("terminal released"))?;
        SessionManager::attach_terminal_locked(state, self.history)
    }
}
impl SessionState {
    pub(super) fn release_terminal(&mut self) -> Vec<thread::JoinHandle<()>> {
        self.finish_companion_terminals();
        self.terminal_input.take();
        self.terminal_started_at.take();
        let mut workers = self
            .terminal
            .take()
            .map(|model| model.stop_workers())
            .unwrap_or_default();
        workers.extend(self.terminal_input_worker.take());
        for (_, queue) in self.subscribers.drain(..) {
            queue.close();
        }
        workers
    }

    pub(super) fn finish_companion_terminals(&mut self) {
        for source in self
            .companion_terminals
            .drain(..)
            .filter_map(|source| source.upgrade())
        {
            if let Some(model) = &self.terminal {
                let mut snapshot = model.snapshot_with_history(self.output_seq, source.history);
                snapshot.controller = self.controller.wire();
                *source.final_snapshot.lock().unwrap() = Some(snapshot);
            }
        }
    }
}

pub(super) type TerminalStartedAt = Arc<Mutex<Option<String>>>;

pub(crate) struct ProviderTitleUpdate<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) expected_started_at: &'a str,
    pub(crate) cleaned_title: &'a str,
    pub(crate) mission_id: Option<&'a str>,
    pub(crate) pool: &'a DbPool,
    pub(crate) events: &'a Arc<dyn SessionEvents>,
}

fn persist_live_title_for_generation(
    pool: &DbPool,
    session_id: &str,
    cleaned_title: &str,
    expected_started_at: &str,
) -> Result<Option<(bool, Option<String>)>> {
    let conn = pool.get()?;
    let updated = crate::repo::session::set_live_title(
        &conn,
        session_id,
        Some(cleaned_title),
        Some(expected_started_at),
    )?;
    let row = crate::repo::session::get_row(&conn, session_id)?;
    if updated > 0 {
        return Ok(Some((true, row.and_then(|row| row.mission_id))));
    }
    let Some(row) = row.filter(|row| {
        row.started_at.map(|time| time.to_rfc3339()).as_deref() == Some(expected_started_at)
            && row.live_title.as_deref() == Some(cleaned_title)
    }) else {
        return Ok(None);
    };
    Ok(Some((false, row.mission_id)))
}

struct Host {
    manager: Weak<SessionManager>,
    pool: DbPool,
    events: Arc<dyn SessionEvents>,
    agent: bool,
    started_at: TerminalStartedAt,
}
impl TerminalHost for Host {
    fn write_input(&self, id: &str, bytes: &[u8]) -> HostResult<()> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| Error::msg("session manager stopped"))?;
        Ok(manager.inject_direct_stdin(id, bytes, self.events.as_ref())?)
    }
    fn write_reply(&self, id: &str, bytes: &[u8]) -> HostResult<()> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| Error::msg("session manager stopped"))?;
        Ok(manager.inject_stdin(id, bytes)?)
    }
    fn report_input_state(&self, id: &str, observation: InputObservation) {
        if let Some(manager) = self.manager.upgrade() {
            manager.report_input_state(id, observation);
        }
    }
    fn set_live_title(&self, id: &str, title: &str) -> HostResult<()> {
        if !self.agent {
            self.events.terminal_metadata_changed(id);
            return Ok(());
        }
        let expected_started_at = self.started_at.lock().unwrap().clone();
        let conn = self.pool.get()?;
        if crate::repo::session::set_live_title(
            &conn,
            id,
            (!title.is_empty()).then_some(title),
            expected_started_at.as_deref(),
        )? == 0
        {
            return Ok(());
        }
        let mission_id = crate::repo::session::get_row(&conn, id)?.and_then(|row| row.mission_id);
        drop(conn);
        self.events.updated(&SessionUpdatedEvent {
            session_id: id.to_owned(),
            mission_id,
        });
        Ok(())
    }
    fn resize(&self, id: &str, cols: u16, rows: u16) -> HostResult<()> {
        if let Some(manager) = self.manager.upgrade() {
            manager.resize(id, cols, rows, &Arc::new(self.pool.clone()))?;
        }
        Ok(())
    }
    fn is_local_host(&self, host: &str) -> bool {
        crate::shell_integration::is_local_host(host)
    }
    fn clipboard_store(&self, id: &str, target: ClipboardTarget, text: &str) {
        self.events.clipboard(id, target, text);
    }
}

struct SessionTerminalBinding {
    runtime: Option<Runtime>,
    agent: bool,
    cwd: Option<String>,
    initial_title: String,
    started_at: Option<String>,
}

fn session_terminal_binding(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<SessionTerminalBinding> {
    let runtime = crate::repo::session::effective_runtime(conn, id)?
        .as_deref()
        .and_then(Runtime::parse);
    let row = crate::repo::session::get_row(conn, id)?;
    let agent = runtime.is_some_and(|runtime| !runtime.is_shell());
    let cwd = row.as_ref().and_then(|row| row.cwd.clone());
    let initial_title = row
        .as_ref()
        .filter(|_| agent)
        .and_then(|row| row.live_title.as_deref())
        .and_then(|title| crate::session::title::provider_title(title, cwd.as_deref()))
        .unwrap_or_default();
    let started_at = row
        .and_then(|row| row.started_at)
        .map(|time| time.to_rfc3339());
    Ok(SessionTerminalBinding {
        runtime,
        agent,
        cwd,
        initial_title,
        started_at,
    })
}

impl SessionManager {
    pub fn discard_unlisted_terminal(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
    pub fn prepare_unlisted_terminal(
        self: &Arc<Self>,
        id: &str,
        size: (u16, u16),
        pool: &DbPool,
        events: &Arc<dyn SessionEvents>,
    ) -> Result<()> {
        let (model, started_at) =
            self.create_terminal_with_started_at(id, size.0, size.1, pool, events)?;
        let (input, worker) = self.start_terminal_input(id, Arc::clone(&model), Arc::clone(events));
        let mut control = self.terminal_control.lock().unwrap();
        let controller = self.inherited_controller(id, pool, &mut control)?;
        let state = self.session_state_or_insert(id);
        let mut state = state.lock().unwrap();
        state.finish_companion_terminals();
        for (_, queue) in state.subscribers.drain(..) {
            queue.close();
        }
        state.last_exit_was_killed = false;
        state.controller = controller;
        state.terminal = Some(model);
        state.terminal_started_at = Some(started_at);
        state.terminal_input = Some(input);
        state.terminal_input_worker = Some(worker);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn create_terminal(
        self: &Arc<Self>,
        id: &str,
        cols: u16,
        rows: u16,
        pool: &DbPool,
        events: &Arc<dyn SessionEvents>,
    ) -> Result<Arc<TerminalModel>> {
        self.create_terminal_with_started_at(id, cols, rows, pool, events)
            .map(|(model, _)| model)
    }
    pub(super) fn create_terminal_with_started_at(
        self: &Arc<Self>,
        id: &str,
        cols: u16,
        rows: u16,
        pool: &DbPool,
        events: &Arc<dyn SessionEvents>,
    ) -> Result<(Arc<TerminalModel>, TerminalStartedAt)> {
        let conn = pool.get()?;
        let binding = session_terminal_binding(&conn, id)?;
        drop(conn);
        let started_at = Arc::new(Mutex::new(binding.started_at));
        let program_status_authority = binding.runtime.is_some_and(|runtime| {
            crate::runtimes::adapter(runtime)
                .status_semantics()
                .program_status_authority
        });
        self.session_state_or_insert(id)
            .lock()
            .unwrap()
            .model
            .set_program_status_authority(program_status_authority);
        let host = Arc::new(Host {
            manager: Arc::downgrade(self),
            pool: pool.clone(),
            events: Arc::clone(events),
            agent: binding.agent,
            started_at: Arc::clone(&started_at),
        });
        let model = TerminalModel::new(
            host,
            id.to_owned(),
            cols,
            rows,
            ModelOptions {
                codex_native_attention: binding.runtime.is_some_and(|runtime| {
                    crate::runtimes::adapter(runtime)
                        .status_semantics()
                        .native_attention
                }),
                agent: binding.agent,
                shell: binding.runtime.is_some_and(Runtime::is_shell),
                initial_title: binding.initial_title,
                cwd: binding.cwd,
            },
        )
        .map_err(|error| Error::msg(error.to_string()))?;
        model.set_palette(*self.terminal_palette.lock().unwrap());
        Ok((model, started_at))
    }
    pub(super) fn rebind_terminal_session(
        &self,
        id: &str,
        terminal: &TerminalModel,
        started_at: &TerminalStartedAt,
        pool: &DbPool,
    ) -> Result<()> {
        let conn = pool.get()?;
        let binding = session_terminal_binding(&conn, id)?;
        drop(conn);
        let started_at = Arc::clone(started_at);
        let next_started_at = binding.started_at;
        terminal.sync_session_title(binding.initial_title, move || {
            *started_at.lock().unwrap() = next_started_at;
        });
        Ok(())
    }
    pub(crate) fn apply_provider_title(&self, update: ProviderTitleUpdate<'_>) -> Result<bool> {
        let attached = self.session_state(update.session_id).and_then(|state| {
            let state = state.lock().unwrap();
            state
                .terminal
                .clone()
                .zip(state.terminal_started_at.clone())
        });
        if let Some((terminal, terminal_started_at)) = attached {
            let (tx, rx) = mpsc::sync_channel(1);
            let session_id = update.session_id.to_owned();
            let expected = update.expected_started_at.to_owned();
            let fallback_mission = update.mission_id.map(str::to_owned);
            let pool = update.pool.clone();
            let events = Arc::clone(update.events);
            let applied = terminal.apply_session_title(
                update.cleaned_title.to_owned(),
                move |cleaned, cache_changed| {
                    if terminal_started_at.lock().unwrap().as_deref() != Some(expected.as_str()) {
                        let _ = tx.send(Ok(false));
                        return None;
                    }
                    match persist_live_title_for_generation(&pool, &session_id, cleaned, &expected)
                    {
                        Ok(Some((db_updated, row_mission))) => {
                            let mission_id = row_mission.or(fallback_mission);
                            Some(Box::new(move || {
                                if db_updated || cache_changed {
                                    events.updated(&SessionUpdatedEvent {
                                        session_id,
                                        mission_id,
                                    });
                                }
                                let _ = tx.send(Ok(true));
                            }))
                        }
                        Ok(None) => {
                            let _ = tx.send(Ok(false));
                            None
                        }
                        Err(error) => {
                            let _ = tx.send(Err(error));
                            None
                        }
                    }
                },
            );
            if applied {
                if let Ok(outcome) = rx.recv() {
                    return outcome;
                }
            }
        }
        match persist_live_title_for_generation(
            update.pool,
            update.session_id,
            update.cleaned_title,
            update.expected_started_at,
        )? {
            Some((db_updated, row_mission)) => {
                if db_updated {
                    update.events.updated(&SessionUpdatedEvent {
                        session_id: update.session_id.to_owned(),
                        mission_id: row_mission.or_else(|| update.mission_id.map(str::to_owned)),
                    });
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }
    pub(crate) fn terminal_output_sequence(&self, id: &str) -> u64 {
        self.session_state(id)
            .map(|state| state.lock().unwrap().output_seq)
            .unwrap_or(0)
    }

    pub fn terminal_model(&self, id: &str) -> Result<Arc<TerminalModel>> {
        let state = self
            .session_state(id)
            .ok_or_else(|| Error::msg(format!("session not found: {id}")))?;
        let model = state
            .lock()
            .unwrap()
            .terminal
            .clone()
            .ok_or_else(|| Error::msg(format!("terminal not found: {id}")))?;
        Ok(model)
    }
    pub fn attach_terminal(&self, id: &str) -> Result<TerminalAttachment> {
        self.attach_terminal_with_history(id, usize::MAX)
    }
    pub fn attach_terminal_with_history(
        &self,
        id: &str,
        history: usize,
    ) -> Result<TerminalAttachment> {
        let state = self
            .session_state(id)
            .ok_or_else(|| Error::msg(format!("session not found: {id}")))?;
        let mut state = state.lock().unwrap();
        Self::attach_terminal_locked(&mut state, history)
    }
    pub fn attach_companion_terminal(
        &self,
        id: &str,
        history: usize,
    ) -> Result<(TerminalAttachment, Arc<CompanionTerminal>)> {
        let state = self
            .session_state(id)
            .ok_or_else(|| Error::msg("terminal not found"))?;
        let source = Arc::new(CompanionTerminal {
            state: Arc::downgrade(&state),
            history: history.min(5000),
            final_snapshot: Mutex::new(None),
        });
        let mut state = state.lock().unwrap();
        let attachment = Self::attach_terminal_locked(&mut state, source.history)?;
        state
            .companion_terminals
            .retain(|source| source.strong_count() != 0);
        state.companion_terminals.push(Arc::downgrade(&source));
        Ok((attachment, source))
    }
    fn attach_terminal_locked(
        state: &mut SessionState,
        history: usize,
    ) -> Result<TerminalAttachment> {
        let model = state
            .terminal
            .as_ref()
            .ok_or_else(|| Error::msg("terminal not found"))?;
        let mut snapshot = model.snapshot_with_history(state.output_seq, history);
        snapshot.controller = state.controller.wire();
        let subscriber_id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
        let queue = Arc::new(FrameQueue::default());
        state.subscribers.push((subscriber_id, Arc::clone(&queue)));
        Ok(TerminalAttachment {
            subscriber_id,
            snapshot,
            frames: Box::new(Frames(queue)),
        })
    }
    pub fn detach_terminal(&self, id: &str, subscriber: u64) {
        if let Some(state) = self.session_state(id) {
            state.lock().unwrap().subscribers.retain(|(found, queue)| {
                if *found == subscriber {
                    queue.close();
                    false
                } else {
                    true
                }
            });
        }
    }
    #[cfg(test)]
    pub(crate) fn record_terminal_input(&self, id: &str) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        self.session_state(id)
            .unwrap()
            .lock()
            .unwrap()
            .terminal_input = Some(tx);
        rx
    }

    pub fn desktop_input(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let _control = self.terminal_control.lock().unwrap();
        if !self
            .session_state(id)
            .is_some_and(|state| state.lock().unwrap().controller == Controller::Desktop)
        {
            return Err(Error::msg("terminal is controlled elsewhere"));
        }
        self.queue_terminal_input(id, bytes)
    }

    pub fn observe_terminal_input(
        &self,
        id: &str,
        connection: u64,
        input: runner_core::protocol::terminal::InputEvent,
    ) -> Result<()> {
        use runner_core::protocol::terminal::InputEvent;
        let mut control = self.terminal_control.lock().unwrap();
        let model = {
            let state = self
                .session_state(id)
                .ok_or_else(|| Error::msg("terminal not found"))?;
            let state = state.lock().unwrap();
            if state.controller != Controller::Desktop {
                return Err(Error::msg("terminal is controlled elsewhere"));
            }
            state
                .terminal
                .clone()
                .ok_or_else(|| Error::msg("terminal not found"))?
        };
        match input {
            InputEvent::Composing { composing: true } => {
                if control
                    .compositions
                    .get(id)
                    .is_some_and(|owner| *owner != connection)
                {
                    self.clear_composition(id, &mut control);
                }
                control.compositions.insert(id.to_owned(), connection);
            }
            InputEvent::Composing { composing: false } => {
                if control
                    .compositions
                    .get(id)
                    .is_some_and(|owner| *owner != connection)
                {
                    return Ok(());
                }
                control.compositions.remove(id);
            }
            _ => {}
        }
        model.observe_input(&input);
        Ok(())
    }

    fn clear_composition(&self, id: &str, control: &mut ControlState) {
        if control.compositions.remove(id).is_some() {
            if let Ok(model) = self.terminal_model(id) {
                model.observe_input(&runner_core::protocol::terminal::InputEvent::Composing {
                    composing: false,
                });
                self.notify_delivery_event(id, router::SessionDeliveryEvent::InputCleared);
            }
        }
    }

    pub(crate) fn clear_connection_compositions(&self, connection: u64) {
        let mut control = self.terminal_control.lock().unwrap();
        let ids: Vec<_> = control
            .compositions
            .iter()
            .filter(|(_, owner)| **owner == connection)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.clear_composition(&id, &mut control);
        }
    }

    pub(crate) fn take_terminal_input_control(
        &self,
        id: &str,
        connection: u64,
        pool: &DbPool,
    ) -> Result<()> {
        let mut control = self.terminal_control.lock().unwrap();
        for id in self.control_group(id, pool)?.1 {
            if control
                .compositions
                .get(&id)
                .is_some_and(|owner| *owner != connection)
            {
                self.clear_composition(&id, &mut control);
            }
        }
        Ok(())
    }

    fn queue_terminal_input(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let state = self
            .session_state(id)
            .ok_or_else(|| Error::msg(format!("session not found: {id}")))?;
        let state = state.lock().unwrap();
        state
            .terminal_input
            .as_ref()
            .ok_or_else(|| Error::msg(format!("terminal input unavailable: {id}")))?
            .send(bytes.to_vec())
            .map_err(|_| Error::msg(format!("terminal input worker stopped: {id}")))
    }
    pub(super) fn start_terminal_input(
        &self,
        id: &str,
        model: Arc<TerminalModel>,
        events: Arc<dyn SessionEvents>,
    ) -> (mpsc::Sender<Vec<u8>>, thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let id = id.to_owned();
        let worker = thread::Builder::new()
            .name(format!("native-term-input-{id}"))
            .spawn(move || {
                while let Ok(bytes) = rx.recv() {
                    if let Err(error) = model.write_input(&bytes) {
                        events.input_error(&id, &error.to_string());
                    }
                }
            })
            .expect("spawn terminal input thread");
        (tx, worker)
    }
    pub(super) fn push_frame(state: &mut SessionState, frame: TerminalFrame) {
        state
            .subscribers
            .retain(|(_, queue)| queue.push(frame.clone()));
    }
    pub fn resize_terminal(
        &self,
        id: &str,
        _origin: u64,
        cols: u16,
        rows: u16,
        pool: &Arc<DbPool>,
    ) -> Result<()> {
        // The sender also adopts the accepted size from the ordered frame stream.
        self.resize(id, cols, rows, pool)
    }
    fn control_group(&self, id: &str, pool: &DbPool) -> Result<(String, Vec<String>)> {
        let conn = pool.get()?;
        let tab = crate::repo::node::find_for_session(&conn, id)?;
        Ok(match tab {
            Some(tab) => (tab.id.clone(), crate::repo::node::session_ids(&tab)),
            None => (format!("session:{id}"), vec![id.to_owned()]),
        })
    }

    pub(super) fn inherited_controller(
        &self,
        id: &str,
        pool: &DbPool,
        control: &mut ControlState,
    ) -> Result<Controller> {
        let (group, ids) = self.control_group(id, pool)?;
        let controller = control.groups.entry(group).or_insert_with(|| {
            ids.iter()
                .filter(|other| other.as_str() != id)
                .filter_map(|other| self.session_state(other))
                .find_map(|state| {
                    let state = state.lock().unwrap();
                    state
                        .terminal_input
                        .as_ref()
                        .map(|_| state.controller.clone())
                })
                .unwrap_or_default()
        });
        Ok(controller.clone())
    }

    pub(crate) fn reconcile_tab_control(&self, tab_id: &str, pool: &DbPool) -> Result<()> {
        let mut control = self.terminal_control.lock().unwrap();
        let conn = pool.get()?;
        let Some(tab) = crate::repo::node::get(&conn, tab_id)? else {
            return Ok(());
        };
        let controller = control.groups.entry(tab.id.clone()).or_default().clone();
        let ids = crate::repo::node::session_ids(&tab);
        for id in &ids {
            control.groups.remove(&format!("session:{id}"));
        }
        self.set_group_controller(&ids, "", &controller, &mut control);
        Ok(())
    }

    fn set_group_controller(
        &self,
        ids: &[String],
        except: &str,
        controller: &Controller,
        control: &mut ControlState,
    ) {
        for id in ids.iter().filter(|id| id.as_str() != except) {
            let Some(state) = self.session_state(id) else {
                continue;
            };
            let mut state = state.lock().unwrap();
            if state.terminal_input.is_none() || state.controller == *controller {
                continue;
            }
            state.controller = controller.clone();
            Self::push_controller(&mut state);
            drop(state);
            self.clear_composition(id, control);
        }
    }

    fn push_controller(state: &mut SessionState) {
        if let Some(model) = &state.terminal {
            let (cols, rows) = model.size();
            state.output_seq += 1;
            Self::push_frame(
                state,
                TerminalFrame::Resized {
                    seq: state.output_seq,
                    cols,
                    rows,
                    controller: state.controller.wire(),
                },
            );
        }
    }

    pub fn browser_input(&self, id: &str, channel: u64, bytes: &[u8]) -> Result<()> {
        let _control = self.terminal_control.lock().unwrap();
        let model = {
            let state = self
                .session_state(id)
                .ok_or_else(|| Error::msg("terminal not found"))?;
            let state = state.lock().unwrap();
            if !matches!(state.controller, Controller::Browser { channel: owner, .. } if owner == channel)
            {
                return Err(Error::msg("terminal is controlled elsewhere"));
            }
            state
                .terminal
                .clone()
                .ok_or_else(|| Error::msg("terminal not found"))?
        };
        model.observe_input(&crate::companion::classify_input(bytes));
        self.queue_terminal_input(id, bytes)
    }

    pub fn browser_attached(
        &self,
        id: &str,
        subscriber: u64,
        channel: u64,
        pool: &DbPool,
    ) -> Result<()> {
        let mut attachments = self.terminal_control.lock().unwrap();
        let (group, _) = self.control_group(id, pool)?;
        attachments
            .attachments
            .insert((channel, id.to_owned(), subscriber), group);
        Ok(())
    }

    pub fn browser_detached(
        &self,
        id: &str,
        subscriber: u64,
        channel: u64,
        release: bool,
        pool: &DbPool,
    ) -> Result<()> {
        let mut attachments = self.terminal_control.lock().unwrap();
        let group = match attachments
            .attachments
            .remove(&(channel, id.to_owned(), subscriber))
        {
            Some(group) => group,
            None if subscriber == 0 => self.control_group(id, pool)?.0,
            None => return Ok(()),
        };
        if !release {
            return Ok(());
        }
        let conn = pool.get()?;
        let ids = if let Some(tab) = crate::repo::node::get(&conn, &group)? {
            crate::repo::node::session_ids(&tab)
        } else if group == format!("session:{id}") {
            vec![id.to_owned()]
        } else {
            Vec::new()
        };
        drop(conn);
        let current_group = self.control_group(id, pool)?.0;
        if current_group.starts_with("session:")
            && current_group != group
            && !attachments
                .attachments
                .keys()
                .any(|(owner, member, _)| *owner == channel && member == id)
        {
            self.release_browser_locked(id, channel);
        }
        if attachments
            .attachments
            .iter()
            .any(|((owner, _, _), other)| *owner == channel && *other == group)
        {
            return Ok(());
        }
        if matches!(attachments.groups.get(&group), Some(Controller::Browser { channel: owner, .. }) if *owner == channel)
        {
            attachments.groups.insert(group, Controller::None);
        }
        for id in ids {
            self.release_browser_locked(&id, channel);
        }
        Ok(())
    }

    pub fn take_back_terminal(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        pool: &Arc<DbPool>,
    ) -> Result<()> {
        let mut control = self.terminal_control.lock().unwrap();
        let (group, ids) = self.control_group(id, pool)?;
        control.groups.insert(group, Controller::Desktop);
        self.set_group_controller(&ids, id, &Controller::Desktop, &mut control);
        self.resize_controlled(id, cols, rows, pool, ResizeControl::TakeBack)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn browser_resize(
        &self,
        id: &str,
        channel: u64,
        name: &str,
        cols: u16,
        rows: u16,
        take: bool,
        viewport: bool,
        pool: &Arc<DbPool>,
    ) -> Result<()> {
        let mut control = self.terminal_control.lock().unwrap();
        self.terminal_model(id)?;
        if take || !viewport {
            let (group, ids) = self.control_group(id, pool)?;
            let free = ids
                .iter()
                .filter_map(|id| self.session_state(id))
                .all(|state| {
                    let state = state.lock().unwrap();
                    state.terminal_input.is_none() || state.controller == Controller::None
                });
            if !take
                && !viewport
                && !free
                && self
                    .session_state(id)
                    .is_some_and(|state| state.lock().unwrap().controller == Controller::None)
            {
                return Ok(());
            }
            if take || !viewport && free {
                control.groups.insert(
                    group,
                    Controller::Browser {
                        channel,
                        name: name.into(),
                    },
                );
                self.set_group_controller(
                    &ids,
                    id,
                    &Controller::Browser {
                        channel,
                        name: name.into(),
                    },
                    &mut control,
                );
                self.clear_composition(id, &mut control);
            }
        }
        self.resize_controlled(
            id,
            cols.clamp(20, 512),
            rows.clamp(8, 256),
            pool,
            ResizeControl::Browser {
                channel,
                name: name.into(),
                take,
                viewport,
            },
        )
    }
    #[cfg(test)]
    pub fn release_browser(&self, id: &str, channel: u64) {
        let _control = self.terminal_control.lock().unwrap();
        self.release_browser_locked(id, channel);
    }
    fn release_browser_locked(&self, id: &str, channel: u64) {
        if let Some(state) = self.session_state(id) {
            let mut state = state.lock().unwrap();
            if matches!(state.controller, Controller::Browser { channel: owner, .. } if owner == channel)
            {
                state.controller = Controller::None;
                Self::push_controller(&mut state);
            }
        }
    }
    pub fn release_browser_channel(&self, channel: u64) {
        let mut attachments = self.terminal_control.lock().unwrap();
        attachments
            .attachments
            .retain(|(owner, _, _), _| *owner != channel);
        for controller in attachments.groups.values_mut() {
            if matches!(controller, Controller::Browser { channel: owner, .. } if *owner == channel)
            {
                *controller = Controller::None;
            }
        }
        let ids: Vec<_> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.release_browser_locked(&id, channel);
        }
    }
    pub fn terminal_was_stopped(&self, id: &str) -> bool {
        self.session_state(id).is_some_and(|state| {
            let state = state.lock().unwrap();
            state.killed || state.last_exit_was_killed
        })
    }
    pub fn browser_controller(&self, id: &str, channel: u64) -> runner_core::companion::Controller {
        use runner_core::companion::Controller as Wire;
        self.session_state(id)
            .map(|state| match &state.lock().unwrap().controller {
                Controller::Desktop => Wire::Desktop,
                Controller::None => Wire::None,
                Controller::Browser { channel: owner, .. } if *owner == channel => Wire::You,
                Controller::Browser { name, .. } => Wire::Browser { name: name.clone() },
            })
            .unwrap_or(Wire::None)
    }
}

pub fn metadata(core: &crate::AppCore, id: &str) -> Result<TerminalMetadata> {
    let state = core
        .sessions
        .session_state(id)
        .ok_or_else(|| Error::msg("terminal not found"))?;
    let state = state.lock().unwrap();
    let model = state
        .terminal
        .as_ref()
        .ok_or_else(|| Error::msg("terminal not found"))?;
    let (cols, rows) = model.size();
    Ok(TerminalMetadata {
        title: model.title(),
        live_cwd: model.live_cwd(),
        link_cwd: None,
        cols,
        rows,
        controller: state.controller.wire(),
    })
}
pub fn link_cwd(core: &crate::AppCore, id: &str) -> Result<Option<PathBuf>> {
    let conn = core.db.get()?;
    let Some(row) = crate::repo::session::get_row(&conn, id)? else {
        return Ok(None);
    };
    if let Some(cwd) = row.cwd.filter(|cwd| !cwd.trim().is_empty()) {
        return Ok(Some(PathBuf::from(cwd)));
    }
    let Some(project_id) = row.project_id else {
        return Ok(None);
    };
    Ok(crate::repo::project::get(&conn, &project_id)?.map(|project| PathBuf::from(project.cwd)))
}
pub fn configure(core: &crate::AppCore, id: &str, scrollback: usize, shape: &str) -> Result<()> {
    core.sessions
        .terminal_model(id)?
        .configure_named(scrollback, shape);
    Ok(())
}
pub fn set_palette(core: &crate::AppCore, palette: TerminalPalette) -> Result<()> {
    if let Err(error) = crate::db::set_terminal_palette(&core.db, &palette) {
        log::warn!("persist terminal palette: {error}");
    }
    let palette = runner_terminal::terminal::palette_from_wire(palette);
    let previous = std::mem::replace(
        &mut *core.sessions.terminal_palette.lock().unwrap(),
        palette,
    );
    for id in core.sessions.live_session_ids() {
        if let Ok(model) = core.sessions.terminal_model(&id) {
            model.set_palette(palette);
        }
    }
    if previous != palette {
        core.events.emit("terminal/palette", &());
    }
    Ok(())
}
pub fn palette(core: &crate::AppCore) -> TerminalPalette {
    runner_terminal::terminal::palette_to_wire(*core.sessions.terminal_palette.lock().unwrap())
}
/// Sessions resumed at daemon start answer color queries before any app
/// connects, so they start from the palette the app last sent.
pub fn restore_palette(core: &crate::AppCore) {
    match crate::db::terminal_palette(&core.db) {
        Ok(Some(palette)) => {
            *core.sessions.terminal_palette.lock().unwrap() =
                runner_terminal::terminal::palette_from_wire(palette);
        }
        Ok(None) => {}
        Err(error) => log::warn!("restore terminal palette: {error}"),
    }
}

#[cfg(test)]
mod support;
#[cfg(test)]
mod tests;
