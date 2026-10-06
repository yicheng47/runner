use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use runner_core::app_paths::IpcEndpoint;
use runner_core::daemon_process::{self, NativePaths};
use runner_core::protocol::terminal::{TerminalAttachment, TerminalFrame};
use runner_core::protocol::wire::{self, Binary, Frame};
use runner_core::protocol::{ClientError, Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ipc::{IpcListener, IpcStream};
use crate::AppCore;

pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(8);
pub struct Config {
    pub paths: NativePaths,
    pub endpoint: IpcEndpoint,
    pub mcp_endpoint: IpcEndpoint,
    pub isolated: bool,
}
#[derive(serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    resume_on_launch: bool,
    disabled_agents: Vec<String>,
    enabled_agents: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            resume_on_launch: true,
            disabled_agents: Vec::new(),
            enabled_agents: Vec::new(),
        }
    }
}
impl Settings {
    fn runtimes(&self) -> Vec<crate::model::Runtime> {
        crate::model::Runtime::ALL
            .into_iter()
            .filter(|runtime| {
                !runtime.is_shell()
                    && !self.disabled_agents.iter().any(|key| key == runtime.key())
                    && (runner_core::protocol::runtime_metadata::runtime_default_enabled(*runtime)
                        || self.enabled_agents.iter().any(|key| key == runtime.key()))
            })
            .collect()
    }
}

pub fn run(config: Config) -> Result<()> {
    std::fs::create_dir_all(&config.paths.app_data_dir)?;
    let lock = daemon_process::lock_file(&config.paths.app_data_dir)?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => (),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            return Ok(())
        }
        Err(error) => return Err(error.into()),
    }
    daemon_process::clean_environment(&config.paths.app_data_dir);
    let _ = runner_core::logging::install(&config.paths.log_dir, "runnerd.log");
    runner_core::logging::startup_banner(
        &runner_core::version::display_version(),
        &config.paths.app_data_dir,
    );
    let hash = daemon_process::executable_hash(&std::env::current_exe()?)?;
    let settings: Settings = std::fs::read(config.paths.app_data_dir.join("ui-settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let core = super::boot::boot(&config.paths, settings.runtimes(), false)?;
    if config.isolated {
        core.runtime_discovery.write().unwrap().checking = false;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("runner-ipc")
        .enable_all()
        .build()?;
    let result = runtime.block_on(serve(config, core, hash, settings));
    runtime.shutdown_timeout(Duration::from_secs(1));
    drop(lock);
    result
}

struct Identity {
    #[cfg(unix)]
    endpoint: IpcEndpoint,
    #[cfg(unix)]
    file: (u64, u64),
    #[cfg(windows)]
    pipe: std::os::windows::io::OwnedHandle,
}
impl Identity {
    #[cfg(unix)]
    fn new(endpoint: IpcEndpoint) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(&endpoint.0)?;
        Ok(Self {
            endpoint,
            file: (metadata.dev(), metadata.ino()),
        })
    }
    #[cfg(windows)]
    fn new(pipe: std::os::windows::io::OwnedHandle) -> Self {
        Self { pipe }
    }
    fn owned(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            std::fs::symlink_metadata(&self.endpoint.0)
                .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.file)
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::Pipes::GetNamedPipeInfo(
                    self.pipe.as_raw_handle(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ) != 0
            }
        }
    }
    fn remove(&self) {
        #[cfg(unix)]
        if self.owned() {
            let _ = std::fs::remove_file(&self.endpoint.0);
        }
    }
}

