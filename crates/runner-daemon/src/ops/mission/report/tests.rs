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
    let interrupted = Member {
        interrupted: true,
        ..idle
    };
    let settled = Some(SETTLE_WINDOW);
    let cases = [
        (running(vec![waiting, interrupted]), settled, NeedsYou),
        (running(vec![failed, interrupted]), settled, Failed),
        (running(vec![interrupted, working]), settled, Interrupted),
        (running(vec![interrupted, idle]), settled, Interrupted),
        (
            running(vec![
                Member {
                    lifecycle: Lifecycle::Stopped,
                    ..interrupted
                },
                idle,
            ]),
            settled,
            Settled,
        ),
        (
            PhaseInput {
                running: false,
                ..running(vec![interrupted])
            },
            settled,
            Ended,
        ),
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
    assert!(
        !quiet(&running(vec![interrupted, idle])),
        "an interrupted mission cannot settle"
    );
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
            line(&mission, MissionPhase::Interrupted, ReportKind::State).unwrap(),
            "[runner] Mission 01M6DD9 \"1025 — Remote chat switch\" was interrupted by a Runner restart. Read it with runner mission show 01M6DD9."
        );
    assert!(
        line(&mission, MissionPhase::Interrupted, ReportKind::Linked)
            .unwrap()
            .contains("linked to this chat; it was interrupted by a Runner restart. Read it")
    );
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

    fn log(&self, mission: &Mission) -> runner_core::EventLog {
        runner_core::EventLog::open(&runner_core::event_log::mission_dir(
            &self.core.app_data_dir,
            Some(&self.crew),
            &mission.id,
        ))
        .unwrap()
    }

    /// The member reports a turn starting or ending, as its forwarder would.
    fn status(&self, mission: &Mission, state: &str, source: &str) {
        self.status_with(
            mission,
            serde_json::json!({"state": state, "source": source}),
        );
    }

    /// The member waits for an approval, which its forwarder publishes as busy.
    fn waits_for_approval(&self, mission: &Mission) {
        self.status_with(
            mission,
            serde_json::json!({"state": "busy", "source": "hook", "status": {
                "lifecycle": "running",
                "observation": {"activity": "idle", "source": "hook", "outcome": null,
                    "interactions": [{"id": "w", "reason": "approval", "owners": [], "since": 0}]},
                "exit_code": null, "error_since": null, "unread_since": null,
            }}),
        );
    }

    fn status_with(&self, mission: &Mission, payload: serde_json::Value) {
        self.status_from(mission, "worker", payload);
    }

    fn status_from(&self, mission: &Mission, handle: &str, payload: serde_json::Value) {
        self.log(mission)
            .append(EventDraft::signal(
                Some(&self.crew),
                &mission.id,
                handle,
                SignalType::new("session_status"),
                payload,
            ))
            .unwrap();
        if let Some(router) = self.core.routers.get(&mission.id) {
            let (entries, _) = self.log(mission).read_from_lossy(0).unwrap();
            let last = entries.last().unwrap();
            router.handle_event(&last.event, last.next_offset);
        }
    }

    fn marks(&self, mission: &Mission) -> usize {
        let (entries, _) = self.log(mission).read_from_lossy(0).unwrap();
        entries
            .iter()
            .filter(|entry| {
                entry.event.signal_type.as_ref().map(SignalType::as_str)
                    == Some(crate::router::SESSION_INTERRUPTED)
            })
            .count()
    }

    /// runnerd stops: every running member is stamped for launch resume and killed.
    fn quit(&self) {
        self.core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET status = 'stopped', resume_on_launch = 1,
                            stopped_at = ?1
                      WHERE mission_id IS NOT NULL AND status = 'running'",
                [chrono::Utc::now().to_rfc3339()],
            )
            .unwrap();
    }

    /// The next runnerd boots: it marks interrupted members, mounts the routers and
    /// starts reporting, before launch resume brings the members back.
    fn boot(&self, missions: &[&Mission]) {
        *self.core.reports.tracking.lock().unwrap() = Tracking::default();
        *self.core.reports.delivery.lock().unwrap() = Delivery::default();
        let restarts = crate::ops::mission::interrupt::mark_on_boot(&self.core).unwrap();
        self.core.reports.restarted(restarts);
        for mission in missions {
            self.core.routers.unregister(&mission.id);
            self.core.buses.unmount(&mission.id);
            crate::daemon::block_on(crate::ops::mission::ensure_mission_router_mounted(
                &self.core,
                &mission.id,
            ))
            .unwrap();
        }
    }

    /// A fixture whose crew has a second slot, `helper`.
    fn two_members() -> Self {
        let f = Self::new();
        crate::test_support::insert_test_slot(
            &f.core.db.get().unwrap(),
            "slot-2",
            &f.crew,
            "role",
            "helper",
            1,
            false,
        );
        f
    }

    /// The mission's `helper` member, running and idle.
    fn helper(&self, mission: &Mission) -> String {
        let helper = format!("{}-helper", mission.id);
        let mut row = test_session_row(&helper, SessionStatus::Running);
        row.agent_runtime = Some("codex".into());
        row.mission_id = Some(mission.id.clone());
        row.role_id = Some("role".into());
        row.slot_id = Some("slot-2".into());
        insert_session_row(&self.core.db.get().unwrap(), &row).unwrap();
        self.idle(&helper);
        helper
    }

    fn idle(&self, session: &str) {
        self.core
            .sessions
            .test_running_activity(session, SessionActivityState::Idle);
    }

    /// Launch resume brings one member back; it is Resuming until its agent is up.
    fn resume_one(&self, session: &str) {
        self.core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET status = 'running', resume_on_launch = 0,
                            stopped_at = NULL
                      WHERE id = ?1",
                [session],
            )
            .unwrap();
        self.core.sessions.test_resuming(session);
    }

    fn resume(&self, missions: &[&Mission]) {
        self.core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET status = 'running', resume_on_launch = 0,
                            stopped_at = NULL
                      WHERE mission_id IS NOT NULL",
                [],
            )
            .unwrap();
        for mission in missions {
            self.activity(mission, SessionActivityState::Idle);
        }
    }
}

