// runner-pty-host — sidecar binary that owns PTY allocation, agent
// processes, and the headless terminal model for Runner. The Tauri app
// talks to it over a private Unix socket.
//
// Plan: docs/impls/0011-pty-host-terminal-runtime.md (Steps 2 + 3).
//
// Architecture summary:
//   * One `runner-pty-host` process per app; survives Tauri restarts
//     thanks to setsid + double-fork in `--detach` mode and an exclusive
//     `fs2` lockfile on the data dir.
//   * One reader thread per session: reads raw PTY bytes from a
//     `try_clone`d master fd, feeds them through
//     `alacritty_terminal::vte::ansi::Processor::advance` against a
//     shared `Term<HostListener>` (so an `Attach` can serialize the
//     screen as ANSI), then broadcasts the raw bytes verbatim to all
//     connected subscribers and appends them to the session's
//     durability tape (`terminal.ndjson`).
//   * One reader + one writer thread per IPC connection. Inbound
//     requests are dispatched against the shared `HostState`; outbound
//     messages (request responses and live `HostEvent`s) ride a single
//     `mpsc` per connection, drained by the writer thread.
//
// Initial platform scope is macOS + Linux. Windows is out of scope for
// v1; the `cfg(unix)` gates below make that explicit and the Tauri side
// chooses the tmux fallback if it ever runs the app there.

#![cfg(unix)]

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::Config;
use alacritty_terminal::tty::{self, Options as TtyOptions};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};
use alacritty_terminal::Term;
use fs2::FileExt;
use runner_core::pty_host::{
    HostEvent, HostMessage, HostRequest, HostResponse, HostSessionStatus, HostSnapshot,
    SpawnSpecWire, TerminalReplayEvent,
};

const SOCKET_NAME: &str = "runner-pty-host.sock";
const LOCK_NAME: &str = "pty-host.lock";
const STALE_SOCKET_CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
const FRAME_MAX_BYTES: u32 = 16 * 1024 * 1024;
const PTY_READ_BUF: usize = 8 * 1024;
const TERMINAL_LOG_NAME: &str = "terminal.ndjson";
const DEFAULT_SCROLLING_HISTORY: usize = 10_000;

struct Args {
    socket_dir: PathBuf,
    detach: bool,
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("runner-pty-host: {msg}");
            eprintln!();
            eprintln!("Usage: runner-pty-host --socket-dir <PATH> [--detach]");
            eprintln!("  --socket-dir   Directory the lockfile, socket, and per-session");
            eprintln!("                 replay logs live under (typically");
            eprintln!("                 `<app_data>/pty-host`).");
            eprintln!("  --detach       Daemonize via setsid + double-fork before binding.");
            process::exit(64);
        }
    };

    if let Err(err) = run(args) {
        eprintln!("runner-pty-host: fatal: {err}");
        process::exit(1);
    }
}

fn run(args: Args) -> io::Result<()> {
    fs::create_dir_all(&args.socket_dir)?;
    fs::set_permissions(&args.socket_dir, fs::Permissions::from_mode(0o700))?;

    let sessions_dir = args.socket_dir.join("sessions");
    fs::create_dir_all(&sessions_dir)?;
    fs::set_permissions(&sessions_dir, fs::Permissions::from_mode(0o700))?;

    let lock_path = args.socket_dir.join(LOCK_NAME);
    let socket_path = args.socket_dir.join(SOCKET_NAME);

    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    match lock_file.try_lock_exclusive() {
        Ok(()) => {}
        Err(_) => {
            eprintln!(
                "runner-pty-host: another host already holds {}, exiting",
                lock_path.display()
            );
            return Ok(());
        }
    }

    if socket_path.exists() {
        if !is_socket_alive(&socket_path) {
            let _ = fs::remove_file(&socket_path);
        } else {
            return Err(io::Error::other(format!(
                "stale socket {} appears live but lock is unclaimed; refusing to bind",
                socket_path.display()
            )));
        }
    }

    if args.detach {
        // SAFETY: only this thread is alive (no sessions yet) and we
        // don't touch FDs beyond what the daemonize routine expects.
        unsafe { daemonize()? };
    }

    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    std::mem::forget(lock_file);

    let host = Arc::new(HostState {
        sessions: Mutex::new(HashMap::new()),
        sessions_dir,
    });

    eprintln!(
        "runner-pty-host: listening on {} (pid={})",
        socket_path.display(),
        process::id()
    );

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let host = host.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_connection(host, stream) {
                        eprintln!("runner-pty-host: connection error: {err}");
                    }
                });
            }
            Err(err) => {
                eprintln!("runner-pty-host: accept error: {err}");
            }
        }
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut socket_dir: Option<PathBuf> = None;
    let mut detach = false;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--socket-dir" => {
                let v = iter.next().ok_or("--socket-dir requires a value")?;
                socket_dir = Some(PathBuf::from(v));
            }
            "--detach" => detach = true,
            "--help" | "-h" => return Err("help requested".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        socket_dir: socket_dir.ok_or("--socket-dir is required")?,
        detach,
    })
}

