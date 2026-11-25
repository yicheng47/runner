//! Windows has no LaunchServices to route a second launch to the running app, so the app does it:
//! a session-scoped mutex marks the running instance, and a named pipe asks it to come forward.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::StreamExt as _;
use windows::core::{HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, WaitForSingleObject};
use windows::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

const HANDOFF_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_INTERVAL_MS: u32 = 250;
const PIPE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) enum Instance {
    /// This process runs the app.
    Primary(Relaunches),
    /// The running instance accepted this launch; this process should exit.
    Secondary,
}

/// Later launches asking this process to come forward.
#[derive(Clone)]
pub(crate) struct Relaunches(Rc<RefCell<UnboundedReceiver<Handoff>>>);

impl Relaunches {
    pub(crate) async fn next(&self) -> Option<Handoff> {
        std::future::poll_fn(|cx| self.0.borrow_mut().poll_next_unpin(cx)).await
    }

    /// Refuses pending and later launches, which then wait for this process to exit.
    pub(crate) fn close(&self) {
        let mut requests = self.0.borrow_mut();
        requests.close();
        while requests.try_recv().is_ok() {}
    }
}

/// A launch waiting for an answer: it exits once accepted and keeps trying to start if dropped.
pub(crate) struct Handoff(oneshot::Sender<()>);

impl Handoff {
    pub(crate) fn accept(self) {
        let _ = self.0.send(());
    }
}

/// One instance of the relaunch pipe's server end, closed when dropped.
struct Pipe(HANDLE);

// Only the listener thread uses the handle once it owns the pipe.
unsafe impl Send for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub(crate) fn claim(identifier: &str) -> Instance {
    let deadline = Instant::now() + HANDOFF_TIMEOUT;
    let (mutex_name, pipe_name) = names(identifier);
    // Held until the process exits, which releases it for the next launch.
    let mutex = match unsafe { CreateMutexW(None, true, &mutex_name) } {
        Ok(mutex) => mutex,
        Err(error) => {
            eprintln!("Runner single-instance mutex unavailable: {error}");
            return Instance::Primary(listen(&pipe_name, deadline));
        }
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        loop {
            if signal(&pipe_name, deadline) {
                return Instance::Secondary;
            }
            // No answer while the running instance is still starting or already quitting, as
            // when the updater relaunches Runner: take over once it exits.
            let wait = unsafe { WaitForSingleObject(mutex, RETRY_INTERVAL_MS) };
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!("Runner: the running instance did not answer; starting another");
                break;
            }
        }
    }
    Instance::Primary(listen(&pipe_name, deadline))
}

fn names(identifier: &str) -> (HSTRING, HSTRING) {
    let mut session = 0;
    let _ = unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
    (
        HSTRING::from(format!("Local\\{identifier}-instance")),
        HSTRING::from(format!(r"\\.\pipe\{identifier}-{session}-activate")),
    )
}

fn listen(pipe_name: &HSTRING, deadline: Instant) -> Relaunches {
    let (tx, rx) = unbounded();
    // Created before startup continues, so a launch racing this one finds the pipe.
    let first = create_pipe(pipe_name, true).ok();
    let pipe_name = pipe_name.clone();
    let spawned = std::thread::Builder::new()
        .name("runner-relaunch".into())
        .spawn(move || {
            // A previous instance's pipe can outlive its mutex while that process exits.
            if let Some(pipe) = first.or_else(|| acquire(&pipe_name, deadline)) {
                serve(pipe, &pipe_name, tx);
            }
        });
    if let Err(error) = spawned {
        eprintln!("Runner relaunch listener failed to start: {error}");
    }
    Relaunches(Rc::new(RefCell::new(rx)))
}