async fn serve(config: Config, core: AppCore, hash: String, settings: Settings) -> Result<()> {
    let mut listener = IpcListener::bind(&config.endpoint)?;
    #[cfg(unix)]
    let daemon_file = Identity::new(config.endpoint.clone())?;
    #[cfg(windows)]
    let daemon_file = Identity::new(listener.duplicate_handle()?);
    core.mcp.start(
        &config.mcp_endpoint,
        core.clone(),
        &tokio::runtime::Handle::current(),
    )?;
    #[cfg(unix)]
    let mcp_file = Identity::new(config.mcp_endpoint.clone())?;
    #[cfg(windows)]
    let mcp_file = Identity::new(core.mcp.duplicate_handle()?);
    core.usage.manage_client_lifetime(config.isolated);
    if !config.isolated {
        core.usage.start_scheduler(core.clone());
    }
    let (resume_tx, resume_rx) = tokio::sync::watch::channel(None);
    let resume_consumer = Arc::new(super::resume::ResumeConsumer::default());
    let consumer = resume_consumer.clone();
    let resume_core = core.clone();
    let resume = tokio::task::spawn_blocking(move || {
        let result = super::resume::consume_resume_on_launch_until(
            &resume_core,
            settings.resume_on_launch,
            |id| {
                let conn = resume_core.db.get().ok()?;
                let row = crate::repo::session::get_row(&conn, id).ok()??;
                Some((
                    row.last_cols.unwrap_or(80) as u16,
                    row.last_rows.unwrap_or(24) as u16,
                ))
            },
            &consumer,
        );
        let _ = resume_tx.send(Some(result.as_ref().ok().cloned().unwrap_or_default()));
        result
    });
    let cancel = CancellationToken::new();
    let welcome = wire::Welcome {
        exe_sha256: hash,
        pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut connections = tokio::task::JoinSet::new();
    let window_owners = Arc::new(Mutex::new(HashMap::new()));
    let mut next_connection = 0;
    let mut ownership = tokio::time::interval(Duration::from_secs(2));
    let signal = os_shutdown();
    tokio::pin!(signal);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = &mut signal => { cancel.cancel(); break; }
            _ = ownership.tick() => {
                if !daemon_file.owned() || !mcp_file.owned() { log::error!("runnerd endpoint replaced; stopping"); cancel.cancel(); break; }
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => { if let Err(error) = result { log::warn!("runnerd connection task: {error}"); } }
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    let core = core.clone(); let welcome = welcome.clone(); let cancel = cancel.clone();
                    let isolated = config.isolated;
                    let resume_report = resume_rx.clone();
                    next_connection += 1;
                    let windows = Arc::new(ConnectionWindows { core: core.clone(), owners: window_owners.clone(), id: next_connection, closed: AtomicBool::new(false) });
                    connections.spawn(async move { if let Err(error) = connection(stream, core, welcome, cancel, isolated, windows, resume_report).await { log::debug!("runnerd client: {error}"); } });
                }
                Err(error) => { log::error!("runnerd accept: {error}"); cancel.cancel(); break; }
            }
        }
    }
    drop(listener);
    core.mcp.stop_accepting();
    core.usage.set_polling_enabled(false);
    cancel.cancel();
    let shutdown_core = core.clone();
    let teardown = tokio::task::spawn_blocking(move || -> Result<()> {
        resume_consumer.stop();
        shutdown_core.sessions.begin_shutdown();
        let mut ids = {
            let mut conn = shutdown_core.db.get()?;
            crate::repo::session::mark_running_for_resume_on_launch(&mut conn)?
        };
        for id in shutdown_core.sessions.live_session_ids() {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        shutdown_core.sessions.cancel_all_pending_spawns();
        shutdown_core.sessions.kill_many(&ids)?;
        shutdown_core.sessions.drain_terminal_workers();
        Ok(())
    });
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, teardown).await {
        Ok(result) => {
            if let Err(error) = result
                .context("join runnerd teardown")
                .and_then(|result| result)
            {
                log::error!("runnerd teardown: {error:#}");
            }
        }
        Err(_) => log::error!(
            "runnerd teardown exceeded {}s; process exit closes remaining PTYs and job handles",
            SHUTDOWN_TIMEOUT.as_secs()
        ),
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), resume).await;
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    core.mcp.stop();
    mcp_file.remove();
    daemon_file.remove();
    Ok(())
}

struct Connected {
    core: AppCore,
}
impl Drop for Connected {
    fn drop(&mut self) {
        self.core.usage.client_disconnected();
    }
}
struct Attachments {
    core: AppCore,
    ids: Mutex<HashMap<u64, (String, u64)>>,
}
impl Attachments {
    fn remove(&self, id: u64) {
        if let Some((session, subscriber)) = self.ids.lock().unwrap().remove(&id) {
            self.core.sessions.detach_terminal(&session, subscriber);
        }
    }
}
impl Drop for Attachments {
    fn drop(&mut self) {
        for (_, (session, subscriber)) in self.ids.get_mut().unwrap().drain() {
            self.core.sessions.detach_terminal(&session, subscriber);
        }
    }
}