fn is_socket_alive(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(STALE_SOCKET_CONNECT_TIMEOUT));
            true
        }
        Err(_) => false,
    }
}

// --- Host state ---------------------------------------------------------

struct HostState {
    sessions: Mutex<HashMap<String, Arc<SessionHandle>>>,
    sessions_dir: PathBuf,
}

struct SessionHandle {
    session_id: String,
    pty: Mutex<tty::Pty>,
    writer: Arc<Mutex<File>>,
    term: Arc<FairMutex<Term<HostListener>>>,
    cols: AtomicU16,
    rows: AtomicU16,
    seq: AtomicU64,
    command: String,
    pid: u32,
    exit_code: Mutex<Option<i32>>,
    alive: AtomicBool,
    subscribers: Mutex<Vec<mpsc::Sender<HostMessage>>>,
    log: Mutex<BufWriter<File>>,
}

/// Local impl of `alacritty_terminal::event::EventListener`. The only
/// `AlacEvent` we act on is `PtyWrite`: when the parser auto-responds to
/// queries like DA1, CSI 6 n, or color requests, we forward those bytes
/// back into the child via the shared writer. All other events
/// (Title/Bell/Wakeup/ChildExit/etc.) are dropped — the reader thread
/// handles child-exit separately via EOF detection.
#[derive(Clone)]
struct HostListener {
    writer: Arc<Mutex<File>>,
}

impl EventListener for HostListener {
    fn send_event(&self, event: AlacEvent) {
        if let AlacEvent::PtyWrite(s) = event {
            if let Ok(mut w) = self.writer.lock() {
                let _ = w.write_all(s.as_bytes());
            }
        }
    }
}

/// `Dimensions` impl carrying just `(cols, rows)`. Used to build `Term`
/// and to call `Term::resize`.
#[derive(Clone, Copy)]
struct HostSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for HostSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

// --- Per-connection handling -------------------------------------------

fn handle_connection(host: Arc<HostState>, stream: UnixStream) -> io::Result<()> {
    let read_half = stream.try_clone()?;
    let writer_stream = stream;

    let (out_tx, out_rx) = mpsc::channel::<HostMessage>();
    let writer_shutdown = Arc::new(AtomicBool::new(false));

    // Writer thread drains the mpsc and writes framed JSON. Exits when
    // the channel closes (all senders dropped — happens when the reader
    // thread + every subscriber clone is gone) or when a socket write
    // errors. Either way it shuts the socket so the reader thread also
    // unblocks.
    let writer_stream_clone = writer_stream.try_clone()?;
    let writer_shutdown_for_thread = writer_shutdown.clone();
    let writer_handle = thread::spawn(move || {
        let mut writer = BufWriter::new(writer_stream_clone);
        while let Ok(msg) = out_rx.recv() {
            let payload = match serde_json::to_vec(&msg) {
                Ok(b) => b,
                Err(err) => {
                    eprintln!("runner-pty-host: serialize HostMessage: {err}");
                    break;
                }
            };
            if write_frame(&mut writer, &payload).is_err() || writer.flush().is_err() {
                break;
            }
        }
        writer_shutdown_for_thread.store(true, Ordering::Release);
        let _ = writer.get_ref().shutdown(Shutdown::Both);
    });

    let mut reader = BufReader::new(read_half);
    let reader_tx = out_tx.clone();
    let result = run_reader(&host, &mut reader, reader_tx);

    // Reader exits: close the socket so the writer thread unblocks.
    let _ = writer_stream.shutdown(Shutdown::Both);
    drop(out_tx);
    let _ = writer_handle.join();
    let _ = writer_shutdown; // silence unused-binding warning (kept for symmetry)
    result
}

fn run_reader<R: BufRead>(
    host: &Arc<HostState>,
    reader: &mut R,
    out_tx: mpsc::Sender<HostMessage>,
) -> io::Result<()> {
    loop {
        let frame = match read_frame(reader)? {
            Some(bytes) => bytes,
            None => return Ok(()),
        };
        let request: HostRequest = match serde_json::from_slice(&frame) {
            Ok(r) => r,
            Err(err) => {
                let _ = out_tx.send(HostMessage::Response(HostResponse::Error {
                    message: format!("invalid request: {err}"),
                }));
                continue;
            }
        };
        let response = dispatch(host, &out_tx, request);
        if out_tx.send(HostMessage::Response(response)).is_err() {
            return Ok(());
        }
    }
}