fn create_pipe(pipe_name: &HSTRING, first: bool) -> std::io::Result<Pipe> {
    let first = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        FILE_FLAGS_AND_ATTRIBUTES(0)
    };
    let pipe = unsafe {
        CreateNamedPipeW(
            pipe_name,
            PIPE_ACCESS_DUPLEX | first,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            1,
            1,
            0,
            None,
        )
    };
    if pipe.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Pipe(pipe))
}

fn acquire(pipe_name: &HSTRING, deadline: Instant) -> Option<Pipe> {
    loop {
        match create_pipe(pipe_name, true) {
            Ok(pipe) => return Some(pipe),
            Err(error) if Instant::now() >= deadline => {
                eprintln!("Runner relaunch pipe unavailable: {error}");
                return None;
            }
            Err(_) => std::thread::sleep(PIPE_RETRY_INTERVAL),
        }
    }
}

fn serve(mut pipe: Pipe, pipe_name: &HSTRING, tx: UnboundedSender<Handoff>) {
    loop {
        if let Err(error) = unsafe { ConnectNamedPipe(pipe.0, None) } {
            // A launch can connect, write and close before this call (ERROR_NO_DATA); its
            // byte is still buffered, so read it like any other connection.
            let code = error.code();
            if code != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0)
                && code != HRESULT::from_win32(ERROR_NO_DATA.0)
            {
                eprintln!("Runner relaunch pipe failed: {error}");
                return;
            }
        }
        // The launching process grants foreground rights before it writes, so wait for the
        // byte before asking the window to come forward.
        let mut byte = [0u8; 1];
        let mut read = 0;
        if unsafe { ReadFile(pipe.0, Some(&mut byte), Some(&mut read), None) }.is_ok() && read == 1
        {
            let (answer, accepted) = oneshot::channel();
            if tx.unbounded_send(Handoff(answer)).is_err() {
                return;
            }
            if futures::executor::block_on(accepted).is_ok() {
                let mut written = 0;
                let _ = unsafe { WriteFile(pipe.0, Some(&[1]), Some(&mut written), None) };
            }
        }
        // Disconnecting would discard an unread answer, so the launch keeps this instance until
        // it hangs up, and a new instance takes the next launch.
        match create_pipe(pipe_name, false) {
            Ok(next) => pipe = next,
            Err(error) => {
                eprintln!("Runner relaunch pipe unavailable: {error}");
                return;
            }
        }
    }
}

fn open(pipe_name: &HSTRING) -> Option<HANDLE> {
    unsafe {
        CreateFileW(
            pipe_name,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .ok()
}

/// Asks the running instance to come forward; true once it has accepted, before `deadline`.
fn signal(pipe_name: &HSTRING, deadline: Instant) -> bool {
    let Some(pipe) = open(pipe_name) else {
        return false;
    };
    let mut server = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe, &mut server) }.is_ok() {
        let _ = unsafe { AllowSetForegroundWindow(server) };
    }
    let mut answer = [0u8; 1];
    let accepted = transfer(pipe, deadline, |overlapped| unsafe {
        WriteFile(pipe, Some(&[1]), None, Some(overlapped))
    }) == Some(1)
        && transfer(pipe, deadline, |overlapped| unsafe {
            ReadFile(pipe, Some(&mut answer), None, Some(overlapped))
        }) == Some(1);
    let _ = unsafe { CloseHandle(pipe) };
    accepted
}