struct ConnectionCleanup {
    cancel: CancellationToken,
    attachments: Arc<Attachments>,
    reader: tokio::task::AbortHandle,
    windows: Arc<ConnectionWindows>,
}
impl Drop for ConnectionCleanup {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.reader.abort();
        self.windows.closed.store(true, Ordering::Release);
        let windows = self.windows.clone();
        tokio::task::spawn_blocking(move || windows.remove());
        let ids = self
            .attachments
            .ids
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            self.attachments.remove(id);
        }
    }
}

struct ConnectionWindows {
    core: AppCore,
    owners: Arc<Mutex<HashMap<String, u64>>>,
    id: u64,
    closed: AtomicBool,
}

impl ConnectionWindows {
    fn dispatch(&self, request: Request) -> Option<Response> {
        if let Request::node_mark_viewed {
            window_label,
            id,
            member_ids,
            viewed_session_id,
        } = request
        {
            let response = crate::ops::node::node_mark_viewed_with(
                &self.core,
                &id,
                viewed_session_id.as_deref(),
                || {
                    let mut owners = self.owners.lock().unwrap();
                    if self.closed.load(Ordering::Acquire)
                        || owners
                            .get(&window_label)
                            .is_some_and(|owner| *owner != self.id)
                    {
                        return Err(crate::error::Error::msg(
                            "window connection closed or replaced",
                        ));
                    }
                    owners.insert(window_label.clone(), self.id);
                    self.core.windows.mark_focused(&window_label);
                    self.core.windows.set_subjects(
                        &window_label,
                        member_ids
                            .into_iter()
                            .map(crate::windows::Subject::DirectChat)
                            .collect(),
                    );
                    self.core
                        .windows
                        .set_viewed_session(&window_label, viewed_session_id.as_deref());
                    Ok(())
                },
            )
            .map_err(|error| ClientError::msg(error.to_string()));
            return Some(Response::node_mark_viewed(response));
        }
        let label = match &request {
            Request::window_register { label }
            | Request::report_subjects { label, .. }
            | Request::mark_focused { label }
            | Request::mark_blurred { label }
            | Request::unregister { label }
            | Request::window_set_mission { label, .. } => label,
            _ => return Some(super::dispatch(&self.core, request)),
        };
        // Keep cleanup and window mutations atomic, including requests already in flight.
        let mut owners = self.owners.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        if !matches!(request, Request::window_register { .. })
            && owners.get(label).is_some_and(|owner| *owner != self.id)
        {
            let error = Err(ClientError::msg("window belongs to another connection"));
            return Some(match request {
                Request::report_subjects { .. } => Response::report_subjects(error),
                Request::mark_focused { .. } => Response::mark_focused(error),
                Request::mark_blurred { .. } => Response::mark_blurred(error),
                Request::unregister { .. } => Response::unregister(error),
                Request::window_set_mission { .. } => Response::window_set_mission(error),
                _ => unreachable!(),
            });
        }
        if matches!(request, Request::unregister { .. }) {
            owners.remove(label);
        } else {
            owners.insert(label.clone(), self.id);
        }
        Some(match request {
            Request::report_subjects {
                label,
                subjects,
                viewed_session_id,
            } => {
                self.core.windows.set_subjects(&label, subjects);
                self.core
                    .windows
                    .set_viewed_session(&label, viewed_session_id.as_deref());
                let visible = self.core.windows.focused_direct_sessions(&label);
                drop(owners);
                Response::report_subjects(self.mark_viewed(&visible))
            }
            Request::mark_focused { label } => {
                self.core.windows.mark_focused(&label);
                let visible = self.core.windows.focused_direct_sessions(&label);
                drop(owners);
                Response::mark_focused(self.mark_viewed(&visible))
            }
            _ => super::dispatch(&self.core, request),
        })
    }

