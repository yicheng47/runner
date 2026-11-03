// Reports to a mission's linked chat (#748). Each chat holds at most one
// pending report per mission; reports go one at a time, oldest first, each
// waiting for the chat to be idle. An attempt is the inbox dispatcher's:
// type the line, press Enter after SUBMIT_DELAY, confirm, and pause while
// the line sits unsubmitted.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use runner_terminal::delivery_probe::{InputProbe, ProbeState};

use super::dispatch::CONFIRM_WINDOW;
use super::{
    DeliveryReadiness, DeliverySnapshot, SessionDeliveryEvent, StdinInjector, SUBMIT_DELAY,
};

const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const WAIT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReportKind {
    /// The mission entered a state the chat hears about.
    State,
    /// The mission was linked to the chat by someone other than its agent.
    Linked,
    /// A member crashed, whatever state that leaves the mission in.
    Crashed,
}

/// A report's line, read when it is sent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    Send(String),
    /// The report no longer applies.
    Drop,
    /// The mission's state is not known yet.
    Wait,
}

/// What a report says, read when it is sent.
pub(crate) trait ReportSource {
    fn line(&self, chat: &str, mission: &str, kind: ReportKind) -> Line;
    /// Whether `chat` can still receive reports: it exists and is not archived.
    fn chat_open(&self, chat: &str) -> bool;
}

struct Pending {
    mission: String,
    kind: ReportKind,
    seq: u64,
}

struct Attempt {
    seq: u64,
    mission: String,
    token: u64,
    deadline: Instant,
    snapshot: Option<DeliverySnapshot>,
    probe: InputProbe,
    submitted: bool,
    second_enter: bool,
}

/// A report whose attempt ended without confirmation, so its line may still be
/// in the composer; `seen` once the probe has found it there.
struct Unsent {
    seq: u64,
    snapshot: DeliverySnapshot,
    probe: InputProbe,
    seen: bool,
}

#[derive(Default)]
struct Chat {
    queue: Vec<Pending>,
    attempt: Option<Attempt>,
    unsent: Option<Unsent>,
    /// The composer cleared since the line was last seen there.
    input_cleared: bool,
    /// While an unsent line holds the chat, when to look at the composer again.
    pause_check: Option<Instant>,
    retry: Option<Instant>,
    /// What last held a waiting report, logged when it changes.
    held: Option<String>,
    /// The last unconfirmed attempt's report and the chat as it was typed into, so a
    /// turn its line started after the confirmation window still counts as delivered.
    late: Option<(u64, DeliverySnapshot)>,
}

#[derive(Default)]
pub(crate) struct ReportDispatcher {
    chats: HashMap<String, Chat>,
    seq: u64,
}

impl ReportDispatcher {
    /// Queue a report; a report for the same mission still waiting in that chat is replaced.
    pub(crate) fn queue(&mut self, chat: &str, mission: &str, kind: ReportKind) {
        self.seq += 1;
        let seq = self.seq;
        let queue = &mut self.chats.entry(chat.to_owned()).or_default().queue;
        match queue.iter_mut().find(|pending| pending.mission == mission) {
            Some(pending) => {
                pending.kind = kind;
                pending.seq = seq;
            }
            None => queue.push(Pending {
                mission: mission.to_owned(),
                kind,
                seq,
            }),
        }
    }

    /// Drop the mission's waiting reports, as when it is unlinked or its link moves.
    pub(crate) fn drop_mission(&mut self, mission: &str) {
        for chat in self.chats.values_mut() {
            chat.queue.retain(|pending| pending.mission != mission);
        }
    }