// --- Request dispatch --------------------------------------------------

fn dispatch(
    host: &Arc<HostState>,
    out_tx: &mpsc::Sender<HostMessage>,
    request: HostRequest,
) -> HostResponse {
    match request {
        HostRequest::Spawn { spec } => match spawn_session(host, spec) {
            Ok((session_id, pid)) => HostResponse::Spawned { session_id, pid },
            Err(err) => HostResponse::Error {
                message: format!("spawn failed: {err}"),
            },
        },
        HostRequest::Attach { session_id } => match attach_session(host, &session_id, out_tx) {
            Ok(snapshot) => HostResponse::Snapshot(snapshot),
            Err(err) => HostResponse::Error {
                message: format!("attach failed: {err}"),
            },
        },
        HostRequest::Input {
            session_id,
            data_base64,
        }
        | HostRequest::Paste {
            session_id,
            data_base64,
        } => {
            // v1: paste is plain bytes plus bracketed-paste wrapping
            // when the bracketed-paste mode bit is set. For Step 3 we
            // collapse paste onto plain input and revisit in Step 8.
            let bytes = match base64_decode(&data_base64) {
                Ok(b) => b,
                Err(err) => {
                    return HostResponse::Error {
                        message: format!("input base64: {err}"),
                    };
                }
            };
            match write_input(host, &session_id, &bytes) {
                Ok(()) => HostResponse::Ack,
                Err(err) => HostResponse::Error {
                    message: format!("input failed: {err}"),
                },
            }
        }
        HostRequest::Key { session_id, key } => {
            // v1: key translation is the client's responsibility; this
            // path is reserved for future xterm-side decoding. Plumb
            // it through `Input` for now.
            match write_input(host, &session_id, key.as_bytes()) {
                Ok(()) => HostResponse::Ack,
                Err(err) => HostResponse::Error {
                    message: format!("key failed: {err}"),
                },
            }
        }
        HostRequest::Resize {
            session_id,
            cols,
            rows,
        } => match resize_session(host, &session_id, cols, rows) {
            Ok(()) => HostResponse::Ack,
            Err(err) => HostResponse::Error {
                message: format!("resize failed: {err}"),
            },
        },
        HostRequest::Stop { session_id } => match stop_session(host, &session_id) {
            Ok(()) => HostResponse::Ack,
            Err(err) => HostResponse::Error {
                message: format!("stop failed: {err}"),
            },
        },
        HostRequest::Status { session_id } => match session_status(host, &session_id) {
            Ok(status) => HostResponse::SessionStatus(status),
            Err(err) => HostResponse::Error {
                message: format!("status failed: {err}"),
            },
        },
        HostRequest::List => {
            let sessions = host
                .sessions
                .lock()
                .expect("sessions mutex poisoned")
                .values()
                .map(session_status_from_handle)
                .collect();
            HostResponse::Sessions { sessions }
        }
    }
}

// --- Spawn --------------------------------------------------------------

fn spawn_session(
    host: &Arc<HostState>,
    spec: SpawnSpecWire,
) -> io::Result<(String, u32)> {
    let session_id = new_ulid();
    let session_dir = host.sessions_dir.join(&session_id);
    fs::create_dir_all(&session_dir)?;
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700))?;

    let log_path = session_dir.join(TERMINAL_LOG_NAME);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log = Mutex::new(BufWriter::new(log_file));

    let tty_options = TtyOptions {
        shell: Some(tty::Shell::new(spec.command.clone(), spec.args.clone())),
        working_directory: spec.cwd.clone().map(PathBuf::from),
        drain_on_exit: true,
        env: spec.env.clone().into_iter().collect(),
    };

    let window_size = WindowSize {
        num_lines: spec.rows,
        num_cols: spec.cols,
        cell_width: 0,
        cell_height: 0,
    };

    let pty = tty::new(&tty_options, window_size, session_id_to_window_id(&session_id))
        .map_err(|err| io::Error::other(format!("tty::new: {err}")))?;
    let pid = pty.child().id();

    let reader_file = pty.file().try_clone()?;
    let writer_file = pty.file().try_clone()?;
    let writer = Arc::new(Mutex::new(writer_file));

    let listener = HostListener {
        writer: writer.clone(),
    };
    let config = Config {
        scrolling_history: DEFAULT_SCROLLING_HISTORY,
        ..Config::default()
    };
    let size = HostSize {
        cols: spec.cols as usize,
        rows: spec.rows as usize,
    };
    let term = Arc::new(FairMutex::new(Term::new(config, &size, listener)));

    let command_summary = format!(
        "{} {}",
        spec.command,
        spec.args.join(" ").trim_end()
    )
    .trim()
    .to_string();

    let handle = Arc::new(SessionHandle {
        session_id: session_id.clone(),
        pty: Mutex::new(pty),
        writer,
        term: term.clone(),
        cols: AtomicU16::new(spec.cols),
        rows: AtomicU16::new(spec.rows),
        seq: AtomicU64::new(0),
        command: command_summary,
        pid,
        exit_code: Mutex::new(None),
        alive: AtomicBool::new(true),
        subscribers: Mutex::new(Vec::new()),
        log,
    });

    host.sessions
        .lock()
        .expect("sessions mutex poisoned")
        .insert(session_id.clone(), handle.clone());

    let handle_for_thread = handle.clone();
    thread::Builder::new()
        .name(format!("pty-host-reader-{session_id}"))
        .spawn(move || session_reader_thread(handle_for_thread, reader_file))?;

    Ok((session_id, pid))
}