    fn mark_viewed(&self, visible: &[String]) -> std::result::Result<(), ClientError> {
        let result = crate::ops::node::mark_direct_sessions_viewed(&self.core, visible)
            .map_err(|error| ClientError::msg(error.to_string()));
        self.core.broadcast_focus_map();
        result
    }

    fn remove(&self) {
        let mut owners = self.owners.lock().unwrap();
        owners.retain(|label, owner| {
            if *owner != self.id {
                return true;
            }
            self.core.windows.unregister(label);
            false
        });
        self.core.broadcast_focus_map();
    }
}

async fn read_frame(read: &mut (impl tokio::io::AsyncRead + Unpin)) -> std::io::Result<Frame> {
    let len = read.read_u32_le().await? as usize;
    if !(1..=wire::MAX_FRAME).contains(&len) {
        return Err(wire::invalid("invalid frame length"));
    }
    let kind = read.read_u8().await?;
    let mut payload = vec![0; len - 1];
    read.read_exact(&mut payload).await?;
    Ok(Frame { kind, payload })
}
async fn write_frame(write: &mut (impl tokio::io::AsyncWrite + Unpin), frame: Frame) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), write.write_all(&frame.encode()?)).await??;
    Ok(())
}
async fn connection(
    stream: IpcStream,
    core: AppCore,
    welcome: wire::Welcome,
    shutdown: CancellationToken,
    _isolated: bool,
    windows: Arc<ConnectionWindows>,
    mut resume_report: tokio::sync::watch::Receiver<
        Option<runner_core::protocol::AutoResumeReport>,
    >,
) -> Result<()> {
    let (mut read, mut write) = stream.into_split();
    let hello = tokio::time::timeout(Duration::from_millis(500), read_frame(&mut read)).await??;
    if hello.kind != wire::HELLO {
        anyhow::bail!("expected Hello");
    }
    let hello: wire::Hello = hello.decode()?;
    let matches = hello.exe_sha256.is_empty() || hello.exe_sha256 == welcome.exe_sha256;
    write_frame(
        &mut write,
        Frame::json(
            if matches {
                wire::WELCOME
            } else {
                wire::MISMATCH
            },
            &welcome,
        )?,
    )
    .await?;
    // Mismatched clients must still be able to stop any daemon version.
    if !matches {
        let frame = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut read)).await??;
        if frame.kind == wire::SHUTDOWN && frame.decode::<wire::Shutdown>()?.stop_sessions {
            shutdown.cancel();
        }
        return Ok(());
    }
    let _connected = Connected { core: core.clone() };
    core.usage.client_connected(core.clone());
    let local = CancellationToken::new();
    let attachments = Arc::new(Attachments {
        core: core.clone(),
        ids: Mutex::default(),
    });
    let (control_tx, mut controls) = mpsc::channel::<Frame>(128);
    let (terminal_tx, mut terminals) = mpsc::channel::<Frame>(64);
    let mut events = core.events.subscribe();
    let report_tx = control_tx.clone();
    let report_cancel = local.clone();
    tokio::spawn(async move {
        let report = tokio::select! {
            _ = report_cancel.cancelled() => return,
            report = resume_report.wait_for(|report| report.is_some()) => report.ok().and_then(|report| report.clone()),
        };
        if let Some(report) = report {
            let event = wire::Event {
                event: Some(runner_core::protocol::ClientEvent {
                    name: "daemon/launch-resumed".into(),
                    payload: serde_json::to_value(report).unwrap(),
                }),
                lagged: None,
            };
            if let Ok(frame) = Frame::json(wire::EVENT, &event) {
                let _ = report_tx.send(frame).await;
            }
        }
    });
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Frame>();
    let input_core = core.clone();
    let input_cancel = local.clone();
    tokio::task::spawn_blocking(move || {
        while let Ok(frame) = input_rx.recv() {
            if input_cancel.is_cancelled() {
                break;
            }
            let mut binary = Binary(&frame.payload);
            let session = match binary.string() {
                Ok(session) => session,
                Err(_) => break,
            };
            let mut apply = || -> Result<()> {
                if frame.kind == wire::INPUT {
                    input_core
                        .sessions
                        .queue_terminal_input(&session, binary.0)?;
                } else {
                    let origin = binary.u64()?;
                    let cols = binary.u16()?;
                    let rows = binary.u16()?;
                    input_core.sessions.resize_terminal(
                        &session,
                        origin,
                        cols,
                        rows,
                        &input_core.db,
                    )?;
                }
                Ok(())
            };
            if let Err(error) = apply() {
                input_core.events.emit(
                    "session/input-error",
                    &serde_json::json!({"session_id": session, "message": error.to_string()}),
                );
            }
        }
    });
    let (read_tx, mut incoming) = mpsc::channel(64);
    let reader = tokio::spawn(async move {
        loop {
            let frame = read_frame(&mut read).await;
            let end = frame.is_err();
            if read_tx.send(frame).await.is_err() || end {
                break;
            }
        }
    });
    let _cleanup = ConnectionCleanup {
        cancel: local.clone(),
        attachments: attachments.clone(),
        reader: reader.abort_handle(),
        windows: windows.clone(),
    };
    let result = async {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                Some(frame) = controls.recv() => write_frame(&mut write, frame).await?,
                event = events.recv() => {
                    let event = match event {
                        Ok(event) => wire::Event { event: Some(runner_core::protocol::ClientEvent { name: event.name.into(), payload: event.payload }), lagged: None },
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => wire::Event { event: None, lagged: Some(count) },
                        Err(_) => break,
                    };
                    write_frame(&mut write, Frame::json(wire::EVENT, &event)?).await?;
                }
                received = incoming.recv() => {
                    let frame = received.context("client reader closed")??;
                    match frame.kind {
                        wire::SHUTDOWN => { if frame.decode::<wire::Shutdown>()?.stop_sessions { shutdown.cancel(); break; } }
                        wire::REQUEST => {
                            let call: wire::Call = frame.decode()?;
                            let windows = windows.clone(); let tx = control_tx.clone(); let cancel = local.clone();
                            tokio::task::spawn_blocking(move || {
                                if cancel.is_cancelled() { return; }
                                let Some(response) = windows.dispatch(call.request) else { return; };
                                if let Ok(frame) = Frame::json(wire::RESPONSE, &wire::Reply { id: call.id, response }) { let _ = tx.blocking_send(frame); }
                            });
                        }
                        wire::ATTACH => {
                            let mut binary = Binary(&frame.payload); let id = binary.u64()?; let session = binary.string()?;
                            attachments.ids.lock().unwrap().insert(id, (session.clone(), 0));
                            let core = core.clone(); let tx = terminal_tx.clone(); let controls = control_tx.clone(); let attachments = attachments.clone(); let cancel = local.clone();
                            tokio::task::spawn_blocking(move || {
                                if cancel.is_cancelled() { return; }
                                match core.sessions.attach_terminal(&session) {
                                    Ok(attachment) => {
                                        {
                                            let mut ids = attachments.ids.lock().unwrap();
                                            if let Some(entry) = ids.get_mut(&id) { entry.1 = attachment.subscriber_id; }
                                            else { core.sessions.detach_terminal(&session, attachment.subscriber_id); return; }
                                        }
                                        let _ = forward_terminal(id, attachment, &tx, &cancel);
                                        attachments.remove(id);
                                    }
                                    Err(error) => { attachments.remove(id); let mut frame = wire::numbered(wire::ATTACH_ERROR, id); frame.payload.extend_from_slice(error.to_string().as_bytes()); let _ = controls.blocking_send(frame); }
                                }
                            });
                        }
                        wire::DETACH => { attachments.remove(Binary(&frame.payload).u64()?); }
                        wire::INPUT | wire::RESIZE => { input_tx.send(frame)?; }
                        _ => anyhow::bail!("unexpected client frame"),
                    }
                }
                Some(frame) = terminals.recv() => write_frame(&mut write, frame).await?,
            }
        }
        Ok(())
    }.await;
    reader.abort();
    local.cancel();
    drop(input_tx);
    let ids = attachments
        .ids
        .lock()
        .unwrap()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        attachments.remove(id);
    }
    result
}
fn forward_terminal(
    id: u64,
    mut attachment: TerminalAttachment,
    tx: &mpsc::Sender<Frame>,
    cancel: &CancellationToken,
) -> Result<()> {
    tx.blocking_send(wire::snapshot_header(
        id,
        attachment.subscriber_id,
        &attachment.snapshot,
    ))?;
    send_chunks(
        tx,
        wire::SNAPSHOT_DATA,
        id,
        None,
        &attachment.snapshot.bytes,
    )?;
    while !cancel.is_cancelled() {
        let frame = match attachment.frames.recv() {
            Ok(TerminalFrame::Output { seq, bytes }) => {
                send_chunks(tx, wire::OUTPUT, id, Some(seq), &bytes)?;
                continue;
            }
            Ok(TerminalFrame::Resized { seq, cols, rows }) => {
                let mut frame = wire::numbered(wire::RESIZED, id);
                frame.payload.extend_from_slice(&seq.to_le_bytes());
                frame.payload.extend_from_slice(&cols.to_le_bytes());
                frame.payload.extend_from_slice(&rows.to_le_bytes());
                frame
            }
            Ok(TerminalFrame::Resync) => wire::numbered(wire::RESYNC, id),
            Err(_) => wire::numbered(wire::TERMINAL_CLOSED, id),
        };
        let end = frame.kind == wire::TERMINAL_CLOSED || frame.kind == wire::RESYNC;
        tx.blocking_send(frame)?;
        if end {
            break;
        }
    }
    Ok(())
}
fn send_chunks(
    tx: &mpsc::Sender<Frame>,
    kind: u8,
    id: u64,
    seq: Option<u64>,
    bytes: &[u8],
) -> Result<()> {
    let size = wire::TERMINAL_CHUNK - 17;
    let chunks = bytes.len().max(1).div_ceil(size);
    for index in 0..chunks {
        let mut frame = wire::numbered(kind, id);
        if let Some(seq) = seq {
            frame.payload.extend_from_slice(&seq.to_le_bytes());
        }
        frame.payload.push(u8::from(index + 1 == chunks));
        let start = (index * size).min(bytes.len());
        let end = ((index + 1) * size).min(bytes.len());
        frame.payload.extend_from_slice(&bytes[start..end]);
        tx.blocking_send(frame)?;
    }
    Ok(())
}

