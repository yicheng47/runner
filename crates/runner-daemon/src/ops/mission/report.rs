// Mission state and reports to the linked chat (#748). The daemon derives
// one state per mission from its members' statuses and pending questions,
// so reports reach the chat while the app is closed and every client reads
// the same state. Entering Needs you, Failed, Interrupted or Settled queues a
// report for the linked chat; `router::report` delivers it.

use std::collections::{BTreeMap, HashMap};
use std::sync::{mpsc, Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use runner_core::protocol::status::{Activity, AgentStatus, Lifecycle};

use runner_core::protocol::session::SessionRow;
use runner_core::protocol::MissionPhase;

use crate::model::{Mission, MissionStatus, Runtime, SessionStatus};
use crate::router::report::{Line, ReportDispatcher, ReportKind, ReportSource};
use crate::router::{SessionDeliveryEvent, SessionDeliveryListener};
use crate::{repo, AppCore};

/// How long every member stays idle, with nothing pending, before a mission settles.
pub const SETTLE_WINDOW: Duration = Duration::from_secs(30);
/// Status events can arrive in bursts; the state is re-read at most this often.
const REFRESH_INTERVAL: Duration = Duration::from_millis(250);
/// A missed event still reaches the state within this long.
const HEARTBEAT: Duration = Duration::from_secs(30);
/// A failed read is retried after this long.
const REFRESH_BACKOFF: Duration = Duration::from_secs(5);

/// One member's status, as the state derivation reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Member {
    pub lifecycle: Lifecycle,
    /// The member reports Idle or Ready. Unavailable activity is no evidence of idleness.
    pub idle: bool,
    pub needs_you: bool,
    pub failed: bool,
    /// Mid-turn when runnerd stopped, and no turn since (#1187).
    pub interrupted: bool,
    /// Ended its turn with background work it will wake from still pending (#1183).
    pub background: bool,
}