#[test]
fn a_restart_mid_turn_reports_interrupted_once_and_never_settles() {
    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    f.status(&mission, "busy", "hook");
    f.quit();
    f.boot(&[&mission]);
    assert_eq!(f.marks(&mission), 1);
    f.at(0);
    assert_eq!(f.core.reports.phase(&mission.id), Some(MissionPhase::Ended));
    assert!(f.pending("chat").is_empty());

    f.resume(&[&mission]);
    f.at(1);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted)
    );
    assert_eq!(f.pending("chat"), [(mission.id.clone(), ReportKind::State)]);
    f.at(40);
    f.at(120);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted)
    );
    assert_eq!(f.pending("chat").len(), 1, "no Settled report");
    let summary = crate::daemon::block_on(crate::ops::mission::mission_list_summary_impl(
        &f.core, None,
    ))
    .unwrap();
    assert!(summary[0].session_statuses[0].1.interrupted_since.is_some());
    let shown = crate::ops::mission::feed::mission_status(&f.core, &mission.id).unwrap();
    assert_eq!(shown.phase, Some(MissionPhase::Interrupted));
    assert!(shown.interrupted_at.is_some());
    assert_eq!(
        shown.latest_session_status_by_handle["worker"].state,
        "interrupted"
    );

    f.quit();
    f.boot(&[&mission]);
    assert_eq!(f.marks(&mission), 1, "a second restart adds no second mark");
    f.at(200);
    f.resume(&[&mission]);
    f.at(201);
    f.at(300);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted)
    );
    assert!(f.pending("chat").is_empty(), "nor a second report");

    f.status(&mission, "busy", "input-submit");
    f.at(301);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Working)
    );
    f.status(&mission, "idle", "hook");
    f.at(302);
    f.at(340);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    assert_eq!(f.pending("chat"), [(mission.id.clone(), ReportKind::State)]);
}

#[test]
fn another_members_turn_during_a_staggered_resume_does_not_report_an_old_interruption_again() {
    let f = Fixture::two_members();
    let mission = f.mission(Some("chat"), false);
    let helper = f.helper(&mission);
    f.status(&mission, "busy", "hook");
    f.status_from(
        &mission,
        "helper",
        serde_json::json!({"state": "idle", "source": "hook"}),
    );
    f.quit();
    f.boot(&[&mission]);
    f.resume(&[&mission]);
    f.at(0);
    assert_eq!(f.pending("chat").len(), 1);

    f.quit();
    f.boot(&[&mission]);
    assert_eq!(f.marks(&mission), 1);
    f.at(100);
    f.resume_one(&helper);
    f.idle(&helper);
    f.status_from(
        &mission,
        "helper",
        serde_json::json!({"state": "busy", "source": "input-submit"}),
    );
    f.core
        .sessions
        .test_running_activity(&helper, SessionActivityState::Busy);
    f.at(101);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Working)
    );
    let worker = format!("{}-member", mission.id);
    f.resume_one(&worker);
    f.idle(&worker);
    f.at(102);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted)
    );
    assert!(
        f.pending("chat").is_empty(),
        "the worker never ran a turn, so its interruption is old news"
    );
}