    #[cfg(test)]
    pub(crate) fn pending(&self, chat: &str) -> Vec<(String, ReportKind)> {
        self.chats
            .get(chat)
            .map(|chat| {
                chat.queue
                    .iter()
                    .map(|pending| (pending.mission.clone(), pending.kind))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn chats(&self) -> impl Iterator<Item = &str> {
        self.chats.keys().map(String::as_str)
    }

    pub(crate) fn session_event(
        &mut self,
        port: &dyn StdinInjector,
        chat_id: &str,
        event: SessionDeliveryEvent,
    ) {
        let Some(chat) = self.chats.get_mut(chat_id) else {
            return;
        };
        match event {
            SessionDeliveryEvent::Exited | SessionDeliveryEvent::Respawned => {
                if let Some(attempt) = chat.attempt.take() {
                    log::info!(
                        "report attempt outcome=Unverified session={chat_id} reason={event:?}"
                    );
                    port.finish_delivery(chat_id, attempt.token);
                }
                chat.unsent = None;
                chat.pause_check = None;
                chat.input_cleared = false;
            }
            SessionDeliveryEvent::InputCleared => chat.input_cleared = true,
            _ => {}
        }
        chat.retry = None;
    }

    /// All timing is driven by the caller's clock. Returns the next deadline, if any.
    pub(crate) fn advance(
        &mut self,
        port: &dyn StdinInjector,
        source: &dyn ReportSource,
        now: Instant,
    ) -> Option<Instant> {
        self.chats.retain(|chat_id, chat| {
            if source.chat_open(chat_id) {
                return true;
            }
            if let Some(attempt) = chat.attempt.take() {
                port.finish_delivery(chat_id, attempt.token);
            }
            false
        });
        for (chat_id, chat) in &mut self.chats {
            Self::advance_chat(port, source, chat_id, chat, now);
        }
        self.chats.retain(|_, chat| {
            !chat.queue.is_empty() || chat.attempt.is_some() || chat.pause_check.is_some()
        });
        self.chats
            .values()
            .flat_map(|chat| {
                chat.retry
                    .into_iter()
                    .chain(chat.attempt.as_ref().map(|attempt| attempt.deadline))
                    .chain(chat.pause_check)
            })
            .min()
    }

    fn advance_chat(
        port: &dyn StdinInjector,
        source: &dyn ReportSource,
        chat_id: &str,
        chat: &mut Chat,
        now: Instant,
    ) {
        if let Some(mut attempt) = chat.attempt.take() {
            if now < attempt.deadline {
                chat.attempt = Some(attempt);
                return;
            }
            if !attempt.submitted {
                if let Some(current) = port.delivery_snapshot(chat_id) {
                    attempt.probe.locate(&current.rows);
                }
                let submitted = port.inject_reserved(chat_id, attempt.token, b"\r");
                port.finish_delivery(chat_id, attempt.token);
                if let Err(error) = submitted {
                    log::warn!("report Enter failed session={chat_id}: {error}");
                }
                attempt.submitted = true;
                attempt.deadline = now + CONFIRM_WINDOW;
                chat.attempt = Some(attempt);
                return;
            }
            let current = port.delivery_snapshot(chat_id);
            let same = |before: &DeliverySnapshot, current: &DeliverySnapshot| {
                before.generation == current.generation
                    && before.input_revision == current.input_revision
            };
            let unchanged = attempt
                .snapshot
                .as_ref()
                .zip(current.as_ref())
                .is_some_and(|(before, current)| same(before, current));
            let state = current
                .as_ref()
                .map(|current| attempt.probe.state(&current.rows))
                .unwrap_or(ProbeState::Unknown);
            let unsent = current.as_ref().is_some_and(|current| current.unsent);
            let turn_started =
                attempt
                    .snapshot
                    .as_ref()
                    .zip(current.as_ref())
                    .is_some_and(|(before, current)| {
                        before.generation == current.generation
                            && current.turn_started != before.turn_started
                    });
            let present = state == ProbeState::Present || unsent;
            if unchanged && present && !turn_started && !attempt.second_enter {
                let retried = attempt.snapshot.as_ref().is_some_and(|snapshot| {
                    matches!(port.resubmit(chat_id, snapshot, &attempt.probe), Ok(true))
                });
                if retried {
                    attempt.second_enter = true;
                    attempt.deadline = now + CONFIRM_WINDOW;
                    chat.attempt = Some(attempt);
                    return;
                }
            }
            let outcome = if !unchanged {
                "Unverified"
            } else if turn_started || (state == ProbeState::Cleared && !unsent) {
                "Confirmed"
            } else if present {
                "Unsubmitted"
            } else {
                "Unverified"
            };
            log::info!(
                "report attempt outcome={outcome} session={chat_id} mission={}",
                attempt.mission
            );
            if outcome == "Confirmed" {
                if let Some(snapshot) = &attempt.snapshot {
                    port.confirm_delivery(chat_id, snapshot);
                }
                port.synthesize_report_busy(chat_id);
                chat.queue.retain(|pending| pending.seq != attempt.seq);
                chat.retry = Some(now + SUBMIT_DELAY);
            } else {
                // Any unconfirmed line may still sit in the composer: it holds the chat while
                // it is seen or tracked there, so a second report never stacks on it (#753).
                if let Some(snapshot) = attempt.snapshot {
                    chat.late = Some((attempt.seq, snapshot.clone()));
                    chat.unsent = Some(Unsent {
                        seq: attempt.seq,
                        snapshot,
                        probe: attempt.probe,
                        seen: false,
                    });
                    chat.input_cleared = false;
                    chat.pause_check = Some(now + CONFIRM_WINDOW);
                }
                chat.retry = Some(now + RETRY_INTERVAL);
            }
            return;
        }
        if let Some(mut unsent) = chat.unsent.take() {
            let current = port.delivery_snapshot(chat_id);
            let state = current
                .as_ref()
                .map(|current| unsent.probe.state(&current.rows))
                .unwrap_or(ProbeState::Unknown);
            let tracked = current.as_ref().is_some_and(|current| current.unsent);
            if state == ProbeState::Present {
                unsent.seen = true;
                chat.input_cleared = false;
            }
            let gone = (state == ProbeState::Cleared && !tracked)
                || (state == ProbeState::Unknown
                    && unsent.seen
                    && chat.input_cleared
                    && port.report_readiness(chat_id) == (DeliveryReadiness::Ready { idle: true }));
            if !gone && (unsent.seen || tracked) {
                if !chat.queue.is_empty() {
                    Self::log_hold(chat_id, &mut chat.held, Some("OwnLine".into()));
                }
                chat.unsent = Some(unsent);
                chat.pause_check = Some(now + CONFIRM_WINDOW);
                return;
            }
            chat.pause_check = None;
            if gone {
                // The person sent the line themselves: the chat has the report.
                if current.as_ref().is_some_and(|current| {
                    current.generation == unsent.snapshot.generation
                        && current.turn_started != unsent.snapshot.turn_started
                }) {
                    log::info!("report sent by hand session={chat_id}");
                    chat.queue.retain(|pending| pending.seq != unsent.seq);
                }
                chat.retry = None;
            } else {
                chat.unsent = Some(unsent);
            }
        }
        if chat.retry.is_some_and(|retry| now < retry) {
            return;
        }
        chat.retry = None;
        while let Some(pending) = chat.queue.first() {
            let readiness = Self::readiness(port, chat_id, chat.unsent.is_some());
            Self::log_hold(chat_id, &mut chat.held, Self::hold(readiness));
            match readiness {
                DeliveryReadiness::Ready { idle: true } => {}
                DeliveryReadiness::RecentlyTyping(delay) => {
                    chat.retry = Some(now + delay);
                    return;
                }
                _ => return,
            }
            // A fresh native fork can report its turn after the window: with no input
            // since, a turn started in the same session is the line's own. The evidence
            // stays while a retry is only deferred.
            match chat.late.as_ref() {
                Some((seq, _)) if *seq != pending.seq => chat.late = None,
                Some((_, before))
                    if port.delivery_snapshot(chat_id).is_some_and(|current| {
                        current.generation == before.generation
                            && current.input_revision == before.input_revision
                            && current.turn_started != before.turn_started
                    }) =>
                {
                    log::info!(
                        "report attempt outcome=ConfirmedLate session={chat_id} mission={}",
                        pending.mission
                    );
                    if let Some((_, before)) = chat.late.take() {
                        port.confirm_delivery(chat_id, &before);
                    }
                    chat.queue.remove(0);
                    continue;
                }
                _ => {}
            }
            let line = match source.line(chat_id, &pending.mission, pending.kind) {
                Line::Send(line) => line,
                Line::Drop => {
                    chat.queue.remove(0);
                    continue;
                }
                Line::Wait => {
                    chat.retry = Some(now + WAIT_INTERVAL);
                    return;
                }
            };
            let (seq, mission) = (pending.seq, pending.mission.clone());
            let token = match port.reserve_report(chat_id) {
                Ok(Some(token)) => token,
                Ok(None) => return,
                Err(error) => {
                    log::warn!("reserve report session={chat_id}: {error}");
                    chat.retry = Some(now + RETRY_INTERVAL);
                    return;
                }
            };
            let snapshot = port.delivery_snapshot(chat_id);
            match port.inject_reserved(chat_id, token, line.as_bytes()) {
                Ok(true) => {
                    chat.unsent = None;
                    chat.late = None;
                    chat.attempt = Some(Attempt {
                        seq,
                        mission,
                        token,
                        deadline: now + SUBMIT_DELAY,
                        probe: InputProbe::new(
                            snapshot
                                .as_ref()
                                .map(|snapshot| snapshot.rows.clone())
                                .unwrap_or_default(),
                            &line,
                        ),
                        snapshot,
                        submitted: false,
                        second_enter: false,
                    });
                }
                _ => {
                    port.finish_delivery(chat_id, token);
                    chat.retry = Some(now + RETRY_INTERVAL);
                }
            }
            return;
        }
    }

    /// What a readiness holds a report for, if anything.
    fn hold(readiness: DeliveryReadiness) -> Option<String> {
        match readiness {
            DeliveryReadiness::Ready { idle: true } => None,
            DeliveryReadiness::RecentlyTyping(_) => Some("RecentlyTyping".into()),
            other => Some(format!("{other:?}")),
        }
    }

    /// Log each change in what holds a chat's waiting reports, as the inbox dispatcher does:
    /// a readiness hold, or "OwnLine" while a line Runner typed may still sit in the composer.
    fn log_hold(chat_id: &str, held: &mut Option<String>, hold: Option<String>) {
        if *held == hold {
            return;
        }
        match &hold {
            Some(reason) => log::info!("report delivery held session={chat_id} reason={reason}"),
            None => log::info!("report delivery released session={chat_id}"),
        }
        *held = hold;
    }

    /// A tracker-only unsent hold that no unconfirmed attempt of ours left behind is re-checked
    /// against the composer before it holds the chat, as the inbox dispatcher does.
    fn readiness(port: &dyn StdinInjector, chat_id: &str, own_unsent: bool) -> DeliveryReadiness {
        let readiness = port.report_readiness(chat_id);
        if readiness != DeliveryReadiness::Unsent || own_unsent {
            return readiness;
        }
        port.recheck_unsent(chat_id);
        port.report_readiness(chat_id)
    }
}