fn session_reader_thread(handle: Arc<SessionHandle>, mut reader_file: File) {
    let mut processor: Processor = Processor::new();
    let mut buf = vec![0u8; PTY_READ_BUF];
    loop {
        let n = match reader_file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let bytes = &buf[..n];

        // Feed the headless emulator so an Attach can serialize the
        // current screen state.
        {
            let mut term = handle.term.lock();
            processor.advance(&mut *term, bytes);
        }

        let seq = handle.seq.fetch_add(1, Ordering::AcqRel);
        let data_b64 = base64_encode(bytes);

        // Durability tape — best-effort; logging IO failures shouldn't
        // tear down the live byte stream.
        if let Ok(mut log) = handle.log.lock() {
            let row = TerminalReplayEvent::Output {
                seq,
                data: data_b64.clone(),
            };
            if let Ok(line) = serde_json::to_vec(&row) {
                let _ = log.write_all(&line);
                let _ = log.write_all(b"\n");
                let _ = log.flush();
            }
        }

        broadcast(
            &handle,
            HostEvent::Output {
                session_id: handle.session_id.clone(),
                seq,
                data: data_b64,
            },
        );
    }

    handle.alive.store(false, Ordering::Release);
    // Best effort: collect child exit code.
    let exit_code = {
        let mut guard = handle.pty.lock().expect("pty mutex poisoned");
        match guard.child_mut_try_wait() {
            Some(code) => code,
            None => None,
        }
    };
    *handle.exit_code.lock().expect("exit_code mutex poisoned") = exit_code;
    let seq = handle.seq.fetch_add(1, Ordering::AcqRel);
    broadcast(
        &handle,
        HostEvent::Exit {
            session_id: handle.session_id.clone(),
            seq,
            exit_code,
        },
    );
}

// --- Attach -------------------------------------------------------------

fn attach_session(
    host: &Arc<HostState>,
    session_id: &str,
    out_tx: &mpsc::Sender<HostMessage>,
) -> io::Result<HostSnapshot> {
    let handle = lookup_session(host, session_id)?;
    let snapshot_bytes = {
        let term = handle.term.lock();
        screen_to_ansi::serialize(&*term)
    };
    let last_seq = handle.seq.load(Ordering::Acquire);
    let cols = handle.cols.load(Ordering::Acquire);
    let rows = handle.rows.load(Ordering::Acquire);

    handle
        .subscribers
        .lock()
        .expect("subscribers mutex poisoned")
        .push(out_tx.clone());

    Ok(HostSnapshot {
        events: vec![TerminalReplayEvent::Output {
            seq: last_seq,
            data: base64_encode(&snapshot_bytes),
        }],
        last_seq,
        cols,
        rows,
    })
}

// --- Input / Resize / Stop / Status ------------------------------------

fn write_input(host: &Arc<HostState>, session_id: &str, bytes: &[u8]) -> io::Result<()> {
    let handle = lookup_session(host, session_id)?;
    let mut writer = handle.writer.lock().expect("writer mutex poisoned");
    writer.write_all(bytes)
}

fn resize_session(
    host: &Arc<HostState>,
    session_id: &str,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let handle = lookup_session(host, session_id)?;
    // Resize the headless model first so the next batch of PTY bytes
    // lands in a grid sized to match the agent's post-SIGWINCH redraw.
    {
        let mut term = handle.term.lock();
        term.resize(HostSize {
            cols: cols as usize,
            rows: rows as usize,
        });
    }
    // Then resize the PTY master so the child receives SIGWINCH.
    {
        use alacritty_terminal::event::OnResize;
        let mut pty = handle.pty.lock().expect("pty mutex poisoned");
        pty.on_resize(WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width: 0,
            cell_height: 0,
        });
    }
    handle.cols.store(cols, Ordering::Release);
    handle.rows.store(rows, Ordering::Release);

    let seq = handle.seq.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut log) = handle.log.lock() {
        let row = TerminalReplayEvent::Resize { seq, cols, rows };
        if let Ok(line) = serde_json::to_vec(&row) {
            let _ = log.write_all(&line);
            let _ = log.write_all(b"\n");
            let _ = log.flush();
        }
    }
    broadcast(
        &handle,
        HostEvent::Resize {
            session_id: session_id.to_string(),
            seq,
            cols,
            rows,
        },
    );
    Ok(())
}

