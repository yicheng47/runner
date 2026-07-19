//! Regression for the review finding: dropping the last external
//! `Arc<TerminalSession>` must actually release the session (the
//! event thread used to hold a session Arc, making Drop unreachable
//! and leaking the PTY child).
//!
//! Hermetic on purpose: no `ps`/OS process inspection (denied in
//! sandboxed runs). `Weak::upgrade() == None` after the drop proves
//! no internal thread retains a strong reference — exactly the
//! original cycle — and since `Drop` synchronously kills AND reaps
//! the child, drop() returning implies the child is gone too.

use std::sync::Arc;

use native_spike::terminal::TerminalSession;

#[test]
fn dropping_last_arc_releases_session_and_reaps_child() {
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let session = TerminalSession::spawn("cat", &[], 80, 24, waker).expect("spawn cat under a PTY");
    assert!(session.pid().is_some(), "child should have spawned");

    let weak = Arc::downgrade(&session);
    assert!(weak.upgrade().is_some(), "session alive while held");

    // Returns only after Drop ran: child killed + reaped (wait()).
    drop(session);

    assert!(
        weak.upgrade().is_none(),
        "an internal thread still holds a strong Arc<TerminalSession> — \
         the event-thread ownership cycle is back"
    );
}