async fn os_shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    }
    #[cfg(windows)]
    {
        let mut close = tokio::signal::windows::ctrl_close().expect("CTRL_CLOSE handler");
        let mut logoff = tokio::signal::windows::ctrl_logoff().expect("CTRL_LOGOFF handler");
        let mut shutdown = tokio::signal::windows::ctrl_shutdown().expect("CTRL_SHUTDOWN handler");
        tokio::select! { _ = close.recv() => (), _ = logoff.recv() => (), _ = shutdown.recv() => () }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_window_requests_cannot_recreate_a_disconnected_window() {
        let core = crate::test_support::test_core();
        let windows = Arc::new(ConnectionWindows {
            core: core.clone(),
            owners: Arc::new(Mutex::default()),
            id: 1,
            closed: AtomicBool::new(false),
        });
        windows
            .dispatch(Request::window_register {
                label: "gone".into(),
            })
            .unwrap();
        let owners = windows.owners.lock().unwrap();
        let queued = windows.clone();
        let worker = std::thread::spawn(move || {
            queued.dispatch(Request::report_subjects {
                label: "gone".into(),
                subjects: vec![crate::windows::Subject::DirectChat("chat-1".into())],
                viewed_session_id: Some("chat-1".into()),
            })
        });
        windows.closed.store(true, Ordering::Release);
        drop(owners);
        windows.remove();
        assert!(worker.join().unwrap().is_none());
        assert!(core.windows.snapshot().is_empty());
        let tab = crate::repo::node::create_tab(
            &core.db.get().unwrap(),
            None,
            "chat",
            0,
            r#"{"preset":"single","slots":[],"sizes":{}}"#,
        )
        .unwrap();
        let response = windows
            .dispatch(Request::node_mark_viewed {
                window_label: "gone".into(),
                id: tab.id,
                member_ids: Vec::new(),
                viewed_session_id: None,
            })
            .unwrap();
        assert!(matches!(response, Response::node_mark_viewed(Err(_))));
        assert!(core.windows.snapshot().is_empty());
    }
}