fn stop_session(host: &Arc<HostState>, session_id: &str) -> io::Result<()> {
    let handle = lookup_session(host, session_id)?;
    // SIGTERM via the child pid; we hold a u32 pid even after the
    // reader thread takes the master file. Alacritty's `Pty` exposes
    // the `Child` only by shared reference, so we can't borrow it
    // mutably from another thread to call `kill()` — the libc path is
    // simpler and matches what the agent's own signal handlers would
    // see from a terminal.
    let pid = handle.pid;
    if handle.alive.load(Ordering::Acquire) {
        // SAFETY: libc::kill is signal-safe and pid is a u32 we own.
        let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            // ESRCH means the child already exited — not a real error.
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Err(err);
            }
        }
    }
    Ok(())
}

fn session_status(host: &Arc<HostState>, session_id: &str) -> io::Result<HostSessionStatus> {
    let handle = lookup_session(host, session_id)?;
    Ok(session_status_from_handle(&handle))
}

fn session_status_from_handle(handle: &Arc<SessionHandle>) -> HostSessionStatus {
    let exit_code = handle.exit_code.lock().expect("exit_code mutex poisoned").clone();
    HostSessionStatus {
        session_id: handle.session_id.clone(),
        alive: handle.alive.load(Ordering::Acquire),
        exit_code,
        pid: Some(handle.pid),
        command: handle.command.clone(),
        cols: handle.cols.load(Ordering::Acquire),
        rows: handle.rows.load(Ordering::Acquire),
    }
}

fn lookup_session(host: &Arc<HostState>, session_id: &str) -> io::Result<Arc<SessionHandle>> {
    host.sessions
        .lock()
        .expect("sessions mutex poisoned")
        .get(session_id)
        .cloned()
        .ok_or_else(|| io::Error::other(format!("unknown session: {session_id}")))
}

fn broadcast(handle: &Arc<SessionHandle>, event: HostEvent) {
    let msg = HostMessage::Event(event);
    let mut subs = handle.subscribers.lock().expect("subscribers mutex poisoned");
    subs.retain(|tx| tx.send(msg.clone()).is_ok());
}

// --- Pty::child wait helper --------------------------------------------
//
// `tty::Pty::child()` returns `&Child` and there is no public
// `child_mut()` — but we need `try_wait` to grab an exit code on EOF.
// `Child::try_wait` is `&mut self`; rather than fight the borrow checker
// across alacritty's struct, we inline a small libc-based waitpid here.
trait ChildExitProbe {
    fn child_mut_try_wait(&mut self) -> Option<Option<i32>>;
}

impl ChildExitProbe for tty::Pty {
    fn child_mut_try_wait(&mut self) -> Option<Option<i32>> {
        let pid = self.child().id();
        let mut status: libc::c_int = 0;
        // SAFETY: WNOHANG never blocks; pid is a u32 we own.
        let rc = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        if rc <= 0 {
            return Some(None);
        }
        if libc::WIFEXITED(status) {
            Some(Some(libc::WEXITSTATUS(status)))
        } else if libc::WIFSIGNALED(status) {
            Some(Some(128 + libc::WTERMSIG(status)))
        } else {
            Some(None)
        }
    }
}

// --- Frame I/O ----------------------------------------------------------
//
// 4-byte big-endian payload length, then payload. JSON inside. Length
// capped at FRAME_MAX_BYTES so a malformed peer can't drive the host
// into unbounded allocation.

fn read_frame<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    if let Err(err) = reader.read_exact(&mut len_buf) {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(err);
    }
    let len = u32::from_be_bytes(len_buf);
    if len > FRAME_MAX_BYTES {
        return Err(io::Error::other(format!(
            "frame too large: {len} > {FRAME_MAX_BYTES}"
        )));
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::other("frame payload exceeds u32::MAX"))?;
    if len > FRAME_MAX_BYTES {
        return Err(io::Error::other(format!(
            "frame too large to send: {len} > {FRAME_MAX_BYTES}"
        )));
    }
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    Ok(())
}

// --- Base64 -------------------------------------------------------------
//
// Standard alphabet, no padding tolerance on input strictness. The
// `base64` crate is already a transitive dep of `runner_lib`, but the
// sidecar shouldn't pull `runner_lib` so we do it ourselves.