/// Runs one overlapped read or write on `pipe`, giving up at `deadline`.
fn transfer(
    pipe: HANDLE,
    deadline: Instant,
    start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> Option<u32> {
    let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?;
    let mut overlapped = OVERLAPPED {
        hEvent: event,
        ..Default::default()
    };
    let mut transferred = 0;
    let started = match start(&raw mut overlapped) {
        Ok(()) => true,
        Err(error) => error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0),
    };
    let done = started && {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        let wait = u32::try_from(wait).unwrap_or(u32::MAX);
        let done = unsafe {
            GetOverlappedResultEx(pipe, &raw const overlapped, &mut transferred, wait, false)
        }
        .is_ok();
        if !done {
            // The operation must finish before its buffer and OVERLAPPED go away.
            let _ = unsafe { CancelIoEx(pipe, Some(&raw const overlapped)) };
            let _ =
                unsafe { GetOverlappedResult(pipe, &raw const overlapped, &mut transferred, true) };
        }
        done
    };
    let _ = unsafe { CloseHandle(event) };
    done.then_some(transferred)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use futures::future::{select, Either};
    use windows::Win32::System::Pipes::WaitNamedPipeW;

    use super::*;

    fn pipe_name(case: &str) -> HSTRING {
        HSTRING::from(format!(
            r"\\.\pipe\runner-test-{}-{case}-activate",
            std::process::id()
        ))
    }

    fn deadline() -> Instant {
        Instant::now() + HANDOFF_TIMEOUT
    }

    fn launch(pipe_name: &HSTRING) -> bool {
        signal(pipe_name, deadline())
    }

    fn hang_up(pipe_name: &HSTRING) -> bool {
        open(pipe_name).is_some_and(|pipe| unsafe { CloseHandle(pipe) }.is_ok())
    }

    /// The first half of a launch: connected, with its request sent.
    fn request(pipe_name: &HSTRING) -> HANDLE {
        let pipe = open(pipe_name).unwrap();
        let sent = transfer(pipe, deadline(), |overlapped| unsafe {
            WriteFile(pipe, Some(&[1]), None, Some(overlapped))
        });
        assert_eq!(sent, Some(1));
        pipe
    }

    /// The second half: whether the listener answered.
    fn read_answer(pipe: HANDLE) -> bool {
        let mut answer = [0u8; 1];
        let read = transfer(pipe, deadline(), |overlapped| unsafe {
            ReadFile(pipe, Some(&mut answer), None, Some(overlapped))
        });
        let _ = unsafe { CloseHandle(pipe) };
        read == Some(1)
    }

    /// A launch that gave up waiting for its answer.
    fn request_and_hang_up(pipe_name: &HSTRING) {
        let _ = unsafe { CloseHandle(request(pipe_name)) };
    }

    /// A launch connects only once the listener has an instance waiting for it.
    fn when_accepting(pipe_name: &HSTRING, launch: impl Fn(&HSTRING) -> bool) {
        for _ in 0..10 {
            if launch(pipe_name) {
                return;
            }
            assert!(
                unsafe { WaitNamedPipeW(pipe_name, 5000) }.as_bool(),
                "the relaunch listener stopped accepting: {}",
                std::io::Error::last_os_error()
            );
        }
        panic!("the relaunch listener never took the connection");
    }

    /// Answers each handoff on another thread, counting them as they arrive.
    fn consume(
        rx: UnboundedReceiver<Handoff>,
        accept: impl Fn(usize) -> bool + Send + 'static,
    ) -> Arc<AtomicUsize> {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for handoff in futures::executor::block_on_stream(rx) {
                if accept(seen.fetch_add(1, Ordering::SeqCst)) {
                    handoff.accept();
                }
            }
        });
        count
    }

    fn serve_on_thread(pipe: Pipe, pipe_name: &HSTRING) -> UnboundedReceiver<Handoff> {
        let (tx, rx) = unbounded();
        let pipe_name = pipe_name.clone();
        std::thread::spawn(move || serve(pipe, &pipe_name, tx));
        rx
    }

    #[test]
    fn a_launch_that_closes_before_the_listener_accepts_still_activates() {
        let name = pipe_name("early-request");
        let pipe = create_pipe(&name, true).unwrap();
        request_and_hang_up(&name);
        let count = consume(serve_on_thread(pipe, &name), |_| true);

        when_accepting(&name, launch);

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_launch_that_closes_without_a_request_neither_activates_nor_stops_the_listener() {
        let name = pipe_name("early-hang-up");
        let pipe = create_pipe(&name, true).unwrap();
        assert!(hang_up(&name));
        let count = consume(serve_on_thread(pipe, &name), |_| true);

        when_accepting(&name, launch);

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_refused_launch_is_not_handed_off_and_the_listener_keeps_serving() {
        let name = pipe_name("refused");
        let pipe = create_pipe(&name, true).unwrap();
        let count = consume(serve_on_thread(pipe, &name), |seen| seen > 0);

        assert!(!launch(&name));
        when_accepting(&name, launch);

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_closed_consumer_refuses_launches_and_closes_the_pipe() {
        let name = pipe_name("closed");
        let relaunches = listen(&name, deadline());

        relaunches.close();

        assert!(!launch(&name));
        assert!(open(&name).is_none());
    }

    #[test]
    fn an_answer_stays_readable_after_the_listener_moves_on() {
        let name = pipe_name("unread-answer");
        let pipe = create_pipe(&name, true).unwrap();
        let count = consume(serve_on_thread(pipe, &name), |_| true);

        let first = request(&name);
        when_accepting(&name, launch);

        assert!(read_answer(first));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_launch_gives_up_at_its_deadline_and_the_listener_recovers() {
        let name = pipe_name("unanswered");
        let pipe = create_pipe(&name, true).unwrap();
        let mut rx = serve_on_thread(pipe, &name);
        let bound = Duration::from_secs(5);

        let (sent, signaled) = std::sync::mpsc::channel();
        let client = name.clone();
        std::thread::spawn(move || {
            let _ = sent.send(signal(&client, Instant::now() + Duration::from_millis(100)));
        });
        assert!(!signaled.recv_timeout(bound).unwrap());
        let (sent, queued) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let handoff = futures::executor::block_on(rx.next());
            let _ = sent.send((handoff, rx));
        });
        let (handoff, rx) = queued.recv_timeout(bound).unwrap();
        drop(handoff.unwrap());
        let count = consume(rx, |_| true);
        when_accepting(&name, launch);

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn closing_relaunches_refuses_queued_handoffs() {
        let (tx, rx) = unbounded();
        let relaunches = Relaunches(Rc::new(RefCell::new(rx)));
        let (answer, accepted) = oneshot::channel();
        tx.unbounded_send(Handoff(answer)).unwrap();

        relaunches.close();

        assert!(futures::executor::block_on(accepted).is_err());
        let (answer, _) = oneshot::channel();
        assert!(tx.unbounded_send(Handoff(answer)).is_err());
    }

    #[test]
    fn a_takeover_listens_once_the_previous_pipe_closes() {
        let identifier = format!("runner-test-{}-takeover", std::process::id());
        let (mutex_name, pipe_name) = names(&identifier);
        // The previous instance: the thread that owned the mutex has exited, while its pipe is
        // still open and busy with a connected client.
        let (previous, client) = {
            let pipe_name = pipe_name.clone();
            std::thread::spawn(move || {
                unsafe { CreateMutexW(None, true, &mutex_name) }.unwrap();
                let pipe = create_pipe(&pipe_name, true).unwrap();
                let client = open(&pipe_name).unwrap();
                (pipe, client.0 as isize)
            })
            .join()
            .unwrap()
        };

        let Instance::Primary(relaunches) = claim(&identifier) else {
            panic!("the previous instance cannot accept a launch");
        };
        drop(previous);
        let _ = unsafe { CloseHandle(HANDLE(client as *mut _)) };

        let (done, launched) = oneshot::channel();
        let later = std::thread::spawn(move || {
            let secondary = matches!(claim(&identifier), Instance::Secondary);
            let _ = done.send(());
            secondary
        });
        match futures::executor::block_on(select(Box::pin(relaunches.next()), launched)) {
            Either::Left((Some(handoff), _)) => handoff.accept(),
            _ => panic!("the later launch was not handed to the new instance"),
        }
        assert!(later.join().unwrap());
    }
}
