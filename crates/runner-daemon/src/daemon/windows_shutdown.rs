use std::io;
use std::ptr::{null, null_mut};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::oneshot;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::SetProcessShutdownParameters;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::session::manager::windows_shutdown::SessionEnd;

const STAMP_TIMEOUT: Duration = Duration::from_secs(4);

pub(super) fn set_shutdown_priority() -> io::Result<()> {
    // Application-first range; ordinary children/ConPTY hosts start at 0x280.
    if unsafe { SetProcessShutdownParameters(0x3ff, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) struct ShutdownWindow {
    hwnd: usize,
    thread: Option<JoinHandle<()>>,
    session_end: Arc<SessionEnd>,
    pub(super) signal: oneshot::Receiver<()>,
}

impl ShutdownWindow {
    pub(super) fn new(session_end: Arc<SessionEnd>) -> io::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (signal_tx, signal) = oneshot::channel();
        let state = WindowState {
            session_end: session_end.clone(),
            signal: Some(signal_tx),
        };
        let thread = std::thread::Builder::new()
            .name("runner-session-end".into())
            .spawn(move || unsafe { message_loop(state, ready_tx) })?;
        let hwnd = ready_rx
            .recv()
            .map_err(|_| io::Error::other("shutdown window thread exited during startup"))??;
        Ok(Self {
            hwnd,
            thread: Some(thread),
            session_end,
            signal,
        })
    }
}

impl Drop for ShutdownWindow {
    fn drop(&mut self) {
        self.session_end.cancel();
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct WindowState {
    session_end: Arc<SessionEnd>,
    signal: Option<oneshot::Sender<()>>,
}

unsafe fn message_loop(mut state: WindowState, ready: mpsc::SyncSender<io::Result<usize>>) {
    let instance = unsafe { GetModuleHandleW(null()) };
    let class: Vec<u16> = format!("RunnerSessionEnd-{}\0", ulid::Ulid::new())
        .encode_utf16()
        .collect();
    let descriptor = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { RegisterClassW(&descriptor) } == 0 {
        let _ = ready.send(Err(io::Error::last_os_error()));
        return;
    }
    // No WS_VISIBLE and no parent: broadcasts do not reach HWND_MESSAGE windows.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            (&mut state as *mut WindowState).cast(),
        )
    };
    if hwnd.is_null() {
        let _ = ready.send(Err(io::Error::last_os_error()));
    } else {
        let _ = ready.send(Ok(hwnd as usize));
        let mut message = unsafe { std::mem::zeroed() };
        loop {
            let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
            if result <= 0 {
                if result < 0 {
                    log::error!(
                        "runnerd shutdown message loop: {}",
                        io::Error::last_os_error()
                    );
                }
                break;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        unsafe { DestroyWindow(hwnd) };
    }
    unsafe { UnregisterClassW(class.as_ptr(), instance) };
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if !state.is_null() {
        match message {
            WM_QUERYENDSESSION => {
                unsafe { (*state).session_end.query() };
                log::info!("runnerd WM_QUERYENDSESSION flags={:#x}", lparam as u32);
                return 1;
            }
            WM_ENDSESSION => {
                if wparam == 0 {
                    unsafe { (*state).session_end.cancel() };
                    log::info!("runnerd WM_ENDSESSION cancelled");
                } else {
                    unsafe { (*state).session_end.confirm() };
                    log::info!(
                        "runnerd WM_ENDSESSION flags={:#x}; awaiting resume stamp",
                        lparam as u32
                    );
                    if let Some(signal) = unsafe { (*state).signal.take() } {
                        let _ = signal.send(());
                    }
                    if !unsafe { (*state).session_end.wait_for_stamp(STAMP_TIMEOUT) } {
                        log::error!("runnerd Windows shutdown resume stamp failed or exceeded 4s");
                    }
                }
                return 0;
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                return 0;
            }
            WM_NCDESTROY => {
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::SessionStatus, repo::session, test_support};

    #[tokio::test]
    async fn unavailable_window_does_not_trigger_shutdown() {
        assert!(tokio::time::timeout(
            Duration::from_millis(50),
            super::super::server::os_shutdown(None),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn closing_window_resolves_shutdown() {
        let core = test_support::test_core();
        let mut window = ShutdownWindow::new(core.sessions.session_end.clone()).unwrap();
        assert_ne!(
            unsafe { PostMessageW(window.hwnd as HWND, WM_CLOSE, 0, 0) },
            0
        );
        tokio::time::timeout(
            Duration::from_secs(2),
            super::super::server::os_shutdown(Some(&mut window.signal)),
        )
        .await
        .expect("closing the window must resolve shutdown");
    }

    #[tokio::test]
    async fn end_session_waits_for_committed_stamp_before_returning() {
        for flags in [0, ENDSESSION_LOGOFF as LPARAM] {
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("runner.db");
            let mut core = test_support::test_core();
            core.db = Arc::new(crate::db::open_pool(&database).unwrap());
            session::insert(
                &core.db.get().unwrap(),
                &test_support::test_session_row("live", SessionStatus::Running),
            )
            .unwrap();
            let mut window = ShutdownWindow::new(core.sessions.session_end.clone()).unwrap();
            let hwnd = window.hwnd;
            unsafe {
                assert!(GetParent(hwnd as HWND).is_null());
                assert_eq!(GetAncestor(hwnd as HWND, GA_ROOT) as usize, hwnd);
                assert_eq!(IsWindowVisible(hwnd as HWND), 0);
                assert_eq!(SendMessageW(hwnd as HWND, WM_QUERYENDSESSION, 0, flags), 1);
            }
            let (returned_tx, returned_rx) = mpsc::channel();
            let sender = std::thread::spawn(move || {
                unsafe { SendMessageW(hwnd as HWND, WM_ENDSESSION, 1, flags) };
                returned_tx.send(()).unwrap();
            });
            tokio::time::timeout(
                Duration::from_secs(2),
                super::super::server::os_shutdown(Some(&mut window.signal)),
            )
            .await
            .expect("window must resolve the OS shutdown future");
            assert!(matches!(
                returned_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            assert!(
                !session::get_row(&core.db.get().unwrap(), "live")
                    .unwrap()
                    .unwrap()
                    .resume_on_launch
            );
            assert_eq!(
                super::super::server::stamp_shutdown_sessions(&core).unwrap(),
                ["live"]
            );
            returned_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            sender.join().unwrap();
            let observer = rusqlite::Connection::open(&database).unwrap();
            assert!(
                session::get_row(&observer, "live")
                    .unwrap()
                    .unwrap()
                    .resume_on_launch
            );
        }
    }

    #[tokio::test]
    async fn query_does_not_wait_for_exit_already_reconciling() {
        let core = test_support::test_core();
        let mut row = test_support::test_session_row("exiting", SessionStatus::Running);
        row.agent_session_key = Some("conversation".into());
        session::insert(&core.db.get().unwrap(), &row).unwrap();
        let mut window = ShutdownWindow::new(core.sessions.session_end.clone()).unwrap();
        let hwnd = window.hwnd;
        let mut exit = core.sessions.session_end.exit();
        exit.snapshot = Some(row);
        let (queried_tx, queried_rx) = mpsc::channel();
        let query = std::thread::spawn(move || {
            let result = unsafe { SendMessageW(hwnd as HWND, WM_QUERYENDSESSION, 0, 0) };
            queried_tx.send(result).unwrap();
        });
        // The exit token stays live, as it would during a blocked DB checkout/write.
        assert_eq!(queried_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        query.join().unwrap();
        let (returned_tx, returned_rx) = mpsc::channel();
        let end = std::thread::spawn(move || {
            unsafe { SendMessageW(hwnd as HWND, WM_ENDSESSION, 1, 0) };
            returned_tx.send(()).unwrap();
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            super::super::server::os_shutdown(Some(&mut window.signal)),
        )
        .await
        .unwrap();
        let stamping_core = core.clone();
        let stamp = tokio::task::spawn_blocking(move || {
            super::super::server::stamp_shutdown_sessions(&stamping_core).unwrap()
        });
        assert!(matches!(
            returned_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        session::set_crashed_clearing_key(&core.db.get().unwrap(), "exiting", chrono::Utc::now())
            .unwrap();
        drop(exit);
        assert_eq!(stamp.await.unwrap(), ["exiting"]);
        returned_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        end.join().unwrap();
        let row = session::get_row(&core.db.get().unwrap(), "exiting")
            .unwrap()
            .unwrap();
        assert!(row.resume_on_launch);
        assert_eq!(row.status, SessionStatus::Crashed);
        assert_eq!(row.agent_session_key.as_deref(), Some("conversation"));
    }

    #[tokio::test]
    async fn cancelled_query_does_not_requeue_exits_on_a_later_shutdown() {
        let core = test_support::test_core();
        let row = test_support::test_session_row("exiting", SessionStatus::Running);
        session::insert(&core.db.get().unwrap(), &row).unwrap();
        let mut window = ShutdownWindow::new(core.sessions.session_end.clone()).unwrap();
        let hwnd = window.hwnd;
        let mut exit = core.sessions.session_end.exit();
        exit.snapshot = Some(row);
        unsafe { SendMessageW(hwnd as HWND, WM_QUERYENDSESSION, 0, 0) };
        session::set_exit_status(
            &core.db.get().unwrap(),
            "exiting",
            SessionStatus::Stopped,
            chrono::Utc::now(),
        )
        .unwrap();
        drop(exit);
        unsafe {
            SendMessageW(hwnd as HWND, WM_ENDSESSION, 0, 0);
            SendMessageW(hwnd as HWND, WM_QUERYENDSESSION, 0, 0);
        }
        let sender =
            std::thread::spawn(move || unsafe { SendMessageW(hwnd as HWND, WM_ENDSESSION, 1, 0) });
        tokio::time::timeout(
            Duration::from_secs(2),
            super::super::server::os_shutdown(Some(&mut window.signal)),
        )
        .await
        .unwrap();
        assert!(super::super::server::stamp_shutdown_sessions(&core)
            .unwrap()
            .is_empty());
        sender.join().unwrap();
        assert!(
            !session::get_row(&core.db.get().unwrap(), "exiting")
                .unwrap()
                .unwrap()
                .resume_on_launch
        );
    }

    #[test]
    fn exited_snapshots_do_not_restore_replaced_or_archived_rows() {
        let core = test_support::test_core();
        let mut conn = core.db.get().unwrap();
        let mut snapshots = Vec::new();
        for id in ["eligible", "replaced", "archived"] {
            let mut row = test_support::test_session_row(id, SessionStatus::Running);
            row.agent_session_key = Some(format!("key-{id}"));
            session::insert(&conn, &row).unwrap();
            session::set_crashed_clearing_key(&conn, id, chrono::Utc::now()).unwrap();
            snapshots.push(row);
        }
        conn.execute(
            "UPDATE sessions SET started_at = '2026-01-01T00:00:00Z' WHERE id = 'replaced'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE sessions SET archived_at = '2026-01-01T00:00:00Z' WHERE id = 'archived'",
            [],
        )
        .unwrap();
        assert_eq!(
            session::mark_running_and_exited_for_resume_on_launch(&mut conn, &snapshots).unwrap(),
            ["eligible"]
        );
        for id in ["eligible", "replaced", "archived"] {
            let row = session::get_row(&conn, id).unwrap().unwrap();
            assert_eq!(row.resume_on_launch, id == "eligible");
            assert_eq!(
                row.agent_session_key.as_deref(),
                (id == "eligible").then_some("key-eligible")
            );
        }
    }

    #[tokio::test]
    async fn query_holds_child_exit_until_stamp_and_cancel_releases_it() {
        for ending in [false, true] {
            let core = test_support::test_core();
            let mut row = test_support::test_session_row("live", SessionStatus::Running);
            row.agent_session_key = Some("conversation".into());
            session::insert(&core.db.get().unwrap(), &row).unwrap();
            let mut window = ShutdownWindow::new(core.sessions.session_end.clone()).unwrap();
            let hwnd = window.hwnd;
            unsafe { SendMessageW(hwnd as HWND, WM_QUERYENDSESSION, 0, 0) };
            let exiting_core = core.clone();
            let (started_tx, started_rx) = mpsc::channel();
            let (exited_tx, exited_rx) = mpsc::channel();
            let child = std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let guard = exiting_core.sessions.session_end.exit();
                let conn = exiting_core.db.get().unwrap();
                if guard.preserve_key() {
                    session::set_exit_status(
                        &conn,
                        "live",
                        SessionStatus::Crashed,
                        chrono::Utc::now(),
                    )
                    .unwrap();
                } else {
                    session::set_crashed_clearing_key(&conn, "live", chrono::Utc::now()).unwrap();
                }
                exited_tx.send(()).unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(matches!(
                exited_rx.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            let sender = std::thread::spawn(move || unsafe {
                SendMessageW(hwnd as HWND, WM_ENDSESSION, usize::from(ending), 0)
            });
            if ending {
                tokio::time::timeout(
                    Duration::from_secs(2),
                    super::super::server::os_shutdown(Some(&mut window.signal)),
                )
                .await
                .unwrap();
                assert_eq!(
                    super::super::server::stamp_shutdown_sessions(&core).unwrap(),
                    ["live"]
                );
            }
            sender.join().unwrap();
            exited_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            child.join().unwrap();
            if !ending {
                assert!(matches!(
                    window.signal.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ));
            }
            let conn = core.db.get().unwrap();
            let row = session::get_row(&conn, "live").unwrap().unwrap();
            assert_eq!(row.status, SessionStatus::Crashed);
            assert_eq!(row.resume_on_launch, ending);
            assert_eq!(
                row.agent_session_key.as_deref(),
                ending.then_some("conversation")
            );
        }
    }
}