const B64_CHARS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(B64_CHARS[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_CHARS[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_CHARS[(n & 0x3f) as usize] as char);
        i += 3;
    }
    match bytes.len() - i {
        0 => {}
        1 => {
            let n = u32::from(bytes[i]) << 16;
            out.push(B64_CHARS[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_CHARS[((n >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
            out.push(B64_CHARS[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_CHARS[((n >> 12) & 0x3f) as usize] as char);
            out.push(B64_CHARS[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => unreachable!(),
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in s {
        let v: u32 = match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            other => return Err(format!("invalid base64 byte: {other:#x}")),
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// --- ULID -------------------------------------------------------------

fn new_ulid() -> String {
    // Hand-rolled to avoid a `ulid` dep in the sidecar's compile graph
    // (the workspace already has one for runner-core, but pulling it
    // into this bin adds chrono transitives we don't need). Format
    // approximates Crockford ULID: 48-bit ms-since-epoch + 80-bit
    // random. Not strict ULID; just monotonic-enough + unique-enough
    // for filesystem and registry keys.
    use std::time::{SystemTime, UNIX_EPOCH};
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut id = String::with_capacity(26);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    for i in (0..10).rev() {
        id.push(CROCKFORD[((ms >> (i * 5)) & 0x1f) as usize] as char);
    }
    let mut r = [0u8; 10];
    fill_random(&mut r);
    let mut bits: u128 = 0;
    for &b in &r {
        bits = (bits << 8) | u128::from(b);
    }
    for i in (0..16).rev() {
        id.push(CROCKFORD[((bits >> (i * 5)) & 0x1f) as usize as usize] as char);
    }
    id
}

fn fill_random(buf: &mut [u8]) {
    // Best-effort: read from /dev/urandom; fall back to a seq-and-time
    // mix on error so the sidecar still works on stripped systems.
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() {
            return;
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    for byte in buf.iter_mut() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *byte = (seed >> 32) as u8;
    }
}

fn session_id_to_window_id(session_id: &str) -> u64 {
    // alacritty uses this for window-association in events. We don't
    // run a window; any stable hash is fine.
    let mut h: u64 = 0;
    for byte in session_id.bytes() {
        h = h.wrapping_mul(131).wrapping_add(u64::from(byte));
    }
    h
}

// --- Screen-to-ANSI serializer ------------------------------------------
//
// Walks the Term grid row by row, emitting cursor-positioning escapes,
// SGR runs, and cell glyphs that — when written into a fresh xterm or a
// fresh alacritty `Term` — reproduce the current visible state. This
// is the load-bearing piece for plan §"Resize-stack fix mechanism": the
// `Attach` snapshot is this serializer's output, not the raw event
// tape.

mod screen_to_ansi {
    use super::*;
    use alacritty_terminal::term::TermMode;

    pub fn serialize<L: EventListener>(term: &Term<L>) -> Vec<u8> {
        let mut out = Vec::with_capacity(term.columns() * term.screen_lines() * 2);
        let mode = term.mode();
        // Reset hard, hide cursor, then optionally enter alt-screen.
        out.extend_from_slice(b"\x1b[?25l"); // hide cursor while writing
        out.extend_from_slice(b"\x1b[0m"); // reset SGR
        if mode.contains(TermMode::ALT_SCREEN) {
            // Use ?1049h (the xterm variant) — same code claude-code
            // and codex emit, and what xterm.js handles on the client.
            out.extend_from_slice(b"\x1b[?1049h");
        } else {
            out.extend_from_slice(b"\x1b[?1049l");
        }
        // Clear the (now possibly alternate) screen.
        out.extend_from_slice(b"\x1b[2J\x1b[H");

        let grid = term.grid();
        let cols = term.columns();
        let rows = term.screen_lines();

        for row in 0..rows {
            // Position cursor at column 1 of this row (1-indexed).
            let row_pos = format!("\x1b[{};1H", row + 1);
            out.extend_from_slice(row_pos.as_bytes());

            // Reset SGR at row start to keep the wire format
            // self-correcting under partial reads.
            out.extend_from_slice(b"\x1b[0m");
            let mut current_sgr = SgrState::default();

            // Read cells left-to-right. `grid[Line(row as i32)][Column(col)]`
            // gives us the active screen for both main and alt grids.
            use alacritty_terminal::index::{Column, Line};
            let line = Line(row as i32);
            for col in 0..cols {
                let cell = &grid[line][Column(col)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    // The preceding wide cell already covers this
                    // column; emitting our own char would corrupt
                    // alignment.
                    continue;
                }
                let want = SgrState::from(cell);
                if want != current_sgr {
                    emit_sgr_transition(&mut out, &current_sgr, &want);
                    current_sgr = want;
                }
                let mut tmp = [0u8; 4];
                let ch = if cell.c == '\0' { ' ' } else { cell.c };
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
            }
        }

        // Restore cursor position and visibility.
        let cursor = grid.cursor.point;
        let cursor_line = cursor.line.0.max(0) as usize + 1;
        let cursor_col = cursor.column.0 + 1;
        out.extend_from_slice(format!("\x1b[{};{}H", cursor_line, cursor_col).as_bytes());
        out.extend_from_slice(b"\x1b[0m");
        if mode.contains(TermMode::SHOW_CURSOR) {
            out.extend_from_slice(b"\x1b[?25h");
        }
        out
    }

    #[derive(Default, Clone, PartialEq, Eq)]
    struct SgrState {
        bold: bool,
        italic: bool,
        underline: bool,
        inverse: bool,
        strikeout: bool,
        dim: bool,
        hidden: bool,
        fg: Option<Color>,
        bg: Option<Color>,
    }

    impl From<&Cell> for SgrState {
        fn from(cell: &Cell) -> Self {
            SgrState {
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.contains(Flags::UNDERLINE),
                inverse: cell.flags.contains(Flags::INVERSE),
                strikeout: cell.flags.contains(Flags::STRIKEOUT),
                dim: cell.flags.contains(Flags::DIM),
                hidden: cell.flags.contains(Flags::HIDDEN),
                fg: Some(cell.fg),
                bg: Some(cell.bg),
            }
        }
    }

    fn emit_sgr_transition(out: &mut Vec<u8>, _from: &SgrState, to: &SgrState) {
        // Simplest correct strategy: reset, then re-apply. SGR runs in
        // modern TUIs are short so the "smart minimal transitions"
        // optimization isn't worth the complexity.
        let mut parts: Vec<String> = Vec::new();
        parts.push("0".into());
        if to.bold {
            parts.push("1".into());
        }
        if to.dim {
            parts.push("2".into());
        }
        if to.italic {
            parts.push("3".into());
        }
        if to.underline {
            parts.push("4".into());
        }
        if to.inverse {
            parts.push("7".into());
        }
        if to.hidden {
            parts.push("8".into());
        }
        if to.strikeout {
            parts.push("9".into());
        }
        if let Some(fg) = to.fg {
            push_color(&mut parts, fg, /*background=*/ false);
        }
        if let Some(bg) = to.bg {
            push_color(&mut parts, bg, /*background=*/ true);
        }
        let escape = format!("\x1b[{}m", parts.join(";"));
        out.extend_from_slice(escape.as_bytes());
    }

    fn push_color(parts: &mut Vec<String>, color: Color, background: bool) {
        match color {
            Color::Named(named) => {
                if let Some(code) = named_color_code(named, background) {
                    parts.push(code.to_string());
                }
            }
            Color::Spec(rgb) => {
                let prefix = if background { 48 } else { 38 };
                parts.push(format!("{};2;{};{};{}", prefix, rgb.r, rgb.g, rgb.b));
            }
            Color::Indexed(n) => {
                let prefix = if background { 48 } else { 38 };
                parts.push(format!("{};5;{}", prefix, n));
            }
        }
    }

    fn named_color_code(named: NamedColor, background: bool) -> Option<u16> {
        // Standard SGR codes. We only emit foreground/background
        // mappings the receiving xterm will interpret; Cursor/dim-pair
        // variants are filtered through their canonical counterparts.
        let base_fg = if background { 40 } else { 30 };
        let base_bright = if background { 100 } else { 90 };
        let default = if background { 49 } else { 39 };
        Some(match named {
            NamedColor::Black => base_fg,
            NamedColor::Red => base_fg + 1,
            NamedColor::Green => base_fg + 2,
            NamedColor::Yellow => base_fg + 3,
            NamedColor::Blue => base_fg + 4,
            NamedColor::Magenta => base_fg + 5,
            NamedColor::Cyan => base_fg + 6,
            NamedColor::White => base_fg + 7,
            NamedColor::BrightBlack => base_bright,
            NamedColor::BrightRed => base_bright + 1,
            NamedColor::BrightGreen => base_bright + 2,
            NamedColor::BrightYellow => base_bright + 3,
            NamedColor::BrightBlue => base_bright + 4,
            NamedColor::BrightMagenta => base_bright + 5,
            NamedColor::BrightCyan => base_bright + 6,
            NamedColor::BrightWhite => base_bright + 7,
            NamedColor::Foreground | NamedColor::BrightForeground => default,
            NamedColor::Background => default,
            NamedColor::Cursor => return None,
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite
            | NamedColor::DimForeground => default,
        })
    }
}

// --- Daemonization ------------------------------------------------------

unsafe fn daemonize() -> io::Result<()> {
    match libc::fork() {
        -1 => return Err(io::Error::last_os_error()),
        0 => {}
        _pid => libc::_exit(0),
    }
    if libc::setsid() == -1 {
        return Err(io::Error::last_os_error());
    }
    match libc::fork() {
        -1 => return Err(io::Error::last_os_error()),
        0 => {}
        _pid => libc::_exit(0),
    }
    libc::umask(0o077);
    let root = c"/";
    if libc::chdir(root.as_ptr()) == -1 {
        return Err(io::Error::last_os_error());
    }
    redirect_to_dev_null(libc::STDIN_FILENO)?;
    redirect_to_dev_null(libc::STDOUT_FILENO)?;
    Ok(())
}

unsafe fn redirect_to_dev_null(target_fd: libc::c_int) -> io::Result<()> {
    let path = c"/dev/null";
    let fd = libc::open(path.as_ptr(), libc::O_RDWR);
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    let ok = libc::dup2(fd, target_fd);
    libc::close(fd);
    if ok == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

// --- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_term(cols: usize, rows: usize) -> Term<HostListener> {
        let writer = Arc::new(Mutex::new(
            // Throwaway file — listener never writes during tests
            // because we don't feed it bytes that trigger PtyWrite.
            tempfile::tempfile().expect("tempfile"),
        ));
        let listener = HostListener {
            writer: writer.clone(),
        };
        Term::new(
            Config::default(),
            &HostSize { cols, rows },
            listener,
        )
    }

    #[test]
    fn frame_roundtrip_short() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").unwrap();
        let mut cursor = Cursor::new(buf);
        let out = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn frame_eof_at_start_returns_none() {
        let mut empty = Cursor::new(Vec::new());
        let out = read_frame(&mut empty).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn frame_rejects_oversized_length() {
        let mut bad = Vec::new();
        bad.extend_from_slice(&(FRAME_MAX_BYTES + 1).to_be_bytes());
        let mut cursor = Cursor::new(bad);
        let err = read_frame(&mut cursor).unwrap_err();
        assert!(err.to_string().contains("frame too large"));
    }

    #[test]
    fn base64_round_trip_known_strings() {
        for (raw, encoded) in [
            (b"".as_slice(), ""),
            (b"f".as_slice(), "Zg=="),
            (b"fo".as_slice(), "Zm8="),
            (b"foo".as_slice(), "Zm9v"),
            (b"foob".as_slice(), "Zm9vYg=="),
            (b"fooba".as_slice(), "Zm9vYmE="),
            (b"foobar".as_slice(), "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(raw), encoded);
            assert_eq!(base64_decode(encoded).unwrap(), raw);
        }
    }

    #[test]
    fn ulid_is_26_chars() {
        let id = new_ulid();
        assert_eq!(id.len(), 26, "got {id}");
    }

    #[test]
    fn screen_to_ansi_round_trip_preserves_grid() {
        let cols = 20usize;
        let rows = 5usize;
        let mut a = make_term(cols, rows);
        let mut processor: Processor = Processor::new();
        let input = b"Hello, world!\r\nLine two\r\n\x1b[1mBold\x1b[0m text";
        processor.advance(&mut a, input);

        let bytes = screen_to_ansi::serialize(&a);

        let mut b = make_term(cols, rows);
        let mut processor2: Processor = Processor::new();
        processor2.advance(&mut b, &bytes);

        assert_eq!(
            a.columns(),
            b.columns(),
            "column count diverged after round-trip"
        );
        assert_eq!(
            a.screen_lines(),
            b.screen_lines(),
            "row count diverged after round-trip"
        );
        for row in 0..rows {
            use alacritty_terminal::index::{Column, Line};
            let line = Line(row as i32);
            for col in 0..cols {
                let cell_a = &a.grid()[line][Column(col)];
                let cell_b = &b.grid()[line][Column(col)];
                assert_eq!(
                    cell_a.c, cell_b.c,
                    "char divergence at ({row},{col}): a={:?} b={:?}",
                    cell_a.c, cell_b.c
                );
            }
        }
    }

    #[test]
    fn dispatch_list_returns_empty_sessions() {
        let host = Arc::new(HostState {
            sessions: Mutex::new(HashMap::new()),
            sessions_dir: PathBuf::from("/tmp/nonexistent-pty-host"),
        });
        let (tx, _rx) = mpsc::channel::<HostMessage>();
        match dispatch(&host, &tx, HostRequest::List) {
            HostResponse::Sessions { sessions } => assert!(sessions.is_empty()),
            other => panic!("expected Sessions, got {other:?}"),
        }
    }
}
