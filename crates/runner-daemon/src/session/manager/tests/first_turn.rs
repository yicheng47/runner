use super::*;
use crate::runtimes::TerminalEvent;
use crate::session::manager::first_turn::{
    FirstTurnOutcome, FirstTurnSnapshot, FirstTurnStep, FirstTurnSubmit, FirstTurnTimings,
    PendingFirstTurn, ReadinessDecision, RetryReason,
};
use crate::session::manager::startup::StartupDelivery;
use crate::session::state::agent::AgentEvent;
use crate::session::status::{
    Activity, AgentObservation, HumanInteraction, ObservationSource, WaitReason,
};

type WriteCallback =
    Box<dyn FnMut(usize, &[u8], &std::sync::mpsc::Sender<RuntimeOutput>) + Send + 'static>;

/// Scripted `SessionRuntime` that drives deterministic PTY responses inside
/// `send_bytes` so tests verify exact write sequences with zero fixed sleeps.
struct ScriptedRuntime {
    tx: Mutex<Option<std::sync::mpsc::Sender<RuntimeOutput>>>,
    writes: Mutex<Vec<Vec<u8>>>,
    on_write: Mutex<Option<WriteCallback>>,
}

impl ScriptedRuntime {
    fn new(
        on_write: impl FnMut(usize, &[u8], &std::sync::mpsc::Sender<RuntimeOutput>) + Send + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            tx: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
            on_write: Mutex::new(Some(Box::new(on_write))),
        })
    }

    fn push(&self, output: RuntimeOutput) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(output);
        }
    }

    fn push_output(&self, bytes: &[u8]) {
        self.push(RuntimeOutput::Stream(bytes.to_vec()));
    }

    fn push_agent_sync(&self, event: AgentEvent) {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.push(RuntimeOutput::AgentEvent {
            event,
            feedback: Some(reply_tx),
        });
        reply_rx.recv().expect("forwarder applied agent event");
    }

    fn writes(&self) -> Vec<Vec<u8>> {
        self.writes.lock().unwrap().clone()
    }
}

impl SessionRuntime for ScriptedRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.tx.lock().unwrap() = Some(tx);
        let stop = Arc::new(AtomicBool::new(false));
        Ok((
            RuntimeSession {
                runtime: "scripted".into(),
                session_id: spec.session_id,
            },
            OutputStream::new(rx, stop),
        ))
    }

    fn stop(&self, _session: &RuntimeSession) -> RuntimeResult<()> {
        *self.tx.lock().unwrap() = None;
        Ok(())
    }

    fn send_bytes(&self, _session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        let index = {
            let mut writes = self.writes.lock().unwrap();
            let index = writes.len();
            writes.push(bytes.to_vec());
            index
        };
        let tx = self.tx.lock().unwrap().clone();
        if let Some(tx) = tx {
            if let Some(callback) = self.on_write.lock().unwrap().as_mut() {
                callback(index, bytes, &tx);
            }
        }
        Ok(())
    }

    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        if key == "Enter" {
            self.send_bytes(session, b"\r")
        } else {
            Ok(())
        }
    }

    fn resize(&self, _session: &RuntimeSession, _cols: u16, _rows: u16) -> RuntimeResult<()> {
        Ok(())
    }

    fn status(&self, _session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Ok(Some(SessionStatus {
            alive: false,
            exit_code: Some(0),
            pid: Some(4242),
            command: Some("codex.cmd".into()),
        }))
    }
}

fn send_agent_sync(tx: &std::sync::mpsc::Sender<RuntimeOutput>, event: AgentEvent) {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let _ = tx.send(RuntimeOutput::AgentEvent {
        event,
        feedback: Some(reply_tx),
    });
    let _ = reply_rx.recv();
}

fn screen(lines: &[&str]) -> Vec<String> {
    let mut rows = vec![String::new(); 12];
    let start = rows.len().saturating_sub(lines.len());
    for (offset, line) in lines.iter().enumerate() {
        rows[start + offset] = (*line).to_owned();
    }
    rows
}

fn snapshot_with(rows: Vec<String>) -> FirstTurnSnapshot {
    FirstTurnSnapshot {
        delivery: router::DeliverySnapshot {
            generation: 1,
            input_revision: (0, 0),
            turn_started: 0,
            prompted: false,
            rows,
            unsent: false,
            held: false,
            runtime: Runtime::Codex.key().into(),
        },
        title_turn_started: 0,
        authoritative_turn_signals: false,
        needs_you: false,
    }
}

fn test_timings() -> FirstTurnTimings {
    FirstTurnTimings {
        submit_delay: Duration::from_millis(300),
        confirm_window: Duration::from_millis(1500),
        submit_timeout: Duration::from_secs(20),
    }
}

fn interaction() -> HumanInteraction {
    HumanInteraction {
        id: "wait-1".into(),
        reason: WaitReason::Approval,
        owners: Vec::new(),
        since: 0,
    }
}

fn osc_observation(activity: Activity, needs_you: bool) -> AgentObservation {
    AgentObservation {
        source: ObservationSource::Baseline,
        activity,
        interactions: needs_you.then(interaction).into_iter().collect(),
        outcome: None,
        program_status: Some(Default::default()),
        ..Default::default()
    }
}

fn hook_observation(activity: Activity, needs_you: bool) -> AgentObservation {
    AgentObservation {
        source: ObservationSource::Hook,
        activity,
        interactions: needs_you.then(interaction).into_iter().collect(),
        outcome: None,
        ..Default::default()
    }
}

