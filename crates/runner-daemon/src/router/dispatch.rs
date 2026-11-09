use std::collections::HashMap;
use std::sync::{mpsc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use runner_core::model::{EventDraft, SignalType};
use runner_terminal::delivery_probe::{InputProbe, ProbeState};

use super::{
    DeliveryBlockedEvent, DeliveryReadiness, DeliveryReservation, DeliverySnapshot, Router,
    SessionDeliveryEvent, SUBMIT_DELAY,
};

const REDELIVERY_INTERVAL: Duration = Duration::from_secs(30);
const REDELIVERY_BACKOFF: Duration = Duration::from_secs(120);
pub(crate) const CONFIRM_WINDOW: Duration = Duration::from_millis(1500);
const INPUT_CLEAR_GRACE: Duration = Duration::from_millis(500);

pub(crate) enum Input {
    Backlog {
        handle: String,
        initial_backlog: bool,
    },
    Session {
        session_id: String,
        event: SessionDeliveryEvent,
    },
    RosterChanged,
    Stop,
}

struct Attempt {
    token: u64,
    deadline: Instant,
    snapshot: Option<DeliverySnapshot>,
    probe: InputProbe,
    submitted: bool,
    second_enter: bool,
}

/// A notice whose attempt ended without confirmation and may still sit in the composer;
/// `seen` once the probe has found it there, `tracked` once the input tracker has.
struct Unsent {
    snapshot: DeliverySnapshot,
    probe: InputProbe,
    seen: bool,
    tracked: bool,
}

struct Hold {
    reason: DeliveryReadiness,
    since: Instant,
    warned: bool,
}

struct Subscriber {
    session_id: String,
    wanted: bool,
    attempt: Option<Attempt>,
    retry: Option<Instant>,
    last_attempt: Option<Instant>,
    warned: bool,
    blocked: Option<(usize, String)>,
    unsent: Option<Unsent>,
    pause_check: Option<Instant>,
    input_cleared: bool,
    nudged: bool,
    hold: Option<Hold>,
}

pub(crate) struct Dispatcher {
    subscribers: HashMap<String, Subscriber>,
    redelivery: Instant,
    stopped: bool,
}

impl Dispatcher {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            subscribers: HashMap::new(),
            redelivery: now + REDELIVERY_INTERVAL,
            stopped: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_reason(&self, handle: &str) -> Option<DeliveryReadiness> {
        self.subscribers
            .get(handle)?
            .hold
            .as_ref()
            .map(|hold| hold.reason)
    }

    /// All timing is driven by the caller's clock. Port calls never hold router state.
    pub(crate) fn advance(
        &mut self,
        router: &Router,
        input: Option<Input>,
        now: Instant,
    ) -> Option<Instant> {
        let (sessions, backlogs) = {
            let state = router.state.lock().unwrap();
            (
                state.session_by_handle.clone(),
                state.unread_by_handle.clone(),
            )
        };
        self.subscribers.retain(|handle, subscriber| {
            if sessions.get(handle) == Some(&subscriber.session_id) {
                return true;
            }
            Self::clear(router, handle, subscriber);
            false
        });
        for (handle, session_id) in sessions {
            self.subscribers
                .entry(handle.clone())
                .or_insert_with(|| Subscriber {
                    session_id,
                    // Wait for the backlog input so initial replay can seed the warning state.
                    wanted: false,
                    attempt: None,
                    retry: None,
                    last_attempt: None,
                    warned: false,
                    blocked: None,
                    unsent: None,
                    pause_check: None,
                    input_cleared: false,
                    nudged: false,
                    hold: None,
                });
        }
        let mut reconcile = now >= self.redelivery;
        if reconcile {
            self.redelivery = now + REDELIVERY_INTERVAL;
        }
        match input {
            Some(Input::Stop) => {
                self.stopped = true;
                for (handle, subscriber) in &mut self.subscribers {
                    Self::clear(router, handle, subscriber);
                }
                return None;
            }
            Some(Input::Backlog {
                handle,
                initial_backlog,
            }) => {
                if let Some(subscriber) = self.subscribers.get_mut(&handle) {
                    subscriber.wanted = backlogs.get(&handle).copied().unwrap_or(0) > 0;
                    if initial_backlog
                        && subscriber.wanted
                        && router.injector.delivery_readiness(&subscriber.session_id)
                            == DeliveryReadiness::Unavailable
                    {
                        subscriber.warned = true;
                    }
                }
            }
            Some(Input::Session { session_id, event }) => {
                for subscriber in self
                    .subscribers
                    .values_mut()
                    .filter(|s| s.session_id == session_id)
                {
                    match event {
                        SessionDeliveryEvent::Idle => {
                            reconcile = true;
                            subscriber.nudged = true;
                        }
                        // Our own ignored Enter redraws the composer, which silence detection
                        // reports as a turn that ends two seconds later.
                        SessionDeliveryEvent::SilenceIdle => {
                            reconcile = true;
                            subscriber.nudged |= subscriber.last_attempt.is_none_or(|last| {
                                now.saturating_duration_since(last) >= REDELIVERY_INTERVAL
                            });
                        }
                        SessionDeliveryEvent::InputCleared => {
                            subscriber.input_cleared = true;
                            subscriber.retry = Some(now + INPUT_CLEAR_GRACE)
                        }
                        SessionDeliveryEvent::Exited | SessionDeliveryEvent::Respawned => {
                            if let Some(attempt) = subscriber.attempt.take() {
                                log::info!("router attempt outcome=Unverified mission={} session={session_id} runtime={} reason={event:?}", router.mission_id, attempt.snapshot.as_ref().map(|s| s.runtime.as_str()).unwrap_or("unknown"));
                                router.injector.finish_delivery(&session_id, attempt.token);
                            }
                            subscriber.unsent = None;
                            subscriber.pause_check = None;
                            subscriber.input_cleared = false;
                            subscriber.nudged = false;
                            subscriber.wanted = true;
                            subscriber.retry = None;
                            subscriber.warned = false;
                        }
                        SessionDeliveryEvent::ReadinessChanged => {
                            subscriber.retry = Some(now + INPUT_CLEAR_GRACE);
                            subscriber.nudged = true;
                        }
                        SessionDeliveryEvent::StartupReady
                        | SessionDeliveryEvent::InputQueueDrained => subscriber.retry = None,
                        SessionDeliveryEvent::DeliveryFinished => {}
                    }
                }
            }
            _ => {}
        }
        if self.stopped {
            return None;
        }
        for (handle, subscriber) in &mut self.subscribers {
            let unread = backlogs.get(handle).copied().unwrap_or(0);
            if subscriber.retry.is_some_and(|deadline| now >= deadline) {
                subscriber.retry = None;
            }
            if let Some(mut attempt) = subscriber.attempt.take() {
                if now < attempt.deadline {
                    subscriber.attempt = Some(attempt);
                } else if !attempt.submitted {
                    if let Some(current) = router.injector.delivery_snapshot(&subscriber.session_id)
                    {
                        attempt.probe.locate(&current.rows);
                    }
                    let submitted = router.injector.inject_reserved(
                        &subscriber.session_id,
                        attempt.token,
                        b"\r",
                    );
                    router
                        .injector
                        .finish_delivery(&subscriber.session_id, attempt.token);
                    attempt.submitted = true;
                    attempt.deadline = now + CONFIRM_WINDOW;
                    if let Err(error) = submitted {
                        log::warn!(
                            "notification Enter failed session={}: {error}",
                            subscriber.session_id
                        );
                    }
                    subscriber.attempt = Some(attempt);
                } else {
                    let current = router.injector.delivery_snapshot(&subscriber.session_id);
                    let unchanged = attempt.snapshot.as_ref().zip(current.as_ref()).is_some_and(
                        |(before, current)| {
                            before.generation == current.generation
                                && before.input_revision == current.input_revision
                        },
                    );
                    let state = current
                        .as_ref()
                        .map(|current| attempt.probe.state(&current.rows))
                        .unwrap_or(ProbeState::Unknown);
                    // A moved composer reads as cleared to the probe while the tracker still holds our text.
                    let unsent = current.as_ref().is_some_and(|current| current.unsent);
                    let turn_started = attempt.snapshot.as_ref().zip(current.as_ref()).is_some_and(
                        |(before, current)| {
                            before.generation == current.generation
                                && current.turn_started != before.turn_started
                        },
                    );
                    let prompted =
                        turn_started && current.as_ref().is_some_and(|current| current.prompted);
                    let outcome = if prompted {
                        "Confirmed"
                    } else if !unchanged {
                        "Unverified"
                    } else if turn_started || (state == ProbeState::Cleared && !unsent) {
                        "Confirmed"
                    } else if (state == ProbeState::Present || unsent) && !attempt.second_enter {
                        let retried = attempt.snapshot.as_ref().is_some_and(|snapshot| {
                            matches!(
                                router.injector.resubmit(
                                    &subscriber.session_id,
                                    snapshot,
                                    &attempt.probe
                                ),
                                Ok(true)
                            )
                        });
                        if retried {
                            attempt.second_enter = true;
                            attempt.deadline = now + CONFIRM_WINDOW;
                            subscriber.attempt = Some(attempt);
                            continue;
                        }
                        let still_unchanged = attempt
                            .snapshot
                            .as_ref()
                            .zip(
                                router
                                    .injector
                                    .delivery_snapshot(&subscriber.session_id)
                                    .as_ref(),
                            )
                            .is_some_and(|(before, current)| {
                                before.generation == current.generation
                                    && before.input_revision == current.input_revision
                                    && (current.unsent
                                        || attempt.probe.state(&current.rows)
                                            == ProbeState::Present)
                            });
                        if still_unchanged {
                            "Unsubmitted"
                        } else {
                            "Unverified"
                        }
                    } else if state == ProbeState::Present || unsent {
                        "Unsubmitted"
                    } else {
                        "Unverified"
                    };
                    let runtime = current
                        .as_ref()
                        .or(attempt.snapshot.as_ref())
                        .map(|snapshot| snapshot.runtime.as_str())
                        .unwrap_or("unknown");
                    log::info!("router attempt outcome={outcome} mission={} session={} handle=@{handle} runtime={runtime}", router.mission_id, subscriber.session_id);
                    if outcome == "Confirmed" {
                        if let Some(snapshot) = &attempt.snapshot {
                            router
                                .injector
                                .confirm_delivery(&subscriber.session_id, snapshot);
                        }
                        let draft = EventDraft::signal(
                            router.crew_id.as_deref(),
                            &router.mission_id,
                            handle,
                            SignalType::new("session_status"),
                            serde_json::json!({"state":"busy"}),
                        );
                        if let Err(error) = router
                            .injector
                            .synthesize_wake_busy(&subscriber.session_id, draft)
                        {
                            log::warn!("publish confirmed delivery busy: {error}");
                        }
                        Self::hold(router, handle, subscriber, None, unread, now);
                        Self::blocked(router, handle, subscriber, unread, None);
                    } else if let Some(snapshot) = attempt.snapshot {
                        subscriber.unsent = Some(Unsent {
                            snapshot,
                            probe: attempt.probe,
                            seen: false,
                            tracked: unsent || current.as_ref().is_some_and(|current| current.held),
                        });
                        subscriber.pause_check = Some(now + CONFIRM_WINDOW);
                        subscriber.nudged = false;
                    }
                    subscriber.retry = Some(now + SUBMIT_DELAY);
                }
            }
            let backoff_passed = subscriber
                .last_attempt
                .is_none_or(|last| now.saturating_duration_since(last) >= REDELIVERY_BACKOFF);
            if let Some(mut unsent) = subscriber.unsent.take() {
                let current = router.injector.delivery_snapshot(&subscriber.session_id);
                let own = current.as_ref().is_some_and(|current| {
                    current.generation == unsent.snapshot.generation
                        && current.input_revision == unsent.snapshot.input_revision
                });
                let state = current
                    .as_ref()
                    .map(|current| unsent.probe.state(&current.rows))
                    .unwrap_or(ProbeState::Unknown);
                let tracked = current.as_ref().is_some_and(|current| current.unsent);
                let started = current.as_ref().is_some_and(|current| {
                    current.generation == unsent.snapshot.generation
                        && current.turn_started != unsent.snapshot.turn_started
                });
                if state == ProbeState::Present {
                    unsent.seen = true;
                    subscriber.input_cleared = false;
                }
                unsent.tracked |= tracked || current.as_ref().is_some_and(|current| current.held);
                let readiness = router.injector.delivery_readiness(&subscriber.session_id);
                // As for reports, a turn that reported the notice as its prompt, or whose notice
                // the tracker let go of, took it (#1225).
                let prompted = current.as_ref().is_some_and(|current| current.prompted);
                let gone = (started && (prompted || (unsent.tracked && !tracked)))
                    || (state == ProbeState::Cleared && !tracked)
                    || (state == ProbeState::Unknown
                        && unsent.seen
                        && subscriber.input_cleared
                        && matches!(
                            readiness,
                            DeliveryReadiness::Ready { .. }
                                | DeliveryReadiness::Working
                                | DeliveryReadiness::Unconfirmed
                        ));
                if !gone && (unsent.seen || tracked) {
                    // Typing again would stack a second notice on this one (#753).
                    let opportunity = subscriber.retry.is_none()
                        && (std::mem::take(&mut subscriber.nudged)
                            || (reconcile && backoff_passed));
                    if opportunity
                        && own
                        && unread > 0
                        && matches!(
                            readiness,
                            DeliveryReadiness::Ready { .. }
                                | DeliveryReadiness::Working
                                | DeliveryReadiness::Unconfirmed
                                | DeliveryReadiness::Unsent
                                | DeliveryReadiness::Drafting
                        )
                    {
                        if let Some(current) = current {
                            match router.injector.resubmit(
                                &subscriber.session_id,
                                &current,
                                &unsent.probe,
                            ) {
                                Ok(true) => {
                                    log::info!("router resubmitted unsent notice mission={} session={} handle=@{handle}", router.mission_id, subscriber.session_id);
                                    subscriber.last_attempt = Some(now);
                                    subscriber.pause_check = None;
                                    subscriber.attempt = Some(Attempt {
                                        token: current.generation,
                                        deadline: now + CONFIRM_WINDOW,
                                        snapshot: Some(current),
                                        probe: unsent.probe,
                                        submitted: true,
                                        second_enter: true,
                                    });
                                    continue;
                                }
                                Ok(false) => {}
                                Err(error) => log::warn!(
                                    "resubmit unsent notice failed session={}: {error}",
                                    subscriber.session_id
                                ),
                            }
                        }
                    }
                    subscriber.unsent = Some(unsent);
                    subscriber.pause_check = Some(now + CONFIRM_WINDOW);
                    Self::hold(
                        router,
                        handle,
                        subscriber,
                        (unread > 0).then_some(DeliveryReadiness::Unsent),
                        unread,
                        now,
                    );
                    Self::blocked(router, handle, subscriber, unread, Some("unsubmitted"));
                    continue;
                }
                if !gone {
                    subscriber.unsent = Some(unsent);
                }
                subscriber.pause_check = None;
                if subscriber
                    .hold
                    .as_ref()
                    .is_some_and(|hold| hold.reason == DeliveryReadiness::Unsent)
                {
                    Self::hold(router, handle, subscriber, None, unread, now);
                }
                if subscriber
                    .blocked
                    .as_ref()
                    .is_some_and(|(_, reason)| reason == "unsubmitted")
                {
                    Self::blocked(router, handle, subscriber, unread, None);
                }
            }
            subscriber.nudged = false;
            if unread == 0 {
                subscriber.wanted = false;
                Self::hold(router, handle, subscriber, None, 0, now);
                Self::blocked(router, handle, subscriber, 0, None);
                continue;
            }
            if reconcile && subscriber.attempt.is_none() && !subscriber.wanted && backoff_passed {
                match Self::readiness(router, subscriber, unread) {
                    DeliveryReadiness::Ready { idle } => {
                        subscriber.wanted = idle;
                        Self::hold(
                            router,
                            handle,
                            subscriber,
                            (!idle).then_some(DeliveryReadiness::Ready { idle }),
                            unread,
                            now,
                        );
                        Self::blocked(router, handle, subscriber, unread, None);
                    }
                    DeliveryReadiness::Working => {
                        subscriber.wanted = false;
                        Self::hold(router, handle, subscriber, None, unread, now);
                        Self::blocked(router, handle, subscriber, unread, None);
                    }
                    DeliveryReadiness::Unavailable => {}
                    readiness => {
                        Self::hold(router, handle, subscriber, Some(readiness), unread, now);
                        Self::blocked(
                            router,
                            handle,
                            subscriber,
                            unread,
                            blocked_reason(readiness),
                        );
                    }
                }
            }
            if !subscriber.wanted
                || subscriber.attempt.is_some()
                || subscriber.retry.is_some_and(|deadline| now < deadline)
            {
                continue;
            }
            subscriber.retry = None;
            let readiness = Self::readiness(router, subscriber, unread);
            Self::hold(
                router,
                handle,
                subscriber,
                (!matches!(
                    readiness,
                    DeliveryReadiness::Ready { idle: true } | DeliveryReadiness::Working
                ))
                .then_some(readiness),
                unread,
                now,
            );
            Self::blocked(
                router,
                handle,
                subscriber,
                unread,
                blocked_reason(readiness),
            );
            match readiness {
                DeliveryReadiness::Ready { idle: true } | DeliveryReadiness::Working => {
                    match router.injector.reserve_delivery(&subscriber.session_id) {
                        Ok(DeliveryReservation::Ready(token)) => {
                            let unread = router
                                .state
                                .lock()
                                .unwrap()
                                .unread_by_handle
                                .get(handle)
                                .copied()
                                .unwrap_or(0);
                            if unread == 0 {
                                router
                                    .injector
                                    .finish_delivery(&subscriber.session_id, token);
                                subscriber.wanted = false;
                                Self::blocked(router, handle, subscriber, 0, None);
                                continue;
                            }
                            let snapshot =
                                router.injector.delivery_snapshot(&subscriber.session_id);
                            let body = format!(
                                "[inbox] {unread} new messages — run `runner msg read` to view."
                            );
                            match router.injector.inject_reserved(
                                &subscriber.session_id,
                                token,
                                body.as_bytes(),
                            ) {
                                Ok(true) => {
                                    subscriber.wanted = false;
                                    subscriber.unsent = None;
                                    subscriber.last_attempt = Some(now);
                                    subscriber.attempt = Some(Attempt {
                                        token,
                                        deadline: now + SUBMIT_DELAY,
                                        probe: InputProbe::new(
                                            snapshot
                                                .as_ref()
                                                .map(|s| s.rows.clone())
                                                .unwrap_or_default(),
                                            &body,
                                        ),
                                        snapshot,
                                        submitted: false,
                                        second_enter: false,
                                    });
                                }
                                _ => {
                                    router
                                        .injector
                                        .finish_delivery(&subscriber.session_id, token);
                                    subscriber.retry = Some(now + REDELIVERY_INTERVAL);
                                }
                            }
                        }
                        _ => subscriber.retry = Some(now + REDELIVERY_INTERVAL),
                    }
                }
                DeliveryReadiness::RecentlyTyping(delay) => subscriber.retry = Some(now + delay),
                DeliveryReadiness::Unavailable if !subscriber.warned => {
                    subscriber.warned = true;
                    router.warn(format!(
                        "delivery to @{handle} queued until the session resumes"
                    ));
                }
                _ => {}
            }
        }
        Some(
            self.subscribers
                .values()
                .flat_map(|subscriber| {
                    subscriber
                        .retry
                        .into_iter()
                        .chain(subscriber.attempt.as_ref().map(|a| a.deadline))
                        .chain(subscriber.pause_check)
                })
                .fold(self.redelivery, Instant::min),
        )
    }

    /// An unsent notice or draft that no unconfirmed attempt of ours left behind is re-checked
    /// against the composer before it holds a member with unread messages.
    fn readiness(router: &Router, subscriber: &Subscriber, unread: usize) -> DeliveryReadiness {
        let readiness = router.injector.delivery_readiness(&subscriber.session_id);
        if !matches!(
            readiness,
            DeliveryReadiness::Unsent | DeliveryReadiness::Drafting
        ) || unread == 0
            || subscriber.unsent.is_some()
            || subscriber.attempt.is_some()
        {
            return readiness;
        }
        router.injector.recheck_composer(&subscriber.session_id);
        router.injector.delivery_readiness(&subscriber.session_id)
    }

    /// Log each change of the reason delivery is held, and warn once a hold with unread messages
    /// outlasts the redelivery backoff. A working member is a hold that never warns.
    fn hold(
        router: &Router,
        handle: &str,
        subscriber: &mut Subscriber,
        reason: Option<DeliveryReadiness>,
        unread: usize,
        now: Instant,
    ) {
        let reason = reason.map(|reason| match reason {
            DeliveryReadiness::RecentlyTyping(_) => {
                DeliveryReadiness::RecentlyTyping(Duration::ZERO)
            }
            reason => reason,
        });
        if subscriber.hold.as_ref().map(|hold| hold.reason) != reason {
            if let Some(previous) = subscriber.hold.take() {
                log::info!(
                    "router delivery released session={} handle=@{handle} reason={:?}",
                    subscriber.session_id,
                    previous.reason
                );
            }
            if let Some(reason) = reason {
                log::info!(
                    "router delivery held session={} handle=@{handle} reason={reason:?}",
                    subscriber.session_id
                );
                subscriber.hold = Some(Hold {
                    reason,
                    since: now,
                    warned: false,
                });
            }
        }
        let Some(hold) = subscriber.hold.as_mut().filter(|hold| {
            !hold.warned
                && unread > 0
                && now.saturating_duration_since(hold.since) >= REDELIVERY_BACKOFF
        }) else {
            return;
        };
        let words = match hold.reason {
            DeliveryReadiness::Unsent => "its inbox notice is still unsubmitted in its terminal",
            DeliveryReadiness::Drafting => "an unsent draft is in its terminal",
            DeliveryReadiness::NeedsYou => "it is waiting for a response to its prompt",
            DeliveryReadiness::RecentlyTyping(_) => "someone keeps typing in its terminal",
            DeliveryReadiness::InFlight => "earlier input to its terminal has not finished",
            DeliveryReadiness::Unconfirmed => "it shows busy but its agent has not reported a turn",
            // A working member's long turn is not a stall. An unavailable session has its own
            // queued warning, and a hooked first turn stays Starting until it ends.
            DeliveryReadiness::Ready { .. }
            | DeliveryReadiness::Working
            | DeliveryReadiness::Unavailable
            | DeliveryReadiness::Pending
            | DeliveryReadiness::Starting => return,
        };
        hold.warned = true;
        router.warn(format!("delivery to @{handle} is held: {words}"));
    }

    fn blocked(
        router: &Router,
        handle: &str,
        subscriber: &mut Subscriber,
        unread: usize,
        reason: Option<&str>,
    ) {
        let next = reason.map(|reason| (unread, reason.to_owned()));
        if subscriber.blocked == next {
            return;
        }
        subscriber.blocked = next.clone();
        router.ui_notifier.delivery_blocked(&DeliveryBlockedEvent {
            mission_id: router.mission_id.clone(),
            session_id: subscriber.session_id.clone(),
            handle: handle.to_owned(),
            unread_count: unread,
            blocked: next.is_some(),
            reason: next.map(|(_, reason)| reason),
        });
    }

    fn clear(router: &Router, handle: &str, subscriber: &mut Subscriber) {
        if let Some(attempt) = subscriber.attempt.take() {
            log::info!("router attempt outcome=Unverified mission={} session={} handle=@{handle} runtime={} reason=Unmounted", router.mission_id, subscriber.session_id, attempt.snapshot.as_ref().map(|s| s.runtime.as_str()).unwrap_or("unknown"));
            router
                .injector
                .finish_delivery(&subscriber.session_id, attempt.token);
        }
        Self::blocked(router, handle, subscriber, 0, None);
    }
}

fn blocked_reason(readiness: DeliveryReadiness) -> Option<&'static str> {
    match readiness {
        DeliveryReadiness::Drafting => Some("draft"),
        DeliveryReadiness::Unsent => Some("unsubmitted"),
        DeliveryReadiness::NeedsYou => Some("needs_you"),
        _ => None,
    }
}

pub(super) struct Driver {
    tx: mpsc::Sender<Input>,
    thread: JoinHandle<()>,
}

impl Driver {
    pub(super) fn start(
        router: Weak<Router>,
        tx: mpsc::Sender<Input>,
        rx: mpsc::Receiver<Input>,
    ) -> Self {
        let thread = std::thread::Builder::new()
            .name("mission-dispatch".into())
            .spawn(move || {
                let mut dispatcher = Dispatcher::new(Instant::now());
                let mut input = Some(Input::RosterChanged);
                loop {
                    let Some(router) = router.upgrade() else {
                        return;
                    };
                    let deadline = dispatcher.advance(&router, input, Instant::now());
                    drop(router);
                    let Some(deadline) = deadline else {
                        return;
                    };
                    input =
                        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                            Ok(input) => Some(input),
                            Err(mpsc::RecvTimeoutError::Timeout) => None,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        };
                }
            })
            .expect("start mission dispatcher");
        Self { tx, thread }
    }
    pub(super) fn stop(self) {
        let _ = self.tx.send(Input::Stop);
        if self.thread.thread().id() != std::thread::current().id() {
            let _ = self.thread.join();
        }
    }
}