#[test]
fn a_settled_mission_stays_settled_while_its_members_resume_one_at_a_time() {
    let f = Fixture::two_members();
    let mission = f.mission(Some("chat"), false);
    let helper = f.helper(&mission);
    let worker = format!("{}-member", mission.id);
    f.status(&mission, "idle", "hook");
    f.status_from(
        &mission,
        "helper",
        serde_json::json!({"state": "idle", "source": "hook"}),
    );
    f.core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET status = 'stopped', resume_on_launch = 1, stopped_at = ?1
                  WHERE mission_id = ?2",
            [
                (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339(),
                mission.id.clone(),
            ],
        )
        .unwrap();
    f.boot(&[&mission]);
    f.at(0);
    f.resume_one(&worker);
    f.idle(&worker);
    f.at(1);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    f.resume_one(&helper);
    f.at(2);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Working),
        "a member still resuming keeps it Working"
    );
    f.idle(&helper);
    f.at(3);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    f.at(60);
    f.at(120);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    assert!(f.pending("chat").is_empty(), "nobody ran a turn");
}

#[test]
fn a_baseline_that_already_sees_the_interruption_still_reports_it() {
    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    f.status(&mission, "busy", "hook");
    f.quit();
    f.boot(&[&mission]);
    f.resume(&[&mission]);
    f.at(0);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted)
    );
    assert_eq!(f.pending("chat"), [(mission.id.clone(), ReportKind::State)]);
}

#[test]
fn a_restart_of_an_idle_or_settled_mission_marks_and_reports_nothing() {
    let f = Fixture::new();
    let settled = f.mission(Some("chat"), false);
    let idle = f.mission(Some("other"), false);
    f.status(&settled, "idle", "hook");
    f.status(&idle, "busy", "hook");
    f.status(&idle, "idle", "hook");
    let stop = |mission: &Mission, after: i64| {
        f.core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET status = 'stopped', resume_on_launch = 1, stopped_at = ?1
                      WHERE mission_id = ?2",
                [
                    (chrono::Utc::now() + chrono::Duration::seconds(after)).to_rfc3339(),
                    mission.id.clone(),
                ],
            )
            .unwrap();
    };
    stop(&settled, 60);
    stop(&idle, 0);
    f.boot(&[&settled, &idle]);
    assert_eq!(f.marks(&settled) + f.marks(&idle), 0);
    f.at(0);
    f.resume(&[&settled, &idle]);
    f.at(1);
    assert_eq!(
        f.core.reports.phase(&settled.id),
        Some(MissionPhase::Settled),
        "a mission settled before the restart is Settled again once its members resume"
    );
    assert_eq!(f.core.reports.phase(&idle.id), Some(MissionPhase::Working));
    f.at(40);
    for mission in [&settled, &idle] {
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Settled)
        );
    }
    assert!(
        f.pending("chat").is_empty(),
        "a mission settled before the restart stays quiet"
    );
    assert_eq!(
        f.pending("other").len(),
        1,
        "one that had not settled yet settles as before"
    );
}

#[test]
fn a_member_waiting_for_input_is_not_interrupted_and_its_wait_is_no_new_turn() {
    let f = Fixture::new();
    let waiting = f.mission(Some("chat"), false);
    f.waits_for_approval(&waiting);
    f.quit();
    f.boot(&[&waiting]);
    assert_eq!(f.marks(&waiting), 0, "waiting on input is not a lost turn");

    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    f.status(&mission, "busy", "hook");
    f.quit();
    f.boot(&[&mission]);
    f.resume(&[&mission]);
    f.waits_for_approval(&mission);
    let (entries, _) = f.log(&mission).read_from_lossy(0).unwrap();
    assert!(
        crate::router::Interruptions::from_entries(&entries)
            .get("worker")
            .is_some(),
        "a wait after resume does not clear the mark"
    );
}