fn spawn_codex_direct(
    runtime: Arc<ScriptedRuntime>,
) -> (Arc<SessionManager>, Arc<Capture>, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool_with_schema();
    let mut codex_role = role("codex", &[]);
    codex_role.runtime = Runtime::Codex.key().into();
    insert_role_row(&pool.get().unwrap(), &codex_role);
    let mgr = manager_with_runtime(Default::default(), runtime);
    let events = capture_for(&mgr);
    let spawned = mgr
        .spawn_direct(
            &codex_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
            dir.path(),
            Arc::clone(&pool),
            events.clone(),
        )
        .unwrap();
    // Simulate a Windows `.cmd` first-turn spawn whose `StartupDelivery` holds reports
    // for the pending first turn.
    {
        let state = mgr.session_state(&spawned.id).unwrap();
        let mut state = state.lock().unwrap();
        state.handle.as_mut().unwrap().startup = Some(StartupDelivery::new(
            Runtime::Codex.key(),
            true,
            Instant::now(),
        ));
    }
    (mgr, events, spawned.id, dir)
}

#[test]
fn pending_first_turn_codex_waits_past_split_2004h_for_session_start_and_preserves_bracketed() {
    let now = Instant::now();
    let mut startup = StartupDelivery::new(Runtime::Codex.key(), true, now);
    let mut pending = PendingFirstTurn::new("first\nsecond".into(), now + Duration::from_secs(10));

    // Codex emits ?2004h ~50ms in while app-server bootstrap is still running.
    pending.observe(b"\x1b[?20");
    assert_eq!(
        pending.decision(Some(&startup), false, now),
        ReadinessDecision::Wait
    );
    assert!(!pending.bracketed());

    pending.observe(b"04h");
    assert!(pending.bracketed());
    assert_eq!(
        pending.decision(Some(&startup), false, now + Duration::from_millis(50)),
        ReadinessDecision::Wait,
        "Codex must not paste during startup composer bootstrap on ?2004h alone"
    );

    // SessionStart arrives after bootstrap finishes: ready for first turn, while
    // StartupDelivery itself keeps holding inbox/reports until the first turn finishes.
    startup.program_session_started();
    assert_eq!(
        pending.decision(Some(&startup), false, now + Duration::from_secs(3)),
        ReadinessDecision::Ready
    );
    assert!(pending.bracketed());

    // needs_you pauses readiness even when SessionStart already fired.
    assert_eq!(
        pending.decision(Some(&startup), true, now + Duration::from_secs(3)),
        ReadinessDecision::Wait
    );
}

#[test]
fn pending_first_turn_handles_osc_idle_early_session_start_hooks_and_fallback() {
    let now = Instant::now();

    // 1. OSC 7501 idle unlocks readiness while keeping pending_turn startup hold closed.
    let mut osc_startup = StartupDelivery::new(Runtime::Codex.key(), true, now);
    let mut osc_pending = PendingFirstTurn::new("prompt".into(), now + Duration::from_secs(10));
    osc_pending.observe(b"\x1b[?2004h");
    assert_eq!(
        osc_pending.decision(Some(&osc_startup), false, now),
        ReadinessDecision::Wait
    );
    let released_inbox = osc_startup.observation(&osc_observation(Activity::Idle, false));
    assert!(
        !released_inbox,
        "inbox/report gate must stay held until the first turn completes"
    );
    assert_eq!(
        osc_pending.decision(Some(&osc_startup), false, now + Duration::from_secs(1)),
        ReadinessDecision::Ready
    );

    // 2. If SessionStart fires before ?2004h (e.g. Claude Code CLI init), wait for ?2004h
    // so the paste is bracketed, then paste immediately once ?2004h arrives.
    let mut claude_startup = StartupDelivery::new(Runtime::ClaudeCode.key(), true, now);
    let mut claude_pending =
        PendingFirstTurn::new("line1\nline2".into(), now + Duration::from_secs(10));
    claude_startup.program_session_started();
    assert_eq!(
        claude_pending.decision(Some(&claude_startup), false, now),
        ReadinessDecision::Wait
    );
    claude_pending.observe(b"\x1b[?2004h");
    assert_eq!(
        claude_pending.decision(Some(&claude_startup), false, now),
        ReadinessDecision::Ready
    );
    assert!(claude_pending.bracketed());

    // 3. Hook-based runtime (Copilot) waits for hook Idle/Ready and respects needs_you.
    let mut hook_startup = StartupDelivery::new(Runtime::Copilot.key(), true, now);
    let mut hook_pending = PendingFirstTurn::new("prompt".into(), now + Duration::from_secs(10));
    hook_pending.observe(b"\x1b[?2004h");
    assert_eq!(
        hook_pending.decision(Some(&hook_startup), false, now),
        ReadinessDecision::Wait
    );
    let _ = hook_startup.observation(&hook_observation(Activity::Idle, true));
    assert_eq!(
        hook_pending.decision(Some(&hook_startup), true, now + Duration::from_secs(20)),
        ReadinessDecision::Wait,
        "needs_you must hold past deadline"
    );
    let released = hook_startup.observation(&hook_observation(Activity::Ready, false));
    assert!(!released);
    assert_eq!(
        hook_pending.decision(Some(&hook_startup), false, now + Duration::from_secs(2)),
        ReadinessDecision::Ready
    );

    // 4. Bridge failure or output-only runtime triggers immediately on ?2004h, and
    // falls back on deadline without ?2004h.
    let mut fallback_startup = StartupDelivery::new(Runtime::Codex.key(), true, now);
    let mut fallback_pending =
        PendingFirstTurn::new("prompt".into(), now + Duration::from_secs(10));
    fallback_pending.observe(b"\x1b[?2004h");
    fallback_startup.bridge_failed();
    assert_eq!(
        fallback_pending.decision(Some(&fallback_startup), false, now),
        ReadinessDecision::Ready
    );

    let output_only = StartupDelivery::new(Runtime::Cursor.key(), true, now);
    let silent = PendingFirstTurn::new("prompt".into(), now + Duration::from_secs(10));
    assert_eq!(
        silent.decision(Some(&output_only), false, now + Duration::from_secs(9)),
        ReadinessDecision::Wait
    );
    assert_eq!(
        silent.decision(Some(&output_only), false, now + Duration::from_secs(10)),
        ReadinessDecision::TimedOut
    );
}