impl From<&AgentStatus> for Member {
    fn from(status: &AgentStatus) -> Self {
        Self {
            lifecycle: status.lifecycle,
            idle: matches!(
                status.observation.activity,
                Activity::Idle | Activity::Ready
            ),
            needs_you: status.observation.needs_you(),
            failed: status.failed_since.is_some(),
            interrupted: status.interrupted_since.is_some(),
            background: status.observation.background,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PhaseInput {
    pub running: bool,
    pub members: Vec<Member>,
    pub pending_asks: usize,
    pub spawning: bool,
}

/// The mission's state, the first that applies winning. `idle_for` is how long
/// every member has been idle with nothing pending, when that is so.
pub(crate) fn derive(input: &PhaseInput, idle_for: Option<Duration>) -> MissionPhase {
    if !input.running {
        return MissionPhase::Ended;
    }
    let live = |member: &&Member| {
        matches!(
            member.lifecycle,
            Lifecycle::Starting | Lifecycle::Resuming | Lifecycle::Running
        )
    };
    // Every member exited, cleanly or not: the mission ended until it is resumed or archived.
    if !input.members.is_empty()
        && !input.spawning
        && !input.members.iter().any(|member| live(&member))
    {
        return MissionPhase::Ended;
    }
    if input.pending_asks > 0 || input.members.iter().filter(live).any(|m| m.needs_you) {
        return MissionPhase::NeedsYou;
    }
    let crashed = input
        .members
        .iter()
        .any(|member| member.lifecycle == Lifecycle::Error);
    if crashed || input.members.iter().filter(live).any(|m| m.failed) {
        return MissionPhase::Failed;
    }
    if input.members.iter().filter(live).any(|m| m.interrupted) {
        return MissionPhase::Interrupted;
    }
    if input.spawning
        || input
            .members
            .iter()
            .filter(live)
            .any(|m| !m.idle || m.background || m.lifecycle != Lifecycle::Running)
    {
        return MissionPhase::Working;
    }
    if idle_for.is_some_and(|idle| idle >= SETTLE_WINDOW) {
        MissionPhase::Settled
    } else {
        MissionPhase::Working
    }
}

/// Whether the settle clock runs: the mission would settle given time.
pub(crate) fn quiet(input: &PhaseInput) -> bool {
    derive(input, Some(Duration::MAX)) == MissionPhase::Settled
}

pub(crate) fn phase_input(
    core: &AppCore,
    mission: &Mission,
    sessions: &[SessionRow],
    statuses: &BTreeMap<String, AgentStatus>,
    pending_asks: usize,
) -> PhaseInput {
    PhaseInput {
        running: mission.status == MissionStatus::Running && mission.archived_at.is_none(),
        members: members(sessions, statuses)
            .map(|(_, member)| member)
            .collect(),
        pending_asks,
        spawning: core.sessions.has_pending_mission_spawns(&mission.id),
    }
}

/// The mission's members, its non-shell roster sessions, with their statuses.
fn members<'a>(
    sessions: &'a [SessionRow],
    statuses: &'a BTreeMap<String, AgentStatus>,
) -> impl Iterator<Item = (&'a str, Member)> + 'a {
    sessions
        .iter()
        .filter(|row| !Runtime::parse(&row.runtime).is_some_and(Runtime::is_shell))
        .map(|row| {
            let member = statuses
                .get(&row.session.id)
                .map(Member::from)
                .unwrap_or(Member {
                    lifecycle: match row.session.status {
                        SessionStatus::Running => Lifecycle::Running,
                        SessionStatus::Stopped => Lifecycle::Stopped,
                        SessionStatus::Crashed => Lifecycle::Error,
                    },
                    ..Member::default()
                });
            (row.session.id.as_str(), member)
        })
}

/// The mission's pending questions: the live router's when it is mounted,
/// else a scan of its log.
pub(crate) fn pending_asks(core: &AppCore, mission: &Mission) -> usize {
    match core.routers.get(&mission.id) {
        Some(router) => router.pending_ask_count(),
        None => super::count_pending_asks_from_log(&runner_core::event_log::mission_dir(
            &core.app_data_dir,
            mission.crew_id.as_deref(),
            &mission.id,
        )),
    }
}

/// What a restart left a running mission, for its first report after boot (#1187).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Restart {
    /// A member was marked interrupted at this boot: report it once, even at the baseline.
    Interrupted,
    /// A member is still interrupted from an earlier restart, which its chat has heard.
    StillInterrupted,
    /// The mission had settled before the restart, and its chat has heard so. A turn
    /// any member starts after this log offset makes its next Settled news again.
    Settled(u64),
}

#[derive(Clone)]
struct Tracked {
    phase: MissionPhase,
    quiet_since: Option<Instant>,
    /// Members that crashed. A new one is reported even when it leaves the mission Ended.
    crashed: std::collections::BTreeSet<String>,
}

/// The derived states. Never held across database or session-manager work,
/// so a request holding a connection can read a state at any time.
#[derive(Default)]
struct Tracking {
    missions: HashMap<String, Tracked>,
    /// The first refresh records each mission's state without reporting it.
    baselined: bool,
    /// Missions launch resume brings back, until their first reported state.
    restarts: HashMap<String, Restart>,
    refresh_due: bool,
    last_refresh: Option<Instant>,
    /// After a failed read, no refresh before this.
    backoff_until: Option<Instant>,
}

impl Tracking {
    /// The earliest a refresh may run: throttled after the last one, and backed off after a
    /// failed one.
    fn next_refresh(&self, now: Instant) -> Instant {
        [
            Some(now),
            self.last_refresh.map(|last| last + REFRESH_INTERVAL),
            self.backoff_until,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(now)
    }

    /// The earliest settle deadline among working missions whose members are all idle.
    fn settle(&self) -> Option<Instant> {
        self.missions
            .values()
            .filter(|tracked| tracked.phase == MissionPhase::Working)
            .filter_map(|tracked| tracked.quiet_since)
            .map(|since| since + SETTLE_WINDOW)
            .min()
    }

    /// When the service next needs to re-read the states. An overdue settle waits for the
    /// throttle and backoff, so the thread never spins on a deadline it cannot act on.
    fn refresh_deadline(&self, now: Instant) -> Option<Instant> {
        let wanted = if self.refresh_due {
            Some(now)
        } else {
            self.settle()
        };
        wanted.map(|wanted| wanted.max(self.next_refresh(now)))
    }
}

#[derive(Default)]
struct Delivery {
    dispatcher: ReportDispatcher,
    listening: std::collections::HashSet<String>,
}

enum Wake {
    Refresh,
    Session(String, SessionDeliveryEvent),
}

/// The daemon's mission states and the reports they send to linked chats.
/// Lock order: `delivery`, then `tracking`. A caller holding a database
/// connection may take `tracking` but must not take `delivery`.
#[derive(Default)]
pub struct Reports {
    tracking: Mutex<Tracking>,
    delivery: Mutex<Delivery>,
    wake: Mutex<Option<mpsc::Sender<Wake>>>,
}

impl SessionDeliveryListener for Reports {
    fn session_delivery_event(&self, session_id: &str, event: SessionDeliveryEvent) {
        self.send(Wake::Session(session_id.to_owned(), event));
    }
}

impl Reports {
    fn send(&self, wake: Wake) {
        if let Some(tx) = self.wake.lock().unwrap().as_ref() {
            let _ = tx.send(wake);
        }
    }

    /// The tracked state, once the service has read the mission.
    pub fn phase(&self, mission_id: &str) -> Option<MissionPhase> {
        self.tracking
            .lock()
            .unwrap()
            .missions
            .get(mission_id)
            .map(|tracked| tracked.phase)
    }

    /// Someone other than the chat's own agent linked the mission: the chat
    /// gets one report of the mission's current state. The caller holds no
    /// database connection.
    pub fn linked(&self, chat: &str, mission: &str) {
        let mut delivery = self.delivery.lock().unwrap();
        delivery.dispatcher.drop_mission(mission);
        delivery.dispatcher.queue(chat, mission, ReportKind::Linked);
        drop(delivery);
        self.send(Wake::Refresh);
    }

    /// The link was removed or moved: a report still waiting for the old chat
    /// is dropped. The caller holds no database connection.
    pub fn unlinked(&self, mission: &str) {
        self.delivery
            .lock()
            .unwrap()
            .dispatcher
            .drop_mission(mission);
        self.send(Wake::Refresh);
    }

    /// Boot recorded what the restart left each resumed mission. Set before the service starts.
    pub(crate) fn restarted(&self, restarts: HashMap<String, Restart>) {
        self.tracking.lock().unwrap().restarts = restarts;
    }

    #[cfg(test)]
    pub(crate) fn pending(&self, chat: &str) -> Vec<(String, ReportKind)> {
        self.delivery.lock().unwrap().dispatcher.pending(chat)
    }

    #[cfg(test)]
    fn refresh_now(&self) {
        self.tracking.lock().unwrap().refresh_due = true;
    }

    /// Start the service's thread, woken by the core's events and the linked
    /// chats' delivery events.
    pub fn start(core: AppCore) {
        let (tx, rx) = mpsc::channel();
        *core.reports.wake.lock().unwrap() = Some(tx.clone());
        let mut events = core.events.subscribe();
        std::thread::Builder::new()
            .name("mission-report-events".into())
            .spawn(move || loop {
                use tokio::sync::broadcast::error::RecvError;
                match events.blocking_recv() {
                    Ok(event) if !relevant(event.name) => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
                if tx.send(Wake::Refresh).is_err() {
                    return;
                }
            })
            .expect("start mission report events");
        std::thread::Builder::new()
            .name("mission-reports".into())
            .spawn(move || {
                let mut deadline = Some(Instant::now());
                loop {
                    let timeout = deadline
                        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                        .unwrap_or(HEARTBEAT)
                        .min(HEARTBEAT);
                    match rx.recv_timeout(timeout) {
                        Ok(wake) => core.reports.wake_with(&core, wake),
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            core.reports.tracking.lock().unwrap().refresh_due = true;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                    while let Ok(wake) = rx.try_recv() {
                        core.reports.wake_with(&core, wake);
                    }
                    deadline = core.reports.tick(&core, Instant::now());
                }
            })
            .expect("start mission reports");
    }

    fn wake_with(&self, core: &AppCore, wake: Wake) {
        match wake {
            Wake::Refresh => self.tracking.lock().unwrap().refresh_due = true,
            Wake::Session(chat, event) => self.delivery.lock().unwrap().dispatcher.session_event(
                core.sessions.as_ref(),
                &chat,
                event,
            ),
        }
    }

    /// Re-read the states when due, queue reports for transitions, and advance
    /// delivery. Returns when it next needs to run.
    pub(crate) fn tick(&self, core: &AppCore, now: Instant) -> Option<Instant> {
        let previous = {
            let mut tracking = self.tracking.lock().unwrap();
            tracking
                .refresh_deadline(now)
                .is_some_and(|deadline| deadline <= now)
                .then(|| {
                    tracking.refresh_due = false;
                    tracking.last_refresh = Some(now);
                    (
                        tracking.missions.clone(),
                        tracking.baselined,
                        tracking.restarts.clone(),
                    )
                })
        };
        if let Some((previous, baselined, restarts)) = previous {
            match read_states(core, &previous, &restarts, now) {
                Ok(states) => self.apply(core, previous, baselined, &restarts, states),
                Err(error) => {
                    log::warn!("mission state refresh failed: {error}");
                    let mut tracking = self.tracking.lock().unwrap();
                    tracking.refresh_due = true;
                    tracking.backoff_until = Some(now + REFRESH_BACKOFF);
                }
            }
        }

        let mut delivery = self.delivery.lock().unwrap();
        let listener: Weak<dyn SessionDeliveryListener> = Arc::downgrade(&core.reports) as _;
        let chats: Vec<String> = delivery.dispatcher.chats().map(str::to_owned).collect();
        for chat in chats {
            if delivery.listening.insert(chat.clone()) {
                core.sessions
                    .register_delivery_listener(&chat, listener.clone());
            }
        }
        let tracking = self.tracking.lock().unwrap();
        let phases = tracking
            .missions
            .iter()
            .map(|(id, tracked)| (id.clone(), tracked.phase))
            .collect();
        let refresh = tracking.refresh_deadline(now);
        drop(tracking);
        let delivery_deadline =
            delivery
                .dispatcher
                .advance(core.sessions.as_ref(), &LineSource { core, phases }, now);
        [delivery_deadline, refresh].into_iter().flatten().min()
    }

    /// Record the new states, queue a report for each transition into a
    /// reported state, and announce the changes. After a restart, a mission's
    /// first reported state is reported only when it is news (#1187).
    fn apply(
        &self,
        core: &AppCore,
        previous: HashMap<String, Tracked>,
        baselined: bool,
        restarts: &HashMap<String, Restart>,
        states: Vec<(Mission, Tracked)>,
    ) {
        let mut changed = Vec::new();
        let mut reports = Vec::new();
        let mut consumed_restarts = Vec::new();
        for (mission, tracked) in &states {
            let before = previous.get(&mission.id);
            let phase_changed = before.map(|before| before.phase) != Some(tracked.phase);
            if phase_changed {
                changed.push(mission.id.clone());
            }
            let restart = restarts.get(&mission.id).copied();
            let first_reading = !(baselined || before.is_some());
            if first_reading
                && !(restart == Some(Restart::Interrupted)
                    && tracked.phase == MissionPhase::Interrupted)
            {
                continue;
            }
            // A crash is reported even when it leaves no member running and the mission Ended.
            let crashed = !first_reading
                && tracked
                    .crashed
                    .iter()
                    .any(|id| !before.is_some_and(|before| before.crashed.contains(id)));
            let kind = if crashed {
                ReportKind::Crashed
            } else if phase_changed && tracked.phase.reported() {
                if let Some(restart) = restart {
                    let carried = match (restart, tracked.phase) {
                        (Restart::StillInterrupted, MissionPhase::Interrupted) => true,
                        (Restart::Settled(offset), MissionPhase::Settled) => {
                            !super::interrupt::turn_since(core, mission, offset)
                        }
                        _ => false,
                    };
                    // A settled mission keeps its carry until a member works, so members
                    // resuming one at a time cannot make its quiet look like news.
                    if !(carried && matches!(restart, Restart::Settled(_))) {
                        consumed_restarts.push(mission.id.clone());
                    }
                    if carried {
                        continue;
                    }
                }
                ReportKind::State
            } else {
                continue;
            };
            if let Some(chat) = mission
                .linked_chat
                .as_ref()
                .filter(|chat| chat.reports && !chat.archived)
            {
                log::info!(
                    "mission {} is {:?}; reporting {kind:?} to chat {}",
                    mission.id,
                    tracked.phase,
                    chat.session_id
                );
                reports.push((chat.session_id.clone(), mission.id.clone(), kind));
            }
        }
        let gone: Vec<String> = previous
            .keys()
            .filter(|id| !states.iter().any(|(mission, _)| &mission.id == *id))
            .cloned()
            .collect();
        {
            let mut tracking = self.tracking.lock().unwrap();
            tracking.missions = states
                .into_iter()
                .map(|(mission, tracked)| (mission.id, tracked))
                .collect();
            tracking.baselined = true;
            tracking.backoff_until = None;
            for id in consumed_restarts.iter().chain(&gone) {
                tracking.restarts.remove(id);
            }
        }
        {
            let mut delivery = self.delivery.lock().unwrap();
            for id in &gone {
                delivery.dispatcher.drop_mission(id);
            }
            for (chat, mission, kind) in reports {
                delivery.dispatcher.queue(&chat, &mission, kind);
            }
        }
        for mission_id in changed {
            core.events.emit(
                "mission/changed",
                &serde_json::json!({ "mission_id": mission_id }),
            );
        }
    }
}

/// Every unarchived mission's state, read without holding the service's locks.
fn read_states(
    core: &AppCore,
    previous: &HashMap<String, Tracked>,
    restarts: &HashMap<String, Restart>,
    now: Instant,
) -> crate::error::Result<Vec<(Mission, Tracked)>> {
    let statuses = crate::ops::session::session_status_snapshot(core)?;
    let conn = core.db.get()?;
    let mut states = Vec::new();
    for mission in repo::mission::list(&conn, None)? {
        let sessions =
            match crate::ops::session::list_for_mission(&conn, &core.app_data_dir, &mission.id) {
                Ok(sessions) => sessions,
                Err(error) => {
                    // Keep the last state, but a mission that cannot be read does not settle.
                    log::warn!("mission {} state unavailable: {error}", mission.id);
                    if let Some(tracked) = previous.get(&mission.id) {
                        let quiet_since = None;
                        let tracked = Tracked {
                            quiet_since,
                            ..tracked.clone()
                        };
                        states.push((mission, tracked));
                    }
                    continue;
                }
            };
        let input = phase_input(
            core,
            &mission,
            &sessions,
            &statuses,
            pending_asks(core, &mission),
        );
        let quiet_since = quiet(&input).then(|| {
            previous
                .get(&mission.id)
                .and_then(|tracked| tracked.quiet_since)
                .unwrap_or_else(|| match restarts.get(&mission.id) {
                    // Settled before the restart and nobody has worked since: it stays Settled.
                    Some(Restart::Settled(offset))
                        if !super::interrupt::turn_since(core, &mission, *offset) =>
                    {
                        now.checked_sub(SETTLE_WINDOW).unwrap_or(now)
                    }
                    _ => now,
                })
        });
        let phase = derive(
            &input,
            quiet_since.map(|since| now.saturating_duration_since(since)),
        );
        let crashed = members(&sessions, &statuses)
            .filter(|(_, member)| member.lifecycle == Lifecycle::Error)
            .map(|(id, _)| id.to_owned())
            .collect();
        states.push((
            mission,
            Tracked {
                phase,
                quiet_since,
                crashed,
            },
        ));
    }
    Ok(states)
}

fn relevant(name: &str) -> bool {
    matches!(
        name,
        "session/status"
            | "session/exit"
            | "session/spawned"
            | "session/archived"
            | "session/updated"
            | "event/appended"
            | "mission/changed"
    )
}

struct LineSource<'a> {
    core: &'a AppCore,
    phases: HashMap<String, MissionPhase>,
}

impl ReportSource for LineSource<'_> {
    fn line(&self, chat: &str, mission: &str, kind: ReportKind) -> Line {
        let Ok(conn) = self.core.db.get() else {
            return Line::Wait;
        };
        let Ok(Some(mission)) = repo::mission::get(&conn, mission) else {
            return Line::Drop;
        };
        let linked = mission
            .linked_chat
            .as_ref()
            .filter(|linked| linked.session_id == chat && linked.reports && !linked.archived);
        if mission.archived_at.is_some() || linked.is_none() {
            return Line::Drop;
        }
        let Some(phase) = self.phases.get(&mission.id).copied() else {
            return Line::Wait;
        };
        line(&mission, phase, kind).map_or(Line::Drop, Line::Send)
    }

    fn chat_open(&self, chat: &str) -> bool {
        self.core.db.get().ok().is_some_and(|conn| {
            repo::session::get_direct(&conn, chat)
                .ok()
                .flatten()
                .is_some_and(|chat| chat.row.archived_at.is_none())
        })
    }
}

const INTERRUPTED: &str = "was interrupted by a Runner restart";

/// The one-line pointer a report types into the chat. It carries no mission
/// content beyond the title.
pub(crate) fn line(mission: &Mission, phase: MissionPhase, kind: ReportKind) -> Option<String> {
    let what = match (kind, phase) {
        (ReportKind::Crashed, _) => "failed".to_owned(),
        (ReportKind::State, MissionPhase::NeedsYou) => "needs you".to_owned(),
        (ReportKind::State, MissionPhase::Failed) => "failed".to_owned(),
        (ReportKind::State, MissionPhase::Interrupted) => INTERRUPTED.to_owned(),
        (ReportKind::State, MissionPhase::Settled) => "settled".to_owned(),
        (
            ReportKind::State,
            MissionPhase::Working | MissionPhase::Ended | MissionPhase::Unknown,
        ) => return None,
        (ReportKind::Linked, phase) => format!(
            "is now linked to this chat; it {}",
            match phase {
                MissionPhase::NeedsYou => "needs you",
                MissionPhase::Failed => "failed",
                MissionPhase::Interrupted => INTERRUPTED,
                MissionPhase::Working | MissionPhase::Unknown => "is working",
                MissionPhase::Settled => "settled",
                MissionPhase::Ended => "ended",
            }
        ),
    };
    let title = mission
        .title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "[runner] Mission {id} \"{title}\" {what}. Read it with runner mission show {id}.",
        id = mission.id
    ))
}

#[cfg(test)]
mod tests;
