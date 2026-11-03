// Mission state and reports to the linked chat (#748). The daemon derives
// one state per mission from its members' statuses and pending questions,
// so reports reach the chat while the app is closed and every client reads
// the same state. Entering Needs you, Failed or Settled queues a report for
// the linked chat; `router::report` delivers it.

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
    if input.spawning
        || input
            .members
            .iter()
            .filter(live)
            .any(|m| !m.idle || m.lifecycle != Lifecycle::Running)
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
                    (tracking.missions.clone(), tracking.baselined)
                })
        };
        if let Some((previous, baselined)) = previous {
            match read_states(core, &previous, now) {
                Ok(states) => self.apply(core, previous, baselined, states),
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
    /// reported state, and announce the changes.
    fn apply(
        &self,
        core: &AppCore,
        previous: HashMap<String, Tracked>,
        baselined: bool,
        states: Vec<(Mission, Tracked)>,
    ) {
        let mut changed = Vec::new();
        let mut reports = Vec::new();
        for (mission, tracked) in &states {
            let before = previous.get(&mission.id);
            let phase_changed = before.map(|before| before.phase) != Some(tracked.phase);
            if phase_changed {
                changed.push(mission.id.clone());
            }
            if !(baselined || before.is_some()) {
                continue;
            }
            // A crash is reported even when it leaves no member running and the mission Ended.
            let crashed = tracked
                .crashed
                .iter()
                .any(|id| !before.is_some_and(|before| before.crashed.contains(id)));
            let kind = if crashed {
                ReportKind::Crashed
            } else if phase_changed && tracked.phase.reported() {
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
                .unwrap_or(now)
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

/// The one-line pointer a report types into the chat. It carries no mission
/// content beyond the title.
pub(crate) fn line(mission: &Mission, phase: MissionPhase, kind: ReportKind) -> Option<String> {
    let what = match (kind, phase) {
        (ReportKind::Crashed, _) => "failed".to_owned(),
        (ReportKind::State, MissionPhase::NeedsYou) => "needs you".to_owned(),
        (ReportKind::State, MissionPhase::Failed) => "failed".to_owned(),
        (ReportKind::State, MissionPhase::Settled) => "settled".to_owned(),
        (ReportKind::State, MissionPhase::Working | MissionPhase::Ended) => return None,
        (ReportKind::Linked, phase) => format!(
            "is now linked to this chat; it {}",
            match phase {
                MissionPhase::NeedsYou => "needs you",
                MissionPhase::Failed => "failed",
                MissionPhase::Working => "is working",
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
mod tests {
    use super::*;
    use crate::model::SessionStatus;
    use crate::ops::mission::{start, MissionLinkInput, MissionStart};
    use crate::ops::project::ProjectScope;
    use crate::session::runtime::SessionActivityState;
    use crate::test_support::{insert_session_row, test_core_in, test_session_row};
    use runner_core::model::{EventDraft, SignalType};

    fn member(lifecycle: Lifecycle) -> Member {
        Member {
            lifecycle,
            ..Member::default()
        }
    }

    fn running(members: Vec<Member>) -> PhaseInput {
        PhaseInput {
            running: true,
            members,
            pending_asks: 0,
            spawning: false,
        }
    }

    #[test]
    fn the_first_state_that_applies_wins() {
        use MissionPhase::*;
        let idle = Member {
            idle: true,
            ..member(Lifecycle::Running)
        };
        let working = member(Lifecycle::Running);
        let waiting = Member {
            needs_you: true,
            ..idle
        };
        let failed = Member {
            failed: true,
            ..idle
        };
        let crashed = member(Lifecycle::Error);
        let stopped = member(Lifecycle::Stopped);
        let settled = Some(SETTLE_WINDOW);
        let cases = [
            (running(vec![waiting, crashed, working]), settled, NeedsYou),
            (
                PhaseInput {
                    pending_asks: 1,
                    ..running(vec![failed])
                },
                settled,
                NeedsYou,
            ),
            (running(vec![crashed, working]), settled, Failed),
            (running(vec![crashed, idle]), settled, Failed),
            (running(vec![failed, working]), settled, Failed),
            (running(vec![working, idle]), settled, Working),
            (running(vec![member(Lifecycle::Starting)]), settled, Working),
            (
                PhaseInput {
                    spawning: true,
                    ..running(vec![])
                },
                settled,
                Working,
            ),
            (
                running(vec![idle, idle]),
                Some(Duration::from_secs(29)),
                Working,
            ),
            (running(vec![idle, stopped]), settled, Settled),
            (running(vec![]), settled, Settled),
            (running(vec![stopped, stopped]), settled, Ended),
            (running(vec![crashed]), settled, Ended),
            (running(vec![crashed, stopped]), settled, Ended),
            (
                PhaseInput {
                    running: false,
                    ..running(vec![waiting])
                },
                settled,
                Ended,
            ),
        ];
        for (input, idle_for, phase) in cases {
            assert_eq!(derive(&input, idle_for), phase, "{input:?}");
        }
        assert!(quiet(&running(vec![idle])));
        assert!(!quiet(&running(vec![working])));
        assert!(!quiet(&running(vec![stopped])));
    }

    #[test]
    fn unavailable_activity_is_no_evidence_that_a_member_is_idle() {
        use runner_core::protocol::status::AgentObservation;
        let status = |activity| AgentStatus {
            lifecycle: Lifecycle::Running,
            observation: AgentObservation {
                activity,
                ..Default::default()
            },
            ..Default::default()
        };
        let unknown = running(vec![Member::from(&status(Activity::Unavailable))]);
        assert!(!quiet(&unknown));
        assert_eq!(derive(&unknown, Some(Duration::MAX)), MissionPhase::Working);
        for activity in [Activity::Idle, Activity::Ready] {
            assert!(quiet(&running(vec![Member::from(&status(activity))])));
        }
        assert!(!quiet(&running(vec![Member::from(&status(
            Activity::Working
        ))])));
    }

    #[test]
    fn a_report_is_a_one_line_pointer() {
        let mission = Mission {
            id: "01M6DD9".into(),
            crew_id: None,
            project_id: None,
            title: "1025 — Remote\\nchat  switch".replace("\\n", "\n"),
            status: MissionStatus::Running,
            goal_override: None,
            cwd: None,
            started_at: chrono::Utc::now(),
            stopped_at: None,
            pinned_at: None,
            archived_at: None,
            linked_chat: None,
        };
        assert_eq!(
            line(&mission, MissionPhase::NeedsYou, ReportKind::State).unwrap(),
            "[runner] Mission 01M6DD9 \"1025 — Remote chat switch\" needs you. Read it with runner mission show 01M6DD9."
        );
        assert!(line(&mission, MissionPhase::Settled, ReportKind::State)
            .unwrap()
            .contains("\" settled. Read it"));
        assert!(line(&mission, MissionPhase::Failed, ReportKind::State)
            .unwrap()
            .contains("\" failed. Read it"));
        assert_eq!(
            line(&mission, MissionPhase::Working, ReportKind::State),
            None
        );
        assert_eq!(line(&mission, MissionPhase::Ended, ReportKind::State), None);
        assert!(line(&mission, MissionPhase::Ended, ReportKind::Crashed)
            .unwrap()
            .contains("\" failed. Read it"));
        assert_eq!(
            line(&mission, MissionPhase::Working, ReportKind::Linked).unwrap(),
            "[runner] Mission 01M6DD9 \"1025 — Remote chat switch\" is now linked to this chat; it is working. Read it with runner mission show 01M6DD9."
        );
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        core: AppCore,
        crew: String,
        now: Instant,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let core = test_core_in(dir.path().to_path_buf());
            let conn = core.db.get().unwrap();
            for chat in ["chat", "other"] {
                let mut row = test_session_row(chat, SessionStatus::Running);
                row.agent_runtime = Some("codex".into());
                insert_session_row(&conn, &row).unwrap();
            }
            let crew = crate::ops::crew::create(
                &conn,
                crate::ops::crew::CreateCrewInput {
                    name: "Crew".into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .id;
            crate::test_support::insert_test_role(&conn, "role", "worker", "codex", "codex");
            crate::test_support::insert_test_slot(&conn, "slot", &crew, "role", "worker", 0, false);
            drop(conn);
            Self {
                _dir: dir,
                core,
                crew,
                now: Instant::now(),
            }
        }

        /// A mission started from `chat` with one running, idle member.
        fn mission(&self, chat: Option<&str>, no_report: bool) -> Mission {
            let mission = self.mission_without_status(chat, no_report);
            self.activity(&mission, SessionActivityState::Idle);
            mission
        }

        /// The same, before the member reports any activity.
        fn mission_without_status(&self, chat: Option<&str>, no_report: bool) -> Mission {
            let mission = start(
                &mut self.core.db.get().unwrap(),
                &self.core.app_data_dir,
                MissionStart {
                    crew_id: Some(self.crew.clone()),
                    scope: ProjectScope::Root,
                    title: "Reported".into(),
                    goal_override: None,
                    cwd: Some("/tmp".into()),
                    started_by_session_id: chat.map(str::to_owned),
                    no_report,
                },
            )
            .unwrap()
            .mission;
            let mut member =
                test_session_row(&format!("{}-member", mission.id), SessionStatus::Running);
            member.agent_runtime = Some("codex".into());
            member.mission_id = Some(mission.id.clone());
            member.role_id = Some("role".into());
            member.slot_id = Some("slot".into());
            insert_session_row(&self.core.db.get().unwrap(), &member).unwrap();
            mission
        }

        fn activity(&self, mission: &Mission, state: SessionActivityState) {
            self.core
                .sessions
                .test_running_activity(&format!("{}-member", mission.id), state);
        }

        fn crash(&self, mission: &Mission) {
            self.core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET status = 'crashed' WHERE mission_id = ?1",
                    [&mission.id],
                )
                .unwrap();
        }

        /// Re-read the states `seconds` after the fixture's start.
        fn at(&self, seconds: u64) {
            self.core.reports.refresh_now();
            self.core
                .reports
                .tick(&self.core, self.now + Duration::from_secs(seconds));
        }

        /// Advance the service `ms` after the fixture's start without asking for a refresh.
        fn tick_ms(&self, ms: u64) -> Option<Instant> {
            self.core
                .reports
                .tick(&self.core, self.now + Duration::from_millis(ms))
        }

        fn ask(&self, mission: &Mission) {
            let log = runner_core::EventLog::open(&runner_core::event_log::mission_dir(
                &self.core.app_data_dir,
                Some(&self.crew),
                &mission.id,
            ))
            .unwrap();
            log.append(EventDraft::signal(
                Some(&self.crew),
                &mission.id,
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"prompt": "Ship?"}),
            ))
            .unwrap();
        }

        fn link(&self, mission: &Mission, chat: &str, caller: Option<&str>) {
            crate::ops::mission::link::mission_link(
                &self.core,
                MissionLinkInput {
                    mission_id: mission.id.clone(),
                    session_id: chat.into(),
                    no_report: false,
                    caller_session_id: caller.map(str::to_owned),
                },
            )
            .unwrap();
        }

        fn pending(&self, chat: &str) -> Vec<(String, ReportKind)> {
            self.core.reports.pending(chat)
        }
    }

    #[test]
    fn a_transition_queues_one_report_and_a_later_one_replaces_it() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);

        f.at(0);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );
        assert!(f.pending("chat").is_empty());

        f.at(31);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Settled)
        );
        assert_eq!(f.pending("chat"), [(mission.id.clone(), ReportKind::State)]);
        f.at(32);
        assert_eq!(f.pending("chat").len(), 1, "a state reports once");

        f.ask(&mission);
        f.at(33);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::NeedsYou)
        );
        assert_eq!(
            f.pending("chat"),
            [(mission.id.clone(), ReportKind::State)],
            "the newer state replaces the report still waiting"
        );
        let summary = crate::daemon::block_on(crate::ops::mission::mission_list_summary_impl(
            &f.core, None,
        ))
        .unwrap();
        assert_eq!(summary[0].phase, Some(MissionPhase::NeedsYou));
    }

    #[test]
    fn a_lone_member_crashing_reports_failed_once_and_ends_the_mission() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.at(0);
        f.at(31);
        f.core.reports.unlinked(&mission.id);

        f.crash(&mission);
        f.at(32);
        assert_eq!(f.core.reports.phase(&mission.id), Some(MissionPhase::Ended));
        assert_eq!(
            f.pending("chat"),
            [(mission.id.clone(), ReportKind::Crashed)],
            "a crash is reported even when it leaves no member running"
        );
        f.at(33);
        f.at(64);
        assert_eq!(f.pending("chat").len(), 1, "one report per crash");
    }

    #[test]
    fn the_first_reading_after_a_daemon_starts_is_silent() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.ask(&mission);
        f.at(0);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::NeedsYou)
        );
        assert!(f.pending("chat").is_empty());
    }

    #[test]
    fn a_member_without_idle_evidence_restarts_the_settle_clock() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.at(0);
        f.activity(&mission, SessionActivityState::Busy);
        f.at(20);
        f.activity(&mission, SessionActivityState::Idle);
        f.at(25);
        f.at(54);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );
        assert!(f.pending("chat").is_empty());
        f.at(55);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Settled)
        );
        assert_eq!(f.pending("chat").len(), 1);

        let unknown = Fixture::new();
        let quiet = unknown.mission_without_status(Some("chat"), false);
        unknown.at(0);
        unknown.at(120);
        assert_eq!(
            unknown.core.reports.phase(&quiet.id),
            Some(MissionPhase::Working),
            "a running member with no activity evidence never settles"
        );
    }

    #[test]
    fn an_overdue_settle_waits_for_the_throttle_instead_of_spinning() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.at(0);
        f.core.reports.refresh_now();
        f.tick_ms(29_900);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );

        let deadline = f.tick_ms(30_000).unwrap();
        assert_eq!(deadline, f.now + Duration::from_millis(30_150));
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );
        assert_eq!(
            f.tick_ms(30_100),
            Some(deadline),
            "the deadline holds until it can act"
        );
        f.tick_ms(30_150);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Settled)
        );
    }

    #[test]
    fn a_mission_that_cannot_be_read_keeps_its_state_without_settling() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.at(0);
        let roster =
            runner_core::event_log::mission_dir(&f.core.app_data_dir, Some(&f.crew), &mission.id)
                .join("roster.json");
        std::fs::write(roster, r#"{"slots": 5}"#).unwrap();

        f.at(31);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );
        assert!(f.pending("chat").is_empty());
        let now = f.now + Duration::from_secs(31);
        assert!(
            f.tick_ms(31_000).is_none_or(|deadline| deadline > now),
            "no deadline the thread cannot act on"
        );
    }

    #[test]
    fn a_failed_refresh_backs_off() {
        let start = Instant::now();
        let tracking = Tracking {
            refresh_due: true,
            last_refresh: Some(start),
            backoff_until: Some(start + REFRESH_BACKOFF),
            ..Tracking::default()
        };
        assert_eq!(
            tracking.refresh_deadline(start + Duration::from_secs(1)),
            Some(start + REFRESH_BACKOFF)
        );
        let idle = Tracking {
            missions: HashMap::from([(
                "m".to_owned(),
                Tracked {
                    phase: MissionPhase::Working,
                    quiet_since: Some(start),
                    crashed: Default::default(),
                },
            )]),
            last_refresh: Some(start + SETTLE_WINDOW - Duration::from_millis(100)),
            ..Tracking::default()
        };
        assert_eq!(
            idle.refresh_deadline(start + SETTLE_WINDOW),
            Some(start + SETTLE_WINDOW + Duration::from_millis(150))
        );
    }

    #[test]
    fn no_report_and_unlinked_missions_queue_nothing() {
        let f = Fixture::new();
        let quiet = f.mission(Some("chat"), true);
        let unlinked = f.mission(None, false);
        f.at(0);
        f.at(31);
        assert_eq!(f.core.reports.phase(&quiet.id), Some(MissionPhase::Settled));
        assert_eq!(
            f.core.reports.phase(&unlinked.id),
            Some(MissionPhase::Settled)
        );
        assert!(f.pending("chat").is_empty());
    }

    #[test]
    fn linking_by_hand_reports_once_unless_the_chat_linked_it_itself() {
        let f = Fixture::new();
        let mission = f.mission(None, false);
        f.at(0);

        f.link(&mission, "chat", Some("chat"));
        assert!(
            f.pending("chat").is_empty(),
            "the chat's own agent ran the command"
        );

        f.link(&mission, "other", None);
        assert_eq!(
            f.pending("other"),
            [(mission.id.clone(), ReportKind::Linked)]
        );
        f.link(&mission, "other", None);
        assert_eq!(
            f.pending("other").len(),
            1,
            "relinking the same chat adds nothing"
        );

        f.link(&mission, "chat", Some("other"));
        assert!(
            f.pending("other").is_empty(),
            "moving the link drops the old chat's report"
        );
        assert_eq!(
            f.pending("chat"),
            [(mission.id.clone(), ReportKind::Linked)]
        );

        crate::ops::mission::link::mission_unlink(&f.core, &mission.id).unwrap();
        assert!(
            f.pending("chat").is_empty(),
            "unlinking drops a waiting report"
        );
    }

    #[test]
    fn an_archived_chat_receives_nothing() {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.at(0);
        f.at(31);
        assert_eq!(f.pending("chat").len(), 1);

        let conn = f.core.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'stopped' WHERE id = 'chat'",
            [],
        )
        .unwrap();
        repo::session::archive(&conn, "chat", chrono::Utc::now()).unwrap();
        drop(conn);
        f.at(32);
        assert!(f.pending("chat").is_empty());
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Settled)
        );
    }
}