#[test]
fn first_turn_submit_confirms_after_first_enter_with_no_second_enter() {
    let t0 = Instant::now();
    let timings = test_timings();
    let baseline = snapshot_with(screen(&["> "]));

    // Case 1: authoritative turn_started signal confirms immediately without waiting for confirm_window.
    let mut submit = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        true,
        timings,
        t0,
    );
    assert_eq!(
        submit.step(Some(&baseline), t0),
        FirstTurnStep::Wait(t0 + timings.submit_delay)
    );
    let mut at_enter = baseline.clone();
    at_enter.delivery.rows = screen(&["> first line", "  second line"]);
    at_enter.authoritative_turn_signals = true;
    assert_eq!(
        submit.step(Some(&at_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    submit.mark_enter_sent(t0 + timings.submit_delay, None);

    let mut started = at_enter.clone();
    started.delivery.turn_started = 1;
    assert_eq!(
        submit.step(Some(&started), t0 + Duration::from_millis(350)),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(submit.attempts(), 1);

    // Case 2: Codex 0.149-0.163 braille spinner title_turn_started confirms immediately.
    let mut title_submit = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        false,
        timings,
        t0,
    );
    assert_eq!(
        title_submit.step(Some(&at_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    title_submit.mark_enter_sent(t0 + timings.submit_delay, None);
    let mut title_started = snapshot_with(screen(&["> first line", "  second line"]));
    title_started.title_turn_started = 1;
    assert_eq!(
        title_submit.step(Some(&title_started), t0 + Duration::from_millis(400)),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(title_submit.attempts(), 1);

    // Case 3: non-authoritative cleared composer confirms after confirm_window with no retry.
    let mut cleared_submit = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        false,
        timings,
        t0,
    );
    let mut non_auth_enter = snapshot_with(screen(&["> first line", "  second line"]));
    non_auth_enter.delivery.unsent = true;
    assert_eq!(
        cleared_submit.step(Some(&non_auth_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    cleared_submit.mark_enter_sent(t0 + timings.submit_delay, None);
    let cleared = snapshot_with(screen(&["> "]));
    assert_eq!(
        cleared_submit.step(
            Some(&cleared),
            t0 + timings.submit_delay + timings.confirm_window
        ),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(cleared_submit.attempts(), 1);

    // Case 4: SessionStart observed + located draft + empty composer after Enter #1,
    // with UserPromptSubmit/working delayed past confirm_window -> no blank retry Enter.
    let mut delayed_signal = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        true,
        timings,
        t0,
    );
    assert_eq!(
        delayed_signal.step(Some(&at_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    delayed_signal.mark_enter_sent(t0 + timings.submit_delay, None);
    let mut empty_composer = snapshot_with(screen(&["> "]));
    empty_composer.authoritative_turn_signals = true;
    assert_eq!(
        delayed_signal.step(
            Some(&empty_composer),
            t0 + timings.submit_delay + timings.confirm_window
        ),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed),
        "empty composer after Enter #1 must not send a blank retry Enter when turn-start signal is delayed"
    );
    assert_eq!(delayed_signal.attempts(), 1);
}

#[test]
fn first_turn_submit_retries_swallowed_newline_across_all_codex_representations() {
    let t0 = Instant::now();
    let timings = test_timings();
    let baseline = snapshot_with(screen(&["> "]));

    // 1. Authoritative mode (Codex 0.172+ with SessionStart/OSC 7501): collapsed placeholder
    // increments char count from swallowed newline -> retries once -> confirms on turn_started.
    let mut auth = FirstTurnSubmit::new("a\nb\nc".into(), baseline.clone(), true, timings, t0);
    let mut pasted = baseline.clone();
    pasted.authoritative_turn_signals = true;
    pasted.delivery.rows = screen(&["> [Pasted Content 3421 chars]"]);
    assert_eq!(
        auth.step(Some(&pasted), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    auth.mark_enter_sent(t0 + timings.submit_delay, None);

    let mut swallowed = pasted.clone();
    swallowed.delivery.rows = screen(&["> [Pasted Content 3422 chars]"]);
    let t1 = t0 + timings.submit_delay + timings.confirm_window;
    assert_eq!(
        auth.step(Some(&swallowed), t1),
        FirstTurnStep::ResendEnter(RetryReason::PasteEvidence)
    );
    auth.mark_enter_sent(t1, Some(swallowed.delivery.rows.clone()));

    let mut started = swallowed.clone();
    started.delivery.turn_started = 1;
    assert_eq!(
        auth.step(Some(&started), t1 + Duration::from_millis(50)),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(auth.attempts(), 2);

    // 2. Non-authoritative shifted multiline paste: swallowed newline pushes lines up and
    // leaves a blank continuation row (where InputProbe alone would report false Cleared).
    // Even if the first retry is ignored during bootstrap (leaving the shifted rows unchanged),
    // the verified shifted draft location is retained across retries until it clears.
    let shift_baseline = snapshot_with(vec![
        String::new(),
        String::new(),
        String::new(),
        "> ".into(),
    ]);
    let mut shifted = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        shift_baseline.clone(),
        false,
        timings,
        t0,
    );
    let mut before_enter = snapshot_with(vec![
        String::new(),
        String::new(),
        "> first line".into(),
        "  second line".into(),
    ]);
    before_enter.delivery.unsent = true;
    assert_eq!(
        shifted.step(Some(&before_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    shifted.mark_enter_sent(t0 + timings.submit_delay, None);

    let after_swallow = snapshot_with(vec![
        String::new(),
        "> first line".into(),
        "  second line".into(),
        String::new(),
    ]);
    assert_eq!(
        shifted.step(Some(&after_swallow), t1),
        FirstTurnStep::ResendEnter(RetryReason::PasteEvidence),
        "swallowed newline that shifted multiline draft upward must retry rather than false-confirm"
    );
    shifted.mark_enter_sent(t1, Some(after_swallow.delivery.rows.clone()));

    // First retry is ignored during bootstrap: screen stays identical -> must retry again!
    let t2 = t1 + timings.confirm_window;
    assert_eq!(
        shifted.step(Some(&after_swallow), t2),
        FirstTurnStep::ResendEnter(RetryReason::PasteEvidence),
        "ignored retry after swallowed-newline shift must retain shifted draft and retry again"
    );
    shifted.mark_enter_sent(t2, Some(after_swallow.delivery.rows.clone()));

    // Second retry clears the draft and restores the empty prompt -> Confirmed.
    let t3 = t2 + timings.confirm_window;
    assert_eq!(
        shifted.step(Some(&shift_baseline), t3),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(shifted.attempts(), 3);

    // Terminal height change after locating the draft must NOT falsely report Confirmed
    // while ProbeState is Unknown and no native turn signal has arrived.
    let mut resized = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        shift_baseline,
        true,
        timings,
        t0,
    );
    assert_eq!(
        resized.step(Some(&before_enter), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    resized.mark_enter_sent(t0 + timings.submit_delay, None);
    let taller_screen = snapshot_with(vec![
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "> first line".into(),
        "  second line".into(),
    ]);
    assert_eq!(
        resized.step(Some(&taller_screen), t1),
        FirstTurnStep::Wait(t1 + timings.confirm_window),
        "terminal height change with Unknown probe must wait for native signal rather than false-confirm"
    );

    // 3. Collapsed paste and late plain-text echo with non-empty history above and a footer below:
    // submitted transcript rows replacing "old output" above an unchanged prompt must NOT retry.
    let history_baseline = snapshot_with(screen(&["old output", "> ", "footer"]));
    let mut history_collapsed = FirstTurnSubmit::new(
        "a\nb\nc".into(),
        history_baseline.clone(),
        false,
        timings,
        t0,
    );
    let history_pasted = snapshot_with(screen(&[
        "old output",
        "> [Pasted Content 3421 chars]",
        "footer",
    ]));
    assert_eq!(
        history_collapsed.step(Some(&history_pasted), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    history_collapsed.mark_enter_sent(t0 + timings.submit_delay, None);
    let history_swallowed = snapshot_with(screen(&[
        "old output",
        "> [Pasted Content 3422 chars]",
        "footer",
    ]));
    assert_eq!(
        history_collapsed.step(Some(&history_swallowed), t1),
        FirstTurnStep::ResendEnter(RetryReason::PasteEvidence)
    );
    history_collapsed.mark_enter_sent(t1, Some(history_swallowed.delivery.rows.clone()));
    let history_submitted = snapshot_with(screen(&["[Pasted Content 3422 chars]", "> ", "footer"]));
    assert_eq!(
        history_collapsed.step(Some(&history_submitted), t2),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed),
        "collapsed paste scrolled into history above a restored prompt must confirm"
    );
    assert_eq!(history_collapsed.attempts(), 2);

    // Same history layout where Enter #1 succeeds immediately (1 Enter total).
    let mut history_once = FirstTurnSubmit::new(
        "a\nb\nc".into(),
        history_baseline.clone(),
        false,
        timings,
        t0,
    );
    assert_eq!(
        history_once.step(Some(&history_pasted), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    history_once.mark_enter_sent(t0 + timings.submit_delay, None);
    let history_submitted_once =
        snapshot_with(screen(&["[Pasted Content 3421 chars]", "> ", "footer"]));
    assert_eq!(
        history_once.step(Some(&history_submitted_once), t1),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(history_once.attempts(), 1);

    // Late plain-text echo: baseline == at_enter ["old output", "> ", "footer"], Enter #1
    // submits immediately and transcript replaces "old output" with "  second line" while
    // prompt "> " remains empty, with no hook until after confirm_window -> MUST NOT retry.
    let mut late_plain_history = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        history_baseline.clone(),
        true,
        timings,
        t0,
    );
    assert_eq!(
        late_plain_history.step(Some(&history_baseline), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    late_plain_history.mark_enter_sent(t0 + timings.submit_delay, None);
    let plain_history_after_submit = snapshot_with(screen(&["  second line", "> ", "footer"]));
    assert_eq!(
        late_plain_history.step(Some(&plain_history_after_submit), t1),
        FirstTurnStep::Wait(t1 + timings.confirm_window),
        "submitted plain-text transcript replacing old output above unchanged prompt must not trigger retry"
    );
    let mut delayed_hook = plain_history_after_submit;
    delayed_hook.delivery.turn_started = 1;
    assert_eq!(
        late_plain_history.step(Some(&delayed_hook), t1 + Duration::from_millis(100)),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(late_plain_history.attempts(), 1);
}

#[test]
fn first_turn_submit_retries_during_bootstrap_and_stops_at_deadline_or_human_input() {
    let t0 = Instant::now();
    let timings = test_timings();
    let baseline = snapshot_with(screen(&["> "]));

    // 1. Codex 0.149-0.166 startup composer ignores Enter until bootstrap ends after 5s.
    let mut bootstrap = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        false,
        timings,
        t0,
    );
    let draft = snapshot_with(screen(&["> first line", "  second line"]));
    let mut now = t0 + timings.submit_delay;
    assert_eq!(
        bootstrap.step(Some(&draft), now),
        FirstTurnStep::SendFirstEnter
    );
    bootstrap.mark_enter_sent(now, None);
    for _ in 0..3 {
        now += timings.confirm_window;
        assert_eq!(
            bootstrap.step(Some(&draft), now),
            FirstTurnStep::ResendEnter(RetryReason::ProbeOrTracker)
        );
        bootstrap.mark_enter_sent(now, None);
    }
    now += timings.confirm_window;
    let cleared = snapshot_with(screen(&["> "]));
    assert_eq!(
        bootstrap.step(Some(&cleared), now),
        FirstTurnStep::Finish(FirstTurnOutcome::Confirmed)
    );
    assert_eq!(bootstrap.attempts(), 4);

    // 2. Late-flushed paste (echo arrives only after Enter #1) retries once, while a
    // non-echoing stub (`current_rows == baseline_rows`) finishes Unverified with 1 Enter.
    let mut late = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        false,
        timings,
        t0,
    );
    assert_eq!(
        late.step(Some(&baseline), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    late.mark_enter_sent(t0 + timings.submit_delay, None);
    assert_eq!(
        late.step(
            Some(&draft),
            t0 + timings.submit_delay + timings.confirm_window
        ),
        FirstTurnStep::ResendEnter(RetryReason::ProbeOrTracker)
    );

    let mut non_echo = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        false,
        timings,
        t0,
    );
    assert_eq!(
        non_echo.step(Some(&baseline), t0 + timings.submit_delay),
        FirstTurnStep::SendFirstEnter
    );
    non_echo.mark_enter_sent(t0 + timings.submit_delay, None);
    assert_eq!(
        non_echo.step(
            Some(&baseline),
            t0 + timings.submit_delay + timings.confirm_window
        ),
        FirstTurnStep::Finish(FirstTurnOutcome::Unverified)
    );
    assert_eq!(non_echo.attempts(), 1);

    // 3. needs_you before first Enter holds in AwaitingFirstEnter and still sends Enter #1 once cleared.
    let mut paused = FirstTurnSubmit::new("prompt".into(), baseline.clone(), false, timings, t0);
    let mut blocked = draft.clone();
    blocked.needs_you = true;
    let t_enter = t0 + timings.submit_delay;
    assert_eq!(
        paused.step(Some(&blocked), t_enter),
        FirstTurnStep::Wait(t_enter + timings.confirm_window)
    );
    assert_eq!(paused.attempts(), 0);
    let t_unblocked = t_enter + timings.confirm_window;
    assert_eq!(
        paused.step(Some(&draft), t_unblocked),
        FirstTurnStep::SendFirstEnter
    );

    // 4. Zero-attempt deadline expiry terminates AwaitingFirstEnter without sending Enter
    // or spinning, both for a delayed worker and after declined first sends.
    let deadline = t_enter + timings.submit_timeout;
    let mut delayed_first =
        FirstTurnSubmit::new("prompt".into(), baseline.clone(), false, timings, t0);
    assert_eq!(
        delayed_first.step(Some(&draft), deadline),
        FirstTurnStep::Finish(FirstTurnOutcome::TimedOut { attempts: 0 }),
        "expired AwaitingFirstEnter must terminate without sending a late first Enter"
    );

    let mut declined_first =
        FirstTurnSubmit::new("prompt".into(), baseline.clone(), false, timings, t0);
    assert_eq!(
        declined_first.step(Some(&draft), t_enter),
        FirstTurnStep::SendFirstEnter
    );
    declined_first.defer_retry(deadline);
    assert_eq!(
        declined_first.step(Some(&draft), deadline),
        FirstTurnStep::Finish(FirstTurnOutcome::TimedOut { attempts: 0 }),
        "declined first Enter capped at expired deadline must terminate instead of spinning"
    );

    // 5. Bounded deadline stops retries; human input and session exit stop immediately.
    let mut timed = FirstTurnSubmit::new(
        "first line\nsecond line".into(),
        baseline.clone(),
        true,
        timings,
        t0,
    );
    let mut auth_draft = draft.clone();
    auth_draft.authoritative_turn_signals = true;
    assert_eq!(
        timed.step(Some(&auth_draft), t_enter),
        FirstTurnStep::SendFirstEnter
    );
    timed.mark_enter_sent(t_enter, None);
    assert_eq!(
        timed.step(Some(&auth_draft), deadline),
        FirstTurnStep::Finish(FirstTurnOutcome::TimedOut { attempts: 1 })
    );

    let mut human = FirstTurnSubmit::new("prompt".into(), baseline.clone(), true, timings, t0);
    let mut typed = auth_draft.clone();
    typed.delivery.input_revision = (1, 0);
    assert_eq!(
        human.step(Some(&typed), t0),
        FirstTurnStep::Finish(FirstTurnOutcome::HumanInput)
    );

    let mut ended = FirstTurnSubmit::new("prompt".into(), baseline, true, timings, t0);
    assert_eq!(
        ended.step(None, t0),
        FirstTurnStep::Finish(FirstTurnOutcome::SessionEnded)
    );
}

#[test]
fn manager_codex_first_turn_waits_for_session_start_confirms_on_first_enter_and_keeps_inbox_held() {
    let runtime = ScriptedRuntime::new(|index, bytes, tx| {
        if index == 0 {
            let _ = tx.send(RuntimeOutput::Stream(
                b"> first line\r\n  second line".to_vec(),
            ));
        } else if index == 1 {
            assert_eq!(bytes, b"\r");
            send_agent_sync(
                tx,
                AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                },
            );
        }
    });
    let (mgr, _events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_secs(2),
        submit_timeout: Duration::from_secs(5),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );

    // Split ?2004h during Codex bootstrap must NOT trigger paste yet.
    runtime.push_output(b"\x1b[?20");
    runtime.push_output(b"04h> ");
    runtime.push_agent_sync(AgentEvent::EditorDraft { drafting: false });
    assert!(
        runtime.writes().is_empty(),
        "must wait for SessionStart instead of pasting on ?2004h"
    );

    // Bootstrap finishes: SessionStart triggers bracketed paste + single Enter, confirmed once.
    runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr.wait_for_first_turn_worker(&id);

    let writes = runtime.writes();
    assert_eq!(
        writes,
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    assert!(mgr.first_turn_warnings().is_empty());
    // StartupDelivery still holds inbox/reports until the first turn completes.
    mgr.note_forwarder_transition(&id, SessionActivityState::Idle, StatusSource::Forwarder);
    assert_eq!(
        mgr.delivery_readiness(&id),
        crate::router::DeliveryReadiness::Starting
    );
    runtime.push_agent_sync(AgentEvent::ProgramTurnFinished);
    assert_eq!(
        mgr.delivery_readiness(&id),
        crate::router::DeliveryReadiness::Ready { idle: true }
    );
    mgr.kill(&id).unwrap();
}

#[test]
fn manager_codex_first_turn_retries_swallowed_enter_and_starts_once() {
    let runtime = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 23 chars]".to_vec(),
            ));
        }
        1 => {
            assert_eq!(bytes, b"\r");
            // Enter #1 swallowed inside paste-burst window as a newline.
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 24 chars]".to_vec(),
            ));
        }
        2 => {
            assert_eq!(bytes, b"\r");
            // Clean Enter #2 starts the turn once.
            send_agent_sync(
                tx,
                AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                },
            );
        }
        other => panic!("unexpected extra write #{other}: {bytes:?}"),
    });
    let (mgr, _events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime.push_output(b"\x1b[?2004h> ");
    runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr.wait_for_first_turn_worker(&id);

    let writes = runtime.writes();
    assert_eq!(
        writes,
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
            b"\r".to_vec(),
        ]
    );
    assert!(mgr.first_turn_warnings().is_empty());
    mgr.kill(&id).unwrap();
}

#[test]
fn manager_codex_startup_composer_without_hooks_retries_until_bootstrap_ends() {
    let runtime = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> first line\r\n  second line".to_vec(),
            ));
        }
        1 | 2 => {
            assert_eq!(bytes, b"\r");
            // Codex 0.149-0.166 startup_draft ignores plain Enter while bootstrapping.
        }
        3 => {
            assert_eq!(bytes, b"\r");
            // Bootstrap finished: composer clears and Codex title spinner marks turn busy.
            let _ = tx.send(RuntimeOutput::Stream(b"\x1b[2J\x1b[H> ".to_vec()));
            let _ = tx.send(RuntimeOutput::TerminalEvent(TerminalEvent::Title(
                SessionActivityState::Busy,
            )));
            send_agent_sync(tx, AgentEvent::EditorDraft { drafting: false });
        }
        other => panic!("unexpected extra write #{other}: {bytes:?}"),
    });
    let (mgr, _events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime.push_output(b"\x1b[?2004h> ");
    // Older Codex without mcp_tool hooks falls back via StatusBridgeFailed -> pastes on ?2004h.
    runtime.push(RuntimeOutput::StatusBridgeFailed);
    mgr.wait_for_first_turn_worker(&id);

    let writes = runtime.writes();
    assert_eq!(
        writes,
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
            b"\r".to_vec(),
            b"\r".to_vec(),
        ]
    );
    assert!(mgr.first_turn_warnings().is_empty());
    mgr.kill(&id).unwrap();
}

#[test]
fn manager_first_turn_deadline_stops_retries_and_logs_warning() {
    let runtime = ScriptedRuntime::new(|index, _bytes, tx| {
        if index == 0 {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> first line\r\n  second line".to_vec(),
            ));
            send_agent_sync(tx, AgentEvent::EditorDraft { drafting: false });
        }
    });
    let (mgr, _events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(10),
        confirm_window: Duration::from_millis(30),
        submit_timeout: ci_scaled_budget(Duration::from_millis(200)),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime.push_output(b"\x1b[?2004h> ");
    runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr.wait_for_first_turn_worker(&id);

    let writes = runtime.writes();
    assert!(
        writes.len() >= 3,
        "expected paste + first Enter + at least one retry before deadline: {writes:?}"
    );
    assert_eq!(writes[0], b"\x1b[200~first line\nsecond line\x1b[201~");
    assert!(writes[1..].iter().all(|write| write == b"\r"));
    let warnings = mgr.first_turn_warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("first-turn submit timed out")),
        "expected timeout warning, got {warnings:?}"
    );
    mgr.kill(&id).unwrap();
}

#[test]
fn manager_codex_empty_composer_with_delayed_turn_start_sends_exactly_one_enter() {
    let runtime = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> first line\r\n  second line".to_vec(),
            ));
        }
        1 => {
            assert_eq!(bytes, b"\r");
            // Enter #1 starts the turn and clears the composer, but UserPromptSubmit is delayed.
            let _ = tx.send(RuntimeOutput::Stream(b"\x1b[2J\x1b[H> ".to_vec()));
            send_agent_sync(tx, AgentEvent::EditorDraft { drafting: false });
        }
        other => panic!("unexpected extra write #{other}: {bytes:?}"),
    });
    let (mgr, _events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime.push_output(b"\x1b[?2004h> ");
    runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr.wait_for_first_turn_worker(&id);
    // Late UserPromptSubmit arrives after confirm_window has already settled on the cleared composer.
    runtime.push_agent_sync(AgentEvent::ProgramTurnConfirmed {
        conversation_changed: false,
        prompt: None,
    });

    assert_eq!(
        runtime.writes(),
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    assert!(mgr.first_turn_warnings().is_empty());
    mgr.kill(&id).unwrap();
}

#[test]
fn manager_retry_revalidates_screen_clear_and_turn_start_at_enqueue() {
    // 1. Draft clears between step() deciding ResendEnter(ProbeOrTracker) and enqueue -> declined.
    let runtime_clear = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> first line\r\n  second line".to_vec(),
            ));
        }
        1 => {
            assert_eq!(bytes, b"\r");
        }
        other => {
            panic!("retry must be declined after live screen clear, got write #{other}: {bytes:?}")
        }
    });
    let (mgr_clear, _events, id_clear, _dir) = spawn_codex_direct(Arc::clone(&runtime_clear));
    mgr_clear.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    let rt_for_clear = Arc::clone(&runtime_clear);
    mgr_clear.set_before_first_turn_retry(move || {
        rt_for_clear.push_output(b"\x1b[2J\x1b[H> ");
        rt_for_clear.push_agent_sync(AgentEvent::EditorDraft { drafting: false });
    });
    mgr_clear.queue_pending_first_turn(
        &id_clear,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime_clear.push_output(b"\x1b[?2004h> ");
    runtime_clear.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr_clear.wait_for_first_turn_worker(&id_clear);
    assert_eq!(
        runtime_clear.writes(),
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    mgr_clear.kill(&id_clear).unwrap();

    // 2. ProgramTurnConfirmed arrives between step() deciding ResendEnter(PasteEvidence) and enqueue -> declined.
    let runtime_turn = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 23 chars]".to_vec(),
            ));
        }
        1 => {
            assert_eq!(bytes, b"\r");
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 24 chars]".to_vec(),
            ));
        }
        other => panic!("retry must be declined after turn start, got write #{other}: {bytes:?}"),
    });
    let (mgr_turn, _events, id_turn, _dir) = spawn_codex_direct(Arc::clone(&runtime_turn));
    mgr_turn.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    let rt_for_turn = Arc::clone(&runtime_turn);
    mgr_turn.set_before_first_turn_retry(move || {
        rt_for_turn.push_agent_sync(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        });
    });
    mgr_turn.queue_pending_first_turn(
        &id_turn,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime_turn.push_output(b"\x1b[?2004h> ");
    runtime_turn.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr_turn.wait_for_first_turn_worker(&id_turn);
    assert_eq!(
        runtime_turn.writes(),
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    mgr_turn.kill(&id_turn).unwrap();

    // 3. Collapsed paste clears between step() deciding ResendEnter(PasteEvidence) and enqueue -> declined.
    let runtime_paste_clear = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 23 chars]".to_vec(),
            ));
        }
        1 => {
            assert_eq!(bytes, b"\r");
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> [Pasted Content 24 chars]".to_vec(),
            ));
        }
        other => panic!(
            "paste-evidence retry must be declined after clear, got write #{other}: {bytes:?}"
        ),
    });
    let (mgr_paste, _events, id_paste, _dir) = spawn_codex_direct(Arc::clone(&runtime_paste_clear));
    mgr_paste.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    let rt_for_paste = Arc::clone(&runtime_paste_clear);
    mgr_paste.set_before_first_turn_retry(move || {
        rt_for_paste.push_output(b"\x1b[2J\x1b[H> ");
        rt_for_paste.push_agent_sync(AgentEvent::EditorDraft { drafting: false });
    });
    mgr_paste.queue_pending_first_turn(
        &id_paste,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime_paste_clear.push_output(b"\x1b[?2004h> ");
    runtime_paste_clear.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    mgr_paste.wait_for_first_turn_worker(&id_paste);
    assert_eq!(
        runtime_paste_clear.writes(),
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    mgr_paste.kill(&id_paste).unwrap();
}

#[test]
fn manager_preserves_human_draft_typed_before_session_start_while_allowing_trust_dialog_input() {
    // 1. User types a draft between ?2004h and SessionStart: automatic paste/Enter must NOT join it.
    let draft_runtime = ScriptedRuntime::new(|_index, _bytes, _tx| {});
    let (mgr, events, id, _dir) = spawn_codex_direct(Arc::clone(&draft_runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(10),
        confirm_window: Duration::from_millis(40),
        submit_timeout: Duration::from_secs(5),
    });
    mgr.queue_pending_first_turn(
        &id,
        "role system prompt\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    draft_runtime.push_output(b"\x1b[?2004h> ");
    draft_runtime.push_agent_sync(AgentEvent::EditorDraft { drafting: false });
    mgr.inject_direct_stdin(&id, b"my own draft", events.as_ref())
        .unwrap();
    draft_runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    // Forwarder replies to ProgramSessionStarted before calling deliver_pending_first_turn;
    // a second synchronous event ensures that post-reply discard has finished before asserting.
    draft_runtime.push_agent_sync(AgentEvent::EditorDraft { drafting: true });
    assert_eq!(
        draft_runtime.writes(),
        vec![b"my own draft".to_vec()],
        "automatic first-turn paste/Enter must not join or submit a human draft typed during bootstrap"
    );
    {
        let state = mgr.session_state(&id).unwrap();
        let state = state.lock().unwrap();
        let handle = state.handle.as_ref().unwrap();
        assert!(handle.pending_first_turn.is_none());
        assert!(handle.first_turn_worker.is_none());
    }
    mgr.kill(&id).unwrap();

    // 2. User clears a folder-trust startup interaction (StartupDelivery::for_reports has
    // trust_cleared == false) before SessionStart -> first turn still pastes and submits once.
    let trust_runtime = ScriptedRuntime::new(|index, bytes, tx| match index {
        0 => {
            // Human Enter clearing the trust prompt.
            assert_eq!(bytes, b"\r");
        }
        1 => {
            // Automatic first-turn bracketed paste.
            let _ = tx.send(RuntimeOutput::Stream(
                b"\r> role system prompt\r\n  second line".to_vec(),
            ));
        }
        2 => {
            assert_eq!(bytes, b"\r");
            send_agent_sync(
                tx,
                AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                },
            );
        }
        other => panic!("unexpected extra write #{other}: {bytes:?}"),
    });
    let (trust_mgr, trust_events, trust_id, _trust_dir) =
        spawn_codex_direct(Arc::clone(&trust_runtime));
    {
        let state = trust_mgr.session_state(&trust_id).unwrap();
        let mut state = state.lock().unwrap();
        state.handle.as_mut().unwrap().startup = Some(StartupDelivery::for_reports(
            Runtime::Codex.key(),
            true,
            Instant::now(),
        ));
    }
    trust_mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(15),
        confirm_window: Duration::from_secs(2),
        submit_timeout: Duration::from_secs(5),
    });
    trust_mgr.queue_pending_first_turn(
        &trust_id,
        "role system prompt\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    trust_runtime.push_output(b"\x1b[?2004hDo you trust this folder? ");
    trust_runtime.push_agent_sync(AgentEvent::EditorDraft { drafting: false });
    // User accepts the folder-trust prompt.
    trust_mgr
        .inject_direct_stdin(&trust_id, b"\r", trust_events.as_ref())
        .unwrap();
    trust_runtime.push_output(b"\x1b[2J\x1b[H> ");
    trust_runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    trust_mgr.wait_for_first_turn_worker(&trust_id);
    assert_eq!(
        trust_runtime.writes(),
        vec![
            b"\r".to_vec(),
            b"\x1b[200~role system prompt\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
        ]
    );
    trust_mgr.kill(&trust_id).unwrap();
}

#[test]
fn manager_first_turn_stops_on_human_input_and_kill_joins_worker() {
    // 1. Human input cancels pending retries immediately.
    let (enter_tx, enter_rx) = std::sync::mpsc::channel();
    let runtime = ScriptedRuntime::new(move |index, _bytes, _tx| {
        if index == 1 {
            let _ = enter_tx.send(());
        }
    });
    let (mgr, events, id, _dir) = spawn_codex_direct(Arc::clone(&runtime));
    mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(10),
        confirm_window: Duration::from_secs(5),
        submit_timeout: Duration::from_secs(15),
    });
    mgr.queue_pending_first_turn(
        &id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    runtime.push_output(b"\x1b[?2004h> ");
    runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    enter_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first Enter sent");
    mgr.inject_direct_stdin(&id, b"manual", events.as_ref())
        .unwrap();
    mgr.wait_for_first_turn_worker(&id);
    assert_eq!(
        runtime.writes(),
        vec![
            b"\x1b[200~first line\nsecond line\x1b[201~".to_vec(),
            b"\r".to_vec(),
            b"manual".to_vec(),
        ]
    );
    mgr.kill(&id).unwrap();

    // 2. Killing a session while the first-turn worker is waiting exits promptly without hanging.
    let (kill_enter_tx, kill_enter_rx) = std::sync::mpsc::channel();
    let kill_runtime = ScriptedRuntime::new(move |index, _bytes, _tx| {
        if index == 1 {
            let _ = kill_enter_tx.send(());
        }
    });
    let (kill_mgr, _kill_events, kill_id, _kill_dir) =
        spawn_codex_direct(Arc::clone(&kill_runtime));
    kill_mgr.set_first_turn_timings(FirstTurnTimings {
        submit_delay: Duration::from_millis(10),
        confirm_window: Duration::from_secs(30),
        submit_timeout: Duration::from_secs(60),
    });
    kill_mgr.queue_pending_first_turn(
        &kill_id,
        "first line\nsecond line",
        Instant::now() + Duration::from_secs(10),
    );
    kill_runtime.push_output(b"\x1b[?2004h> ");
    kill_runtime.push_agent_sync(AgentEvent::ProgramSessionStarted {
        conversation_changed: false,
        compacted: false,
    });
    kill_enter_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first Enter sent before kill");
    let started_kill = Instant::now();
    kill_mgr.kill(&kill_id).unwrap();
    assert!(
        started_kill.elapsed() < Duration::from_secs(5),
        "kill must wake and join the waiting first-turn worker promptly"
    );
}
