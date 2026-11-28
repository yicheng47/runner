use super::input_writer::Written;
use super::*;
use runner_terminal::delivery_probe::{InputProbe, ProbeState};

/// Windows Codex suppresses Enter during its paste-burst window (#753).
/// Matches the inbox path's Windows `SUBMIT_DELAY`.
pub(super) const FIRST_TURN_SUBMIT_DELAY: Duration = Duration::from_millis(300);
pub(super) const FIRST_TURN_SUBMIT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug)]
pub(super) struct FirstTurnTimings {
    pub(super) submit_delay: Duration,
    pub(super) confirm_window: Duration,
    pub(super) submit_timeout: Duration,
}

impl FirstTurnTimings {
    pub(super) const WINDOWS_DEFAULT: Self = Self {
        submit_delay: FIRST_TURN_SUBMIT_DELAY,
        confirm_window: crate::router::dispatch::CONFIRM_WINDOW,
        submit_timeout: FIRST_TURN_SUBMIT_TIMEOUT,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadinessDecision {
    Wait,
    Ready,
    TimedOut,
}

pub(super) struct PendingFirstTurn {
    pub(super) body: String,
    pub(super) deadline: Instant,
    readiness: super::super::runtime::TuiReadiness,
    bracketed: bool,
    tui_ready: bool,
    human_intervened: bool,
    initial_turn_started: u64,
    initial_title_turn_started: u64,
}

impl PendingFirstTurn {
    #[cfg(any(windows, test))]
    pub(super) fn new(body: String, deadline: Instant) -> Self {
        Self {
            body,
            deadline,
            readiness: Default::default(),
            bracketed: false,
            tui_ready: false,
            human_intervened: false,
            initial_turn_started: 0,
            initial_title_turn_started: 0,
        }
    }

    #[cfg(any(windows, test))]
    fn with_initial_turns(mut self, turn_started: u64, title_turn_started: u64) -> Self {
        self.initial_turn_started = turn_started;
        self.initial_title_turn_started = title_turn_started;
        self
    }

    pub(super) fn mark_human_intervened(&mut self) {
        self.human_intervened = true;
    }

    pub(super) fn human_intervened(&self) -> bool {
        self.human_intervened
    }

    pub(super) fn observe(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let seen = self.readiness.observe(bytes);
        self.bracketed |= seen[0];
        self.tui_ready |= seen.into_iter().any(|seen| seen);
    }

    pub(super) fn bracketed(&self) -> bool {
        self.bracketed
    }

    /// Runtimes with OSC 7501 or startup hooks wait for `StartupDelivery` to report
    /// the composer ready for input instead of pasting on `?2004h`, which Codex emits
    /// ~50 ms into a 1-8+ s app-server bootstrap. Output-only runtimes keep
    /// `TuiReadiness`, and both fall back on `deadline` unless `needs_you` is active.
    pub(super) fn decision(
        &self,
        startup: Option<&super::startup::StartupDelivery>,
        needs_you: bool,
        now: Instant,
    ) -> ReadinessDecision {
        if needs_you {
            return ReadinessDecision::Wait;
        }
        let ready = match startup {
            None => true,
            Some(startup) => startup.ready_for_first_turn(self.tui_ready),
        };
        if ready {
            ReadinessDecision::Ready
        } else if now >= self.deadline {
            ReadinessDecision::TimedOut
        } else {
            ReadinessDecision::Wait
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FirstTurnSnapshot {
    pub(super) delivery: router::DeliverySnapshot,
    pub(super) title_turn_started: u64,
    pub(super) authoritative_turn_signals: bool,
    pub(super) needs_you: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FirstTurnOutcome {
    Confirmed,
    SessionEnded,
    HumanInput,
    Unverified,
    TimedOut { attempts: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetryReason {
    ProbeOrTracker,
    PasteEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FirstTurnStep {
    Wait(Instant),
    SendFirstEnter,
    ResendEnter(RetryReason),
    Finish(FirstTurnOutcome),
}

enum SubmitPhase {
    AwaitingFirstEnter { enter_at: Instant },
    Confirming { next_check: Instant },
}

/// Verified location of a multiline draft whose lines shifted upward when an Enter
/// was swallowed as a newline. Retained across retries so an ignored retry keeps
/// recognizing the still-visible draft instead of losing track after the first shift.
#[derive(Clone, Debug)]
struct ShiftedDraft {
    composer_row: usize,
    draft_line: String,
    shifted_row: usize,
}

/// Pure, clock-driven submit and retry state machine for a pasted first turn.
/// Reuses `CONFIRM_WINDOW`, `InputProbe`, `DeliverySnapshot`, and the session's
/// turn-start signals so the full decision logic compiles and is tested on every
/// platform while only the Windows `.cmd`/`.bat` launch path triggers it.
pub(super) struct FirstTurnSubmit {
    body: String,
    baseline: FirstTurnSnapshot,
    expects_turn_signals: bool,
    timings: FirstTurnTimings,
    probe: InputProbe,
    at_enter_rows: Vec<String>,
    collapsed_composer_row: Option<usize>,
    shifted_draft: Option<ShiftedDraft>,
    phase: SubmitPhase,
    deadline: Instant,
    attempts: u32,
}

impl FirstTurnSubmit {
    pub(super) fn new(
        body: String,
        baseline: FirstTurnSnapshot,
        expects_turn_signals: bool,
        timings: FirstTurnTimings,
        now: Instant,
    ) -> Self {
        let probe = InputProbe::new(baseline.delivery.rows.clone(), &body);
        let enter_at = now + timings.submit_delay;
        let deadline = enter_at + timings.submit_timeout;
        Self {
            body,
            baseline,
            expects_turn_signals,
            timings,
            probe,
            at_enter_rows: Vec::new(),
            collapsed_composer_row: None,
            shifted_draft: None,
            phase: SubmitPhase::AwaitingFirstEnter { enter_at },
            deadline,
            attempts: 0,
        }
    }

    pub(super) fn baseline(&self) -> &FirstTurnSnapshot {
        &self.baseline
    }

    pub(super) fn probe(&self) -> &InputProbe {
        &self.probe
    }

    pub(super) fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(super) fn step(
        &mut self,
        current: Option<&FirstTurnSnapshot>,
        now: Instant,
    ) -> FirstTurnStep {
        let Some(current) = current else {
            return FirstTurnStep::Finish(FirstTurnOutcome::SessionEnded);
        };
        if current.delivery.generation != self.baseline.delivery.generation {
            return FirstTurnStep::Finish(FirstTurnOutcome::SessionEnded);
        }
        if current.delivery.input_revision != self.baseline.delivery.input_revision {
            return FirstTurnStep::Finish(FirstTurnOutcome::HumanInput);
        }
        if current.delivery.turn_started != self.baseline.delivery.turn_started
            || current.title_turn_started != self.baseline.title_turn_started
        {
            return FirstTurnStep::Finish(FirstTurnOutcome::Confirmed);
        }

        match self.phase {
            SubmitPhase::AwaitingFirstEnter { enter_at } => {
                if now >= self.deadline {
                    return FirstTurnStep::Finish(FirstTurnOutcome::TimedOut {
                        attempts: self.attempts,
                    });
                }
                if now < enter_at {
                    return FirstTurnStep::Wait(enter_at);
                }
                if current.needs_you {
                    let next_check = (now + self.timings.confirm_window).min(self.deadline);
                    self.phase = SubmitPhase::AwaitingFirstEnter {
                        enter_at: next_check,
                    };
                    return FirstTurnStep::Wait(next_check);
                }
                self.probe.locate(&current.delivery.rows);
                self.at_enter_rows = current.delivery.rows.clone();
                self.collapsed_composer_row = locate_collapsed_composer_row(
                    &self.baseline.delivery.rows,
                    &current.delivery.rows,
                );
                self.shifted_draft = None;
                FirstTurnStep::SendFirstEnter
            }
            SubmitPhase::Confirming { next_check } => {
                if now < next_check {
                    return FirstTurnStep::Wait(next_check);
                }
                let probe_state = self.probe.state(&current.delivery.rows);
                let unsent = current.delivery.unsent;

                if probe_state == ProbeState::Present || unsent {
                    return self.retry_or_timeout(
                        RetryReason::ProbeOrTracker,
                        current.needs_you,
                        now,
                    );
                }

                if let Some(reason) =
                    self.detect_unsubmitted_paste(&current.delivery.rows, probe_state)
                {
                    return self.retry_or_timeout(reason, current.needs_you, now);
                }

                let same_height = self.baseline.delivery.rows.len() == current.delivery.rows.len()
                    && !self.baseline.delivery.rows.is_empty();
                let composer_cleared = (probe_state == ProbeState::Cleared
                    || (same_height
                        && (self.collapsed_composer_row.is_some()
                            || self.shifted_draft.is_some())))
                    && !unsent;
                if composer_cleared {
                    return FirstTurnStep::Finish(FirstTurnOutcome::Confirmed);
                }

                if self.expects_turn_signals || current.authoritative_turn_signals {
                    if now >= self.deadline {
                        return FirstTurnStep::Finish(FirstTurnOutcome::TimedOut {
                            attempts: self.attempts,
                        });
                    }
                    let next = (now + self.timings.confirm_window).min(self.deadline);
                    self.phase = SubmitPhase::Confirming { next_check: next };
                    return FirstTurnStep::Wait(next);
                }

                FirstTurnStep::Finish(FirstTurnOutcome::Unverified)
            }
        }
    }

    pub(super) fn mark_enter_sent(&mut self, now: Instant, sent_rows: Option<Vec<String>>) {
        self.attempts = self.attempts.saturating_add(1);
        if self.shifted_draft.is_none() {
            if let Some(rows) = sent_rows {
                self.at_enter_rows = rows;
            }
        }
        let next_check = (now + self.timings.confirm_window).min(self.deadline);
        self.phase = SubmitPhase::Confirming { next_check };
    }

    pub(super) fn defer_retry(&mut self, now: Instant) {
        let next_check = (now + self.timings.confirm_window).min(self.deadline);
        self.phase = if self.attempts == 0 {
            SubmitPhase::AwaitingFirstEnter {
                enter_at: next_check,
            }
        } else {
            SubmitPhase::Confirming { next_check }
        };
    }

    fn retry_or_timeout(
        &mut self,
        reason: RetryReason,
        needs_you: bool,
        now: Instant,
    ) -> FirstTurnStep {
        if now >= self.deadline {
            return FirstTurnStep::Finish(FirstTurnOutcome::TimedOut {
                attempts: self.attempts,
            });
        }
        if needs_you {
            let next = (now + self.timings.confirm_window).min(self.deadline);
            self.phase = SubmitPhase::Confirming { next_check: next };
            return FirstTurnStep::Wait(next);
        }
        FirstTurnStep::ResendEnter(reason)
    }

    /// Detects pasted content that is still sitting unsubmitted in the composer:
    /// 1. A swallowed newline pushed multiline composer rows upward, leaving blank
    ///    continuation row(s) without restoring the empty prompt below them; once
    ///    detected, the shifted draft location is retained across ignored retries.
    /// 2. A collapsed paste placeholder (e.g. `[Pasted Content 3422 chars]`) remains
    ///    in the original composer rather than having scrolled into submitted history.
    /// 3. The paste echoed only after the first Enter (`at_enter_rows == baseline`),
    ///    and positive evidence places that late echo inside the live composer rather
    ///    than submitted transcript history.
    fn detect_unsubmitted_paste(
        &mut self,
        current_rows: &[String],
        probe_state: ProbeState,
    ) -> Option<RetryReason> {
        let baseline_rows = &self.baseline.delivery.rows;
        if baseline_rows.len() != current_rows.len() || baseline_rows.is_empty() {
            return None;
        }
        if let Some(shifted) = self.shifted_draft.as_mut() {
            if let Some(new_row) = shifted_draft_row(baseline_rows, current_rows, shifted) {
                shifted.shifted_row = new_row;
                return Some(RetryReason::PasteEvidence);
            }
            return None;
        }
        if probe_state == ProbeState::Cleared && self.at_enter_rows.len() == baseline_rows.len() {
            if let Some(shifted) =
                detect_shifted_draft(baseline_rows, &self.at_enter_rows, current_rows)
            {
                self.shifted_draft = Some(shifted);
                return Some(RetryReason::PasteEvidence);
            }
        }
        if probe_state == ProbeState::Unknown {
            if let Some(composer_row) = self.collapsed_composer_row {
                if collapsed_paste_still_in_composer(baseline_rows, current_rows, composer_row) {
                    return Some(RetryReason::PasteEvidence);
                }
            } else if self.at_enter_rows == *baseline_rows
                && current_rows != baseline_rows.as_slice()
            {
                if let Some(composer_row) =
                    locate_collapsed_composer_row(baseline_rows, current_rows)
                {
                    self.collapsed_composer_row = Some(composer_row);
                    return Some(RetryReason::PasteEvidence);
                }
                if late_plain_echo_in_live_composer(baseline_rows, current_rows, &self.body) {
                    let mut fresh = InputProbe::new(baseline_rows.clone(), &self.body);
                    fresh.locate(current_rows);
                    if fresh.state(current_rows) == ProbeState::Present {
                        self.probe = fresh;
                        return Some(RetryReason::ProbeOrTracker);
                    }
                }
            }
        }
        None
    }

    /// Revalidates paste evidence against live terminal rows under the session lock
    /// right before enqueueing a retry Enter.
    fn still_unsubmitted_paste(&self, live_rows: &[String]) -> bool {
        let baseline_rows = &self.baseline.delivery.rows;
        if baseline_rows.len() != live_rows.len() || baseline_rows.is_empty() {
            return false;
        }
        if let Some(shifted) = self.shifted_draft.as_ref() {
            return shifted_draft_row(baseline_rows, live_rows, shifted).is_some();
        }
        self.collapsed_composer_row.is_some_and(|composer_row| {
            collapsed_paste_still_in_composer(baseline_rows, live_rows, composer_row)
        })
    }
}

fn detect_shifted_draft(
    baseline_rows: &[String],
    at_enter_rows: &[String],
    current_rows: &[String],
) -> Option<ShiftedDraft> {
    let composer_row = (0..at_enter_rows.len()).rev().find(|&row| {
        at_enter_rows[row] != baseline_rows[row] && !at_enter_rows[row].trim().is_empty()
    })?;
    let candidate = ShiftedDraft {
        composer_row,
        draft_line: at_enter_rows[composer_row].clone(),
        shifted_row: composer_row,
    };
    let shifted_row = shifted_draft_row(baseline_rows, current_rows, &candidate)?;
    Some(ShiftedDraft {
        shifted_row,
        ..candidate
    })
}

fn shifted_draft_row(
    baseline_rows: &[String],
    current_rows: &[String],
    shifted: &ShiftedDraft,
) -> Option<usize> {
    let composer_row = shifted.composer_row;
    if composer_row == 0 || composer_row >= current_rows.len() {
        return None;
    }
    if !current_rows[composer_row].trim().is_empty() {
        return None;
    }
    let upper_bound = shifted.shifted_row.min(composer_row - 1);
    let found_row = (0..=upper_bound).rev().find(|&row| {
        current_rows[row] == shifted.draft_line
            && ((row + 1)..=composer_row).all(|blank| current_rows[blank].trim().is_empty())
    })?;
    let baseline_prompt = baseline_rows[composer_row].as_str();
    if !baseline_prompt.trim().is_empty()
        && current_rows[(found_row + 1)..]
            .iter()
            .any(|row| row == baseline_prompt)
    {
        return None;
    }
    Some(found_row)
}

fn looks_like_collapsed_paste_row(row: &str) -> bool {
    let mut rest = row;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        if after[..close].to_ascii_lowercase().contains("pasted") {
            return true;
        }
        rest = &after[close + 1..];
    }
    false
}

fn looks_like_prompt_row(row: &str) -> bool {
    let mut tokens = row.split_whitespace();
    let Some(first) = tokens.next() else {
        return false;
    };
    let single_token = tokens.next().is_none();
    let prompt_glyph = !first.chars().any(char::is_alphanumeric);
    prompt_glyph || (single_token && first.ends_with(['>', '›', '❯', '$', '%', '#']))
}

fn row_shares_prompt_prefix(baseline_row: &str, current_row: &str) -> bool {
    let Some(prefix) = baseline_row.split_whitespace().next() else {
        return false;
    };
    looks_like_prompt_row(baseline_row) && current_row.trim_start().starts_with(prefix)
}

fn locate_collapsed_composer_row(baseline_rows: &[String], rows: &[String]) -> Option<usize> {
    if baseline_rows.len() != rows.len() {
        return None;
    }
    for row in (0..rows.len()).rev() {
        if rows[row] == baseline_rows[row]
            || !looks_like_collapsed_paste_row(&rows[row])
            || looks_like_collapsed_paste_row(&baseline_rows[row])
        {
            continue;
        }
        if baseline_rows[row].trim().is_empty() {
            let has_nonempty_below =
                ((row + 1)..rows.len()).any(|below| !baseline_rows[below].trim().is_empty());
            let all_nonempty_below_unchanged = ((row + 1)..rows.len()).all(|below| {
                baseline_rows[below].trim().is_empty() || rows[below] == baseline_rows[below]
            });
            if !has_nonempty_below || !all_nonempty_below_unchanged {
                return Some(row);
            }
        } else if row_shares_prompt_prefix(&baseline_rows[row], &rows[row]) {
            return Some(row);
        }
    }
    None
}

fn collapsed_paste_still_in_composer(
    baseline_rows: &[String],
    current_rows: &[String],
    composer_row: usize,
) -> bool {
    if baseline_rows.len() != current_rows.len() || composer_row >= current_rows.len() {
        return false;
    }
    if !baseline_rows[composer_row].trim().is_empty()
        && current_rows[composer_row] == baseline_rows[composer_row]
    {
        return false;
    }
    if looks_like_collapsed_paste_row(&current_rows[composer_row]) {
        return true;
    }
    if !current_rows[composer_row].trim().is_empty() {
        return false;
    }
    (1..=composer_row).any(|shift| {
        looks_like_collapsed_paste_row(&current_rows[composer_row - shift])
            && ((composer_row - shift + 1)..=composer_row)
                .all(|row| current_rows[row].trim().is_empty())
    })
}

/// Positive evidence that a late-flushed plain-text echo sits in the live composer
/// rather than submitted transcript history replacing non-empty output above an empty prompt.
fn late_plain_echo_in_live_composer(
    baseline_rows: &[String],
    current_rows: &[String],
    body: &str,
) -> bool {
    let last_line = body.lines().last().unwrap_or_default();
    if last_line.is_empty() {
        return false;
    }
    let Some(text_row) = (0..current_rows.len()).rev().find(|&row| {
        current_rows[row] != baseline_rows[row] && current_rows[row].contains(last_line)
    }) else {
        return false;
    };
    let unchanged_prompt_below = ((text_row + 1)..current_rows.len()).any(|below| {
        looks_like_prompt_row(&baseline_rows[below]) && current_rows[below] == baseline_rows[below]
    });
    if unchanged_prompt_below {
        return false;
    }
    if looks_like_prompt_row(&baseline_rows[text_row]) {
        return true;
    }
    if !baseline_rows[text_row].trim().is_empty() {
        return false;
    }
    let has_nonempty_below =
        ((text_row + 1)..current_rows.len()).any(|below| !baseline_rows[below].trim().is_empty());
    !has_nonempty_below
        || ((text_row + 1)..current_rows.len()).any(|below| {
            !baseline_rows[below].trim().is_empty() && current_rows[below] != baseline_rows[below]
        })
}

enum EnterSend {
    Sent(Option<Vec<String>>),
    Declined { recheck: bool },
    Overdue,
}

struct StartedFirstTurnPaste {
    token: Option<u64>,
    baseline: FirstTurnSnapshot,
    input: Arc<input_writer::InputWriter>,
    gate: Arc<DeliveryGate>,
}

struct PrePasteGuard {
    generation: u64,
    input_revision: (u64, u64),
    turn_started: u64,
    title_turn_started: u64,
}

impl SessionManager {
    pub(super) fn delivery_gate_for(&self, session_id: &str) -> Option<Arc<DeliveryGate>> {
        self.session_state(session_id)
            .map(|state| Arc::clone(&state.lock().unwrap().delivery_gate))
    }

    #[cfg(any(windows, test))]
    pub(super) fn queue_pending_first_turn(&self, session_id: &str, body: &str, deadline: Instant) {
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        let turn_started = state.turn_started;
        let title_turn_started = state.title_turn_started;
        if let Some(handle) = state.handle.as_mut() {
            handle.pending_first_turn = Some(
                PendingFirstTurn::new(body.to_owned(), deadline)
                    .with_initial_turns(turn_started, title_turn_started),
            );
            state
                .delivery_gate
                .first_turn_pending
                .store(true, Ordering::Release);
        } else {
            log::warn!("cannot queue first turn for {session_id}: session handle is missing");
        }
    }

    pub(super) fn deliver_pending_first_turn(
        self: &Arc<Self>,
        session_id: &str,
        launch: &Arc<AtomicBool>,
        bytes: &[u8],
    ) {
        let Some(state) = self.session_state(session_id) else {
            return;
        };
        let gate = state.lock().unwrap().delivery_gate.clone();
        let (pending, expects_turn_signals, timed_out, pre_paste, stop) = {
            let delivery = gate.state.lock().unwrap();
            let mut state = state.lock().unwrap();
            if state.killed || launch.load(Ordering::Acquire) {
                return;
            }
            let needs_you = state.model.status().observation.needs_you();
            let has_active_draft = state.model.has_active_draft();
            let turn_started = state.turn_started;
            let title_turn_started = state.title_turn_started;
            let reset_guard = state
                .terminal
                .as_ref()
                .map(|terminal| terminal.input_reset_guard())
                .unwrap_or(0);
            let Some(handle) = state
                .handle
                .as_mut()
                .filter(|handle| Arc::ptr_eq(launch, &handle.stop))
            else {
                return;
            };
            let Some(pending) = handle.pending_first_turn.as_mut() else {
                gate.first_turn_pending.store(false, Ordering::Release);
                return;
            };
            pending.observe(bytes);
            let decision = pending.decision(
                handle.startup.as_ref(),
                needs_you,
                super::super::clock::now(),
            );
            if decision == ReadinessDecision::Wait {
                return;
            }
            let human_owned = pending.human_intervened()
                || has_active_draft
                || turn_started != pending.initial_turn_started
                || title_turn_started != pending.initial_title_turn_started;
            gate.first_turn_pending.store(false, Ordering::Release);
            let pending = handle.pending_first_turn.take().unwrap();
            if human_owned {
                log::info!(
                    "skipping automatic first turn for {session_id}: human draft or turn already active"
                );
                return;
            }
            let expects_turn_signals = handle
                .startup
                .as_ref()
                .is_some_and(|startup| startup.waits_for_input_readiness());
            let pre_paste = PrePasteGuard {
                generation: delivery.generation,
                input_revision: (delivery.next_ticket, reset_guard),
                turn_started,
                title_turn_started,
            };
            (
                pending,
                expects_turn_signals,
                decision == ReadinessDecision::TimedOut,
                pre_paste,
                Arc::clone(&handle.stop),
            )
        };
        if timed_out {
            self.warn_first_turn(format!(
                "no startup readiness signal for {session_id}; delivering first turn after timeout"
            ));
        }
        let payload = if pending.bracketed() {
            format!("\x1b[200~{}\x1b[201~", pending.body)
        } else {
            pending.body.clone()
        };
        let body = pending.body;
        let manager = Arc::clone(self);
        let session_id_owned = session_id.to_owned();
        let stop_for_worker = Arc::clone(&stop);
        // The paste write and Enter confirmation loop must not stall terminal output
        // or hold the session lock.
        let worker = thread::Builder::new()
            .name(format!("first-turn-{session_id}"))
            .spawn(move || {
                manager.run_first_turn_delivery(
                    &session_id_owned,
                    &body,
                    payload.as_bytes(),
                    expects_turn_signals,
                    pre_paste,
                    &stop_for_worker,
                );
            })
            .expect("spawn first-turn delivery worker");
        let stale_worker = {
            let mut state = state.lock().unwrap();
            if let Some(handle) = state
                .handle
                .as_mut()
                .filter(|handle| Arc::ptr_eq(&stop, &handle.stop))
            {
                handle.first_turn_worker.replace(worker)
            } else {
                Some(worker)
            }
        };
        if let Some(worker) = stale_worker {
            let _ = worker.join();
        }
    }

    fn run_first_turn_delivery(
        &self,
        session_id: &str,
        body: &str,
        payload: &[u8],
        expects_turn_signals: bool,
        pre_paste: PrePasteGuard,
        stop: &Arc<AtomicBool>,
    ) {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let timings = self.effective_first_turn_timings();
        let StartedFirstTurnPaste {
            mut token,
            baseline,
            input,
            gate,
        } = match self.begin_first_turn_paste(
            session_id,
            body,
            payload,
            timings.submit_timeout,
            pre_paste,
            stop,
        ) {
            Ok(Some(started)) => started,
            Ok(None) => return,
            Err(error) => {
                self.warn_first_turn(format!(
                    "first-turn delivery failed for {session_id}: {error}"
                ));
                return;
            }
        };
        let mut submit = FirstTurnSubmit::new(
            body.to_owned(),
            baseline,
            expects_turn_signals,
            timings,
            super::super::clock::now(),
        );
        let generation = submit.baseline().delivery.generation;
        let mut rechecked = false;
        loop {
            if stop.load(Ordering::Acquire) {
                if let Some(token) = token.take() {
                    self.finish_delivery(session_id, token);
                }
                return;
            }
            let now = super::super::clock::now();
            let (snapshot, wake_seq) = match self.first_turn_snapshot_with_wake(session_id) {
                Some((snapshot, wake_seq)) => (Some(snapshot), wake_seq),
                None => (None, 0),
            };
            match submit.step(snapshot.as_ref(), now) {
                FirstTurnStep::Wait(until) => {
                    rechecked = false;
                    if token.is_some()
                        && snapshot.as_ref().is_some_and(|snapshot| snapshot.needs_you)
                    {
                        if let Some(token) = token.take() {
                            self.finish_delivery(session_id, token);
                        }
                    }
                    self.wait_for_first_turn_step(
                        &gate,
                        generation,
                        submit.baseline().delivery.input_revision.0,
                        wake_seq,
                        stop,
                        until,
                    );
                }
                FirstTurnStep::SendFirstEnter => {
                    let result = match token.take() {
                        Some(token) => {
                            self.send_first_turn_enter(session_id, token, &input, submit.baseline())
                        }
                        None => self.resend_first_turn_enter(session_id, &input, &submit, None),
                    };
                    match result {
                        Ok(EnterSend::Sent(sent_rows)) => {
                            rechecked = false;
                            submit.mark_enter_sent(super::super::clock::now(), sent_rows);
                        }
                        Ok(EnterSend::Declined { .. }) => {
                            rechecked = false;
                            submit.defer_retry(super::super::clock::now());
                        }
                        Ok(EnterSend::Overdue) => return,
                        Err(error) => {
                            self.warn_first_turn(format!(
                                "first-turn Enter failed for {session_id}: {error}"
                            ));
                            return;
                        }
                    }
                }
                FirstTurnStep::ResendEnter(reason) => {
                    if let Some(token) = token.take() {
                        self.finish_delivery(session_id, token);
                    }
                    match self.resend_first_turn_enter(session_id, &input, &submit, Some(reason)) {
                        Ok(EnterSend::Sent(sent_rows)) => {
                            rechecked = false;
                            log::info!(
                                "first-turn Enter retried session={session_id} attempt={}",
                                submit.attempts() + 1
                            );
                            submit.mark_enter_sent(super::super::clock::now(), sent_rows);
                        }
                        Ok(EnterSend::Declined { recheck }) => {
                            if recheck && !rechecked {
                                rechecked = true;
                                continue;
                            }
                            rechecked = false;
                            submit.defer_retry(super::super::clock::now());
                        }
                        Ok(EnterSend::Overdue) => return,
                        Err(error) => {
                            self.warn_first_turn(format!(
                                "first-turn retry Enter failed for {session_id}: {error}"
                            ));
                            return;
                        }
                    }
                }
                FirstTurnStep::Finish(outcome) => {
                    if let Some(token) = token.take() {
                        self.finish_delivery(session_id, token);
                    }
                    match outcome {
                        FirstTurnOutcome::Confirmed => {
                            self.confirm_delivery(session_id, &submit.baseline().delivery);
                            log::info!(
                                "first-turn delivery confirmed session={session_id} attempts={}",
                                submit.attempts()
                            );
                        }
                        FirstTurnOutcome::Unverified => {
                            self.confirm_delivery(session_id, &submit.baseline().delivery);
                            log::info!(
                                "first-turn delivery unverified session={session_id} attempts={}",
                                submit.attempts()
                            );
                        }
                        FirstTurnOutcome::TimedOut { attempts } => {
                            self.warn_first_turn(format!(
                                "first-turn submit timed out for {session_id} after {attempts} Enter attempt(s)"
                            ));
                        }
                        FirstTurnOutcome::SessionEnded | FirstTurnOutcome::HumanInput => {}
                    }
                    return;
                }
            }
        }
    }

    fn effective_first_turn_timings(&self) -> FirstTurnTimings {
        #[cfg(test)]
        if let Some(timings) = *self.first_turn_timings.lock().unwrap() {
            return timings;
        }
        FirstTurnTimings::WINDOWS_DEFAULT
    }

    fn warn_first_turn(&self, message: String) {
        log::warn!("{message}");
        #[cfg(test)]
        self.first_turn_warnings.lock().unwrap().push(message);
    }

    pub(super) fn first_turn_snapshot_with_wake(
        &self,
        session_id: &str,
    ) -> Option<(FirstTurnSnapshot, u64)> {
        let session = self.session_state(session_id)?;
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.handle.as_ref()?;
        Some((
            session.first_turn_snapshot_locked(&delivery),
            delivery.first_turn_wake,
        ))
    }

    fn begin_first_turn_paste(
        &self,
        session_id: &str,
        body: &str,
        payload: &[u8],
        wait_timeout: Duration,
        pre_paste: PrePasteGuard,
        stop: &Arc<AtomicBool>,
    ) -> Result<Option<StartedFirstTurnPaste>> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(None);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let wait_deadline = super::super::clock::now() + wait_timeout;
        let (token, baseline, input, write) = loop {
            let mut delivery = gate.state.lock().unwrap();
            let mut state = session.lock().unwrap();
            if state.killed || stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            let current_reset_guard = state
                .terminal
                .as_ref()
                .map(|terminal| terminal.input_reset_guard())
                .unwrap_or(0);
            if delivery.generation != pre_paste.generation
                || delivery.next_ticket != pre_paste.input_revision.0
                || current_reset_guard != pre_paste.input_revision.1
                || state.turn_started != pre_paste.turn_started
                || state.title_turn_started != pre_paste.title_turn_started
                || state.model.has_active_draft()
            {
                log::info!(
                    "skipping automatic first turn paste for {session_id}: intervening human input or active turn"
                );
                return Ok(None);
            }
            let Some(handle) = state
                .handle
                .as_ref()
                .filter(|handle| Arc::ptr_eq(stop, &handle.stop))
            else {
                return Ok(None);
            };
            if delivery.in_flight
                || delivery.next_ticket != delivery.next_served
                || state.model.status().observation.needs_you()
            {
                drop(state);
                let remaining = wait_deadline.saturating_duration_since(super::super::clock::now());
                if remaining.is_zero() {
                    drop(delivery);
                    self.warn_first_turn(format!(
                        "first-turn paste timed out waiting for session readiness for {session_id}"
                    ));
                    return Ok(None);
                }
                let _ = gate.ready.wait_timeout(delivery, remaining).unwrap();
                continue;
            }
            let baseline = state.first_turn_snapshot_locked(&delivery);
            let token = delivery.generation;
            let terminal = state.terminal.clone();
            let input = self.input_writer(handle);
            delivery.in_flight = true;
            state.delivery_line = Some(body.to_owned());
            state.delivery_prompted = false;
            drop(state);
            if let Some(terminal) = terminal {
                terminal.track_delivery_input(body);
            }
            let write = input.push(payload);
            drop(delivery);
            break (token, baseline, input, write);
        };
        let write = match write {
            Ok(write) => write,
            Err(error) => {
                self.finish_delivery(session_id, token);
                return Err(error);
            }
        };
        match write.wait() {
            Ok(Written::Done) => Ok(Some(StartedFirstTurnPaste {
                token: Some(token),
                baseline,
                input,
                gate,
            })),
            Ok(Written::Overdue(message)) => {
                self.report_overdue(session_id, Written::Overdue(message));
                self.finish_delivery(session_id, token);
                Ok(None)
            }
            Err(error) => {
                self.finish_delivery(session_id, token);
                Err(error)
            }
        }
    }

    fn send_first_turn_enter(
        &self,
        session_id: &str,
        token: u64,
        input: &Arc<input_writer::InputWriter>,
        baseline: &FirstTurnSnapshot,
    ) -> Result<EnterSend> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(EnterSend::Declined { recheck: false });
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let mut delivery = gate.state.lock().unwrap();
        let mut state = session.lock().unwrap();
        let current_reset_guard = state
            .terminal
            .as_ref()
            .map(|terminal| terminal.input_reset_guard())
            .unwrap_or(0);
        let can_send = delivery.in_flight
            && delivery.generation == token
            && delivery.next_ticket == baseline.delivery.input_revision.0
            && current_reset_guard == baseline.delivery.input_revision.1
            && !state.killed
            && state.handle.is_some()
            && !state.model.status().observation.needs_you()
            && state.turn_started == baseline.delivery.turn_started
            && state.title_turn_started == baseline.title_turn_started;
        if !can_send {
            let released = delivery.in_flight && delivery.generation == token;
            if released {
                delivery.in_flight = false;
                gate.ready.notify_all();
            }
            drop(state);
            drop(delivery);
            if released {
                self.notify_delivery_event(
                    session_id,
                    router::SessionDeliveryEvent::DeliveryFinished,
                );
            }
            return Ok(EnterSend::Declined { recheck: false });
        }
        let previous = state.model.input_checkpoint();
        state.model.apply(
            SessionEvent::ArmCompletion,
            super::super::clock::state_now(),
        );
        let armed = state.model.input_checkpoint();
        // Hold `state` and `delivery` until the Enter is queued so turn/status events
        // cannot race between checking invariants and enqueueing.
        let write = input.push(b"\r");
        let sent_rows = if write.is_ok() {
            if let Some(terminal) = state.terminal.as_ref() {
                terminal.track_delivery_submit();
                Some(terminal.input_rows())
            } else {
                None
            }
        } else {
            None
        };
        delivery.in_flight = false;
        gate.ready.notify_all();
        drop(state);
        drop(delivery);
        self.notify_delivery_event(session_id, router::SessionDeliveryEvent::DeliveryFinished);
        let write = match write {
            Ok(write) => write,
            Err(error) => {
                Self::roll_back_input(&session, previous, armed);
                return Err(error);
            }
        };
        match write.wait() {
            Ok(Written::Done) => {
                self.capture_codex_session_key(session_id);
                Ok(EnterSend::Sent(sent_rows))
            }
            Ok(Written::Overdue(message)) => {
                self.report_overdue(session_id, Written::Overdue(message));
                self.capture_codex_session_key(session_id);
                Ok(EnterSend::Overdue)
            }
            Err(error) => {
                Self::roll_back_input(&session, previous, armed);
                Err(error)
            }
        }
    }

    fn resend_first_turn_enter(
        &self,
        session_id: &str,
        input: &Arc<input_writer::InputWriter>,
        submit: &FirstTurnSubmit,
        reason: Option<RetryReason>,
    ) -> Result<EnterSend> {
        #[cfg(test)]
        if reason.is_some() {
            if let Some(hook) = self.before_first_turn_retry.lock().unwrap().take() {
                hook();
            }
        }
        let baseline = submit.baseline();
        let arm_completion = reason.is_none();
        let Some(session) = self.session_state(session_id) else {
            return Ok(EnterSend::Declined { recheck: false });
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let mut state = session.lock().unwrap();
        let current_reset_guard = state
            .terminal
            .as_ref()
            .map(|terminal| terminal.input_reset_guard())
            .unwrap_or(0);
        if delivery.in_flight
            || delivery.generation != baseline.delivery.generation
            || delivery.next_ticket != baseline.delivery.input_revision.0
            || delivery.next_ticket != delivery.next_served
            || current_reset_guard != baseline.delivery.input_revision.1
            || state.killed
            || state.handle.is_none()
            || state.model.status().observation.needs_you()
            || state.turn_started != baseline.delivery.turn_started
            || state.title_turn_started != baseline.title_turn_started
        {
            return Ok(EnterSend::Declined { recheck: false });
        }
        let (write, sent_rows, armed) = match reason {
            Some(RetryReason::ProbeOrTracker) => {
                let Some(terminal) = state.terminal.as_ref() else {
                    return Ok(EnterSend::Declined { recheck: false });
                };
                let mut writer_busy = false;
                // Serialize revision, live probe/tracker checks, and idle enqueueing while
                // still holding `state` and `delivery` so neither PTY clears nor turn events race.
                let write = terminal.with_unchanged_input(
                    baseline.delivery.input_revision.1,
                    submit.probe(),
                    || match input.push_when_idle(b"\r") {
                        Ok(Some(write)) => Some(Ok(write)),
                        Ok(None) => {
                            writer_busy = true;
                            None
                        }
                        Err(error) => Some(Err(error)),
                    },
                );
                match write {
                    Some(Ok(write)) => (write, Some(terminal.input_rows()), None),
                    Some(Err(error)) => return Err(error),
                    None => {
                        return Ok(EnterSend::Declined {
                            recheck: !writer_busy,
                        });
                    }
                }
            }
            Some(RetryReason::PasteEvidence) => {
                let Some(terminal) = state.terminal.as_ref() else {
                    return Ok(EnterSend::Declined { recheck: false });
                };
                let live_rows = terminal.input_rows();
                if !submit.still_unsubmitted_paste(&live_rows) {
                    return Ok(EnterSend::Declined { recheck: true });
                }
                let write = match input.push_when_idle(b"\r") {
                    Ok(Some(write)) => write,
                    Ok(None) => return Ok(EnterSend::Declined { recheck: false }),
                    Err(error) => return Err(error),
                };
                terminal.track_delivery_submit();
                (write, Some(live_rows), None)
            }
            None => {
                let previous = state.model.input_checkpoint();
                state.model.apply(
                    SessionEvent::ArmCompletion,
                    super::super::clock::state_now(),
                );
                let armed = (previous, state.model.input_checkpoint());
                let write = match input.push_when_idle(b"\r") {
                    Ok(Some(write)) => write,
                    Ok(None) => {
                        drop(state);
                        drop(delivery);
                        Self::roll_back_input(&session, armed.0, armed.1);
                        return Ok(EnterSend::Declined { recheck: false });
                    }
                    Err(error) => {
                        drop(state);
                        drop(delivery);
                        Self::roll_back_input(&session, armed.0, armed.1);
                        return Err(error);
                    }
                };
                let sent_rows = state.terminal.as_ref().map(|terminal| {
                    terminal.track_delivery_submit();
                    terminal.input_rows()
                });
                (write, sent_rows, Some(armed))
            }
        };
        drop(state);
        drop(delivery);
        match write.wait() {
            Ok(Written::Done) => {
                if arm_completion {
                    self.capture_codex_session_key(session_id);
                }
                Ok(EnterSend::Sent(sent_rows))
            }
            Ok(Written::Overdue(message)) => {
                self.report_overdue(session_id, Written::Overdue(message));
                if arm_completion {
                    self.capture_codex_session_key(session_id);
                }
                Ok(EnterSend::Overdue)
            }
            Err(error) => {
                if let Some((previous, after)) = armed {
                    Self::roll_back_input(&session, previous, after);
                }
                Err(error)
            }
        }
    }

    fn wait_for_first_turn_step(
        &self,
        gate: &Arc<DeliveryGate>,
        generation: u64,
        baseline_ticket: u64,
        wake_seq: u64,
        stop: &Arc<AtomicBool>,
        until: Instant,
    ) {
        let delivery = gate.state.lock().unwrap();
        if delivery.generation != generation
            || delivery.next_ticket != baseline_ticket
            || delivery.first_turn_wake != wake_seq
            || stop.load(Ordering::Acquire)
        {
            return;
        }
        let remaining = until.saturating_duration_since(super::super::clock::now());
        if !remaining.is_zero() {
            let _ = gate.ready.wait_timeout(delivery, remaining).unwrap();
        }
    }

    #[cfg(test)]
    pub(super) fn set_first_turn_timings(&self, timings: FirstTurnTimings) {
        *self.first_turn_timings.lock().unwrap() = Some(timings);
    }

    #[cfg(test)]
    pub(super) fn first_turn_warnings(&self) -> Vec<String> {
        self.first_turn_warnings.lock().unwrap().clone()
    }

    #[cfg(test)]
    pub(super) fn set_before_first_turn_retry(&self, hook: impl FnOnce() + Send + 'static) {
        *self.before_first_turn_retry.lock().unwrap() = Some(Box::new(hook));
    }

    #[cfg(test)]
    pub(super) fn wait_for_first_turn_worker(&self, session_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let worker = loop {
            let worker = self.session_state(session_id).and_then(|state| {
                state
                    .lock()
                    .unwrap()
                    .handle
                    .as_mut()
                    .and_then(|handle| handle.first_turn_worker.take())
            });
            if let Some(worker) = worker {
                break worker;
            }
            assert!(
                Instant::now() < deadline,
                "first-turn worker never started for {session_id}"
            );
            thread::sleep(Duration::from_millis(5));
        };
        worker.join().expect("first-turn worker panicked");
    }
}