#[test]
fn a_new_turn_after_the_restart_of_a_settled_mission_still_reports_settled() {
    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    f.status(&mission, "idle", "hook");
    f.core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET status = 'stopped', resume_on_launch = 1, stopped_at = ?1
                  WHERE mission_id = ?2",
            [
                (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339(),
                mission.id.clone(),
            ],
        )
        .unwrap();
    f.boot(&[&mission]);
    f.at(0);
    f.resume(&[&mission]);
    f.at(1);
    f.status(&mission, "busy", "input-submit");
    f.activity(&mission, SessionActivityState::Busy);
    f.at(5);
    f.status(&mission, "idle", "hook");
    f.activity(&mission, SessionActivityState::Idle);
    f.at(6);
    f.at(40);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    assert_eq!(
        f.pending("chat"),
        [(mission.id.clone(), ReportKind::State)],
        "the new turn's completion is news"
    );
}

#[test]
fn stopping_the_mission_clears_its_marks_but_a_failed_stop_keeps_them() {
    let interrupted_fixture = || {
        let f = Fixture::new();
        let mission = f.mission(Some("chat"), false);
        f.status(&mission, "busy", "hook");
        f.quit();
        f.boot(&[&mission]);
        f.resume(&[&mission]);
        (f, mission)
    };
    let interrupted = |f: &Fixture, mission: &Mission| {
        let (entries, _) = f.log(mission).read_from_lossy(0).unwrap();
        crate::router::Interruptions::from_entries(&entries)
            .get("worker")
            .is_some()
    };

    let (f, mission) = interrupted_fixture();
    let member = format!("{}-member", mission.id);
    f.core.sessions.test_attach_live(&member, &mission.id);
    assert!(crate::ops::session::session_kill(&f.core, &member).is_err());
    assert!(
        crate::daemon::block_on(crate::ops::mission::mission_stop_impl(
            &f.core,
            mission.id.clone()
        ))
        .is_err()
    );
    assert!(
        interrupted(&f, &mission),
        "a member a stop failed to end keeps its mark"
    );

    let (f, mission) = interrupted_fixture();
    crate::daemon::block_on(crate::ops::mission::mission_stop_impl(
        &f.core,
        mission.id.clone(),
    ))
    .unwrap();
    assert!(
        !interrupted(&f, &mission),
        "Stop all clears it, so Resume all does not bring it back"
    );
}

#[test]
fn stopping_an_interrupted_member_clears_its_mark() {
    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    f.status(&mission, "busy", "hook");
    f.quit();
    f.boot(&[&mission]);
    f.resume(&[&mission]);
    f.status(&mission, "busy", "spawn");
    f.at(0);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Interrupted),
        "starting a process is not a turn"
    );
    crate::ops::session::session_kill(&f.core, &format!("{}-member", mission.id)).unwrap();
    let (entries, _) = f.log(&mission).read_from_lossy(0).unwrap();
    assert!(crate::router::Interruptions::from_entries(&entries)
        .get("worker")
        .is_none());
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
fn background_work_holds_the_settle_until_the_woken_turn_ends() {
    use crate::session::state::agent::AgentEvent;
    let waiting = Member {
        idle: true,
        background: true,
        ..member(Lifecycle::Running)
    };
    assert!(!quiet(&running(vec![waiting])));
    assert_eq!(
        derive(&running(vec![waiting]), Some(Duration::MAX)),
        MissionPhase::Working
    );

    let f = Fixture::new();
    let mission = f.mission(Some("chat"), false);
    let turn = |events| {
        f.core.sessions.test_agent_event(
            &format!("{}-member", mission.id),
            AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events,
            },
        )
    };
    let stop = |pending| {
        vec![
            AgentEvent::TurnEnded {
                outcome: runner_core::protocol::status::TurnOutcome::Completed,
            },
            AgentEvent::BackgroundWork { pending },
        ]
    };
    f.at(0);
    turn(vec![AgentEvent::TurnStarted]);
    f.at(1);
    turn(stop(true));
    f.at(2);
    for at in [40, 80] {
        f.at(at);
        assert_eq!(
            f.core.reports.phase(&mission.id),
            Some(MissionPhase::Working)
        );
    }
    assert!(f.pending("chat").is_empty());

    turn(vec![AgentEvent::TurnStarted]);
    f.at(90);
    turn(stop(false));
    f.at(95);
    f.at(124);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Working)
    );
    assert!(f.pending("chat").is_empty());
    f.at(125);
    assert_eq!(
        f.core.reports.phase(&mission.id),
        Some(MissionPhase::Settled)
    );
    assert_eq!(f.pending("chat"), [(mission.id.clone(), ReportKind::State)]);
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
