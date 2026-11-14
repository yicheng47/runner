//! Project-local, read-only Mission Control snapshot assembly.

use std::fs;
use std::path::{Path, PathBuf};

use runner_core::protocol::mission_control::{
    MissionControlCard, MissionControlEvidence, MissionControlFieldProvenance, MissionControlLane,
    MissionControlMission, MissionControlSnapshot, MissionControlSourceHealth,
    MissionControlSourceKind,
};

use crate::{error::Result, ops::project, AppCore};

#[derive(Debug)]
struct Ticket {
    id: String,
    summary: String,
    severity: Option<String>,
    blockers: Vec<String>,
    ready: bool,
    closed: bool,
    verify_land: bool,
    path: PathBuf,
}

#[derive(Debug, Default)]
struct Triage {
    state: Option<String>,
    blockers: Vec<String>,
    verify_land: bool,
    stale: bool,
    path: PathBuf,
}

pub fn snapshot(state: &AppCore, project_id: &str) -> Result<MissionControlSnapshot> {
    // Release the project lookup connection before reading mission summaries:
    // an in-memory test pool (and a busy daemon pool) may have no spare
    // connection for the nested session-status query.
    let project = {
        let conn = state.db.get()?;
        project::get(&conn, project_id)?
    };
    let root = PathBuf::from(&project.cwd);
    let mut health = Vec::new();
    let tickets = tickets(&root, &mut health);
    let triage = triage(&root, &mut health);
    let missions = crate::daemon::block_on(super::mission_list_summary_impl(state, None))?;
    let cards = tickets
        .into_iter()
        .map(|ticket| {
            let triage = triage.get(&ticket.id).filter(|triage| !triage.stale);
            let mission = missions.iter().find(|mission| {
                mission.mission.project_id.as_deref() == Some(project_id)
                    && mission_links_ticket(&mission.mission.title, &ticket.id)
            });
            let mission_status = mission.and_then(|mission| {
                match crate::ops::mission::feed::mission_status(state, &mission.mission.id) {
                    Ok(status) => Some(status),
                    Err(_) => {
                        health.push(MissionControlSourceHealth {
                            kind: MissionControlSourceKind::Runner,
                            path: format!("mission:{}", mission.mission.id),
                            warning: "mission feed unavailable".into(),
                        });
                        None
                    }
                }
            });
            let pending_question = mission_status
                .as_ref()
                .and_then(|status| status.pending_asks.first())
                .map(|ask| ask.prompt.clone());
            let active_slots = mission
                .map(|mission| {
                    mission
                        .session_statuses
                        .iter()
                        .filter(|(_, status)| {
                            status.lifecycle == crate::session::status::Lifecycle::Running
                        })
                        .count()
                })
                .unwrap_or_default();
            let mission = mission.map(|mission| MissionControlMission {
                id: mission.mission.id.clone(),
                title: mission.mission.title.clone(),
                active_slots,
                pending_question: pending_question.clone(),
                newest_handoff: newest_handoff(state, &mission.mission.id),
                // Branch is not persisted by Runner. A worktree is the
                // mission's supplied cwd and is the authoritative fact here.
                branch: None,
                worktree: mission.mission.cwd.clone(),
            });
            let triage_blocked = triage.is_some_and(|triage| !triage.blockers.is_empty());
            let triage_verify = triage.is_some_and(|triage| triage.verify_land);
            let mut blockers = ticket.blockers.clone();
            if let Some(triage) = triage {
                blockers.extend(triage.blockers.iter().cloned());
            }
            let lane = lane(
                ticket.closed,
                pending_question.is_some(),
                active_slots > 0,
                ticket.verify_land || triage_verify,
                !ticket.blockers.is_empty() || triage_blocked,
                ticket.ready,
            );
            let mut evidence = vec![MissionControlEvidence {
                kind: MissionControlSourceKind::Ticket,
                path: ticket.path.display().to_string(),
            }];
            if let Some(triage) = triage {
                evidence.push(MissionControlEvidence {
                    kind: MissionControlSourceKind::Triage,
                    path: triage.path.display().to_string(),
                });
            }
            if let Some(mission) = &mission {
                evidence.push(MissionControlEvidence {
                    kind: MissionControlSourceKind::Runner,
                    path: format!("mission:{}", mission.id),
                });
            }
            let mut field_provenance = vec![
                provenance("ticket_id", &evidence[0]),
                provenance("summary", &evidence[0]),
            ];
            if ticket.severity.is_some() {
                field_provenance.push(provenance("severity", &evidence[0]));
            }
            if !ticket.blockers.is_empty() {
                field_provenance.push(provenance("blockers", &evidence[0]));
            }
            if let Some(triage) = triage {
                let triage_evidence = evidence
                    .iter()
                    .find(|evidence| evidence.kind == MissionControlSourceKind::Triage)
                    .expect("triage evidence was added");
                if triage.state.is_some() {
                    field_provenance.push(provenance("triage_state", triage_evidence));
                }
                if !triage.blockers.is_empty() {
                    field_provenance.push(provenance("blockers", triage_evidence));
                }
            }
            if let Some(runner_evidence) = evidence
                .iter()
                .find(|evidence| evidence.kind == MissionControlSourceKind::Runner)
            {
                field_provenance.push(provenance("mission", runner_evidence));
                field_provenance.push(provenance("active_slots", runner_evidence));
                field_provenance.push(provenance("pending_question", runner_evidence));
                field_provenance.push(provenance("newest_handoff", runner_evidence));
                field_provenance.push(provenance("branch", runner_evidence));
                field_provenance.push(provenance("worktree", runner_evidence));
            }
            let lane_evidence = match lane {
                MissionControlLane::Closed
                | MissionControlLane::Ready
                | MissionControlLane::Backlog => &evidence[0],
                MissionControlLane::NeedsYou | MissionControlLane::Running => evidence
                    .iter()
                    .find(|evidence| evidence.kind == MissionControlSourceKind::Runner)
                    .expect("mission lanes have Runner evidence"),
                MissionControlLane::VerifyLand if ticket.verify_land => &evidence[0],
                MissionControlLane::Blocked if !ticket.blockers.is_empty() => &evidence[0],
                MissionControlLane::VerifyLand | MissionControlLane::Blocked => evidence
                    .iter()
                    .find(|evidence| evidence.kind == MissionControlSourceKind::Triage)
                    .expect("triage evidence produced the derived lane"),
            };
            field_provenance.push(provenance("lane", lane_evidence));
            MissionControlCard {
                ticket_id: ticket.id,
                summary: ticket.summary,
                severity: ticket.severity,
                blockers,
                triage_state: triage.and_then(|triage| triage.state.clone()),
                lane,
                mission,
                provenance: field_provenance,
                evidence,
            }
        })
        .collect();
    Ok(MissionControlSnapshot {
        project_id: project.id,
        project_path: project.cwd,
        cards,
        health,
    })
}

fn lane(
    closed: bool,
    pending_question: bool,
    active: bool,
    verify_land: bool,
    blocked: bool,
    ready: bool,
) -> MissionControlLane {
    if closed {
        MissionControlLane::Closed
    } else if pending_question {
        MissionControlLane::NeedsYou
    } else if active {
        MissionControlLane::Running
    } else if verify_land {
        MissionControlLane::VerifyLand
    } else if blocked {
        MissionControlLane::Blocked
    } else if ready {
        MissionControlLane::Ready
    } else {
        MissionControlLane::Backlog
    }
}

fn tickets(root: &Path, health: &mut Vec<MissionControlSourceHealth>) -> Vec<Ticket> {
    let path = root.join("issues");
    let Ok(entries) = fs::read_dir(&path) else {
        health.push(warning(
            MissionControlSourceKind::Ticket,
            path,
            "ticket directory unavailable",
        ));
        return Vec::new();
    };
    let readme = path.join("README.md");
    if fs::read_to_string(&readme).is_err() {
        health.push(warning(
            MissionControlSourceKind::Ticket,
            readme,
            "ticket README unavailable",
        ));
    }
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let file = path.file_name()?.to_str()?;
            (file.starts_with("CR-") && file.ends_with(".md")).then_some(path)
        })
        .collect();
    if let Ok(entries) = fs::read_dir(path.join("archive")) {
        paths.extend(entries.filter_map(|entry| entry.ok()).filter_map(|entry| {
            let path = entry.path();
            let file = path.file_name()?.to_str()?;
            (file.starts_with("CR-") && file.ends_with(".md")).then_some(path)
        }));
    }
    paths
        .into_iter()
        .filter_map(|path| {
            parse_ticket(
                &path,
                path.parent()
                    .is_some_and(|parent| parent.ends_with("archive")),
            )
            .map_err(|warning| health.push(warning))
            .ok()
        })
        .collect()
}

fn triage(
    root: &Path,
    health: &mut Vec<MissionControlSourceHealth>,
) -> std::collections::HashMap<String, Triage> {
    let path = root.join(".crew/triage");
    let Ok(entries) = fs::read_dir(&path) else {
        health.push(warning(
            MissionControlSourceKind::Triage,
            path,
            "triage directory unavailable",
        ));
        return Default::default();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|x| x.to_str()) == Some("md")).then_some(path)
        })
        .filter_map(|path| match parse_triage(&path) {
            Ok((id, triage)) => {
                if triage.stale {
                    health.push(warning(
                        MissionControlSourceKind::Triage,
                        &triage.path,
                        "triage source marked stale",
                    ));
                }
                Some((id, triage))
            }
            Err(warning) => {
                health.push(warning);
                None
            }
        })
        .collect()
}

fn parse_ticket(
    path: &Path,
    archived: bool,
) -> std::result::Result<Ticket, MissionControlSourceHealth> {
    let text = fs::read_to_string(path)
        .map_err(|_| warning(MissionControlSourceKind::Ticket, path, "ticket unreadable"))?;
    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with("CR-"))
        .ok_or_else(|| warning(MissionControlSourceKind::Ticket, path, "ticket id missing"))?
        .to_owned();
    let summary = text
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or(&id)
        .trim_start_matches(&format!("{id}: "))
        .to_owned();
    if summary.is_empty() {
        return Err(warning(
            MissionControlSourceKind::Ticket,
            path,
            "ticket summary missing",
        ));
    }
    Ok(Ticket {
        id,
        summary,
        severity: field(&text, "severity"),
        blockers: fields(&text, "blocker"),
        ready: state_is(&text, "ready"),
        closed: archived || state_is(&text, "closed"),
        verify_land: verify_or_land(&text),
        path: path.to_owned(),
    })
}

fn parse_triage(path: &Path) -> std::result::Result<(String, Triage), MissionControlSourceHealth> {
    let text = fs::read_to_string(path)
        .map_err(|_| warning(MissionControlSourceKind::Triage, path, "triage unreadable"))?;
    let id = text
        .split_whitespace()
        .find(|word| word.starts_with("CR-"))
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .filter(|name| name.starts_with("CR-"))
        })
        .ok_or_else(|| {
            warning(
                MissionControlSourceKind::Triage,
                path,
                "triage ticket id missing",
            )
        })?
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .to_owned();
    Ok((
        id,
        Triage {
            state: field(&text, "state"),
            blockers: fields(&text, "blocker"),
            verify_land: verify_or_land(&text),
            stale: bool_field(&text, "stale") || state_is(&text, "stale"),
            path: path.to_owned(),
        },
    ))
}

fn field(text: &str, name: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{name}:")))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
fn fields(text: &str, name: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix(&format!("{name}:")))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
fn bool_field(text: &str, name: &str) -> bool {
    field(text, name).is_some_and(|value| value.eq_ignore_ascii_case("true"))
}
fn state_is(text: &str, state: &str) -> bool {
    field(text, "state")
        .or_else(|| field(text, "status"))
        .is_some_and(|value| value.eq_ignore_ascii_case(state))
}

fn verify_or_land(text: &str) -> bool {
    ["verify", "verification", "review", "land", "landing"]
        .into_iter()
        .any(|state| state_is(text, state))
}

fn mission_links_ticket(title: &str, ticket_id: &str) -> bool {
    title
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.eq_ignore_ascii_case(ticket_id))
}

fn provenance(field: &str, evidence: &MissionControlEvidence) -> MissionControlFieldProvenance {
    MissionControlFieldProvenance {
        field: field.into(),
        evidence: evidence.clone(),
    }
}

fn newest_handoff(state: &AppCore, mission_id: &str) -> Option<String> {
    let feed = crate::ops::mission::feed::mission_feed(
        state,
        runner_core::protocol::MissionFeedArgs {
            mission_id: mission_id.into(),
            order: runner_core::protocol::MissionFeedOrder::NewestFirst,
            limit: Some(50),
            since_offset: None,
        },
    )
    .ok()?;
    feed.events.into_iter().find_map(|entry| {
        let text = entry.event.payload.get("text")?.as_str()?.trim();
        text.to_ascii_lowercase()
            .contains("handoff")
            .then(|| text.chars().take(160).collect())
    })
}
fn warning(
    kind: MissionControlSourceKind,
    path: impl AsRef<Path>,
    warning: &str,
) -> MissionControlSourceHealth {
    MissionControlSourceHealth {
        kind,
        path: path.as_ref().display().to_string(),
        warning: warning.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_precedence_is_exact() {
        assert_eq!(
            lane(true, true, true, true, true, true),
            MissionControlLane::Closed
        );
        assert_eq!(
            lane(false, true, true, true, true, true),
            MissionControlLane::NeedsYou
        );
        assert_eq!(
            lane(false, false, true, true, true, true),
            MissionControlLane::Running
        );
        assert_eq!(
            lane(false, false, false, true, true, true),
            MissionControlLane::VerifyLand
        );
        assert_eq!(
            lane(false, false, false, false, true, true),
            MissionControlLane::Blocked
        );
        assert_eq!(
            lane(false, false, false, false, false, true),
            MissionControlLane::Ready
        );
    }

    #[test]
    fn ticket_state_is_explicit_and_archives_are_closed() {
        let root = tempfile::tempdir().unwrap();
        let ticket = root.path().join("CR-1.md");
        fs::write(&ticket, "# CR-1: Check the board\nstatus: ready\n").unwrap();
        let ticket = parse_ticket(&ticket, false).unwrap();
        assert!(ticket.ready);
        assert!(!ticket.closed);

        let archived = root.path().join("CR-2.md");
        fs::write(&archived, "# CR-2: Archived ticket\n").unwrap();
        assert!(parse_ticket(&archived, true).unwrap().closed);
    }

    #[test]
    fn verification_review_and_landing_are_explicit_verify_land_evidence() {
        let root = tempfile::tempdir().unwrap();
        for state in ["verification", "review", "landing"] {
            let path = root.path().join(format!("CR-{state}.md"));
            fs::write(&path, format!("# CR-1: {state}\nstatus: {state}\n")).unwrap();
            assert!(parse_ticket(&path, false).unwrap().verify_land, "{state}");
        }

        let triage_path = root.path().join("CR-1-triage.md");
        fs::write(&triage_path, "CR-1\nstate: review\n").unwrap();
        assert!(parse_triage(&triage_path).unwrap().1.verify_land);
    }

    #[test]
    fn mission_linkage_matches_the_exact_ticket_identifier_token() {
        assert!(mission_links_ticket("CR-1 review", "CR-1"));
        assert!(mission_links_ticket("fix: cr-1 / verification", "CR-1"));
        assert!(!mission_links_ticket("CR-10 review", "CR-1"));
        assert!(!mission_links_ticket("CR-1A review", "CR-1"));
    }

    #[test]
    fn malformed_or_missing_sources_are_warnings_not_snapshot_failures() {
        let root = tempfile::tempdir().unwrap();
        let mut health = Vec::new();
        assert!(tickets(root.path(), &mut health).is_empty());
        assert_eq!(health.len(), 1);

        let issues = root.path().join("issues");
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("CR-1.md"), "# \n").unwrap();
        assert!(tickets(root.path(), &mut health).is_empty());
        assert!(health
            .iter()
            .any(|warning| warning.warning == "ticket summary missing"));
    }

    #[test]
    fn stale_triage_remains_visible_with_a_source_scoped_warning() {
        let root = tempfile::tempdir().unwrap();
        let triage_dir = root.path().join(".crew/triage");
        fs::create_dir_all(&triage_dir).unwrap();
        let triage_path = triage_dir.join("CR-1.md");
        fs::write(&triage_path, "CR-1\nstate: stale\n").unwrap();
        let mut health = Vec::new();
        let records = triage(root.path(), &mut health);
        assert!(records["CR-1"].stale);
        assert_eq!(health[0].warning, "triage source marked stale");
    }

    #[test]
    fn ticket_only_source_retains_summary_and_ticket_fields() {
        let root = tempfile::tempdir().unwrap();
        let issues = root.path().join("issues");
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("README.md"), "# Tickets\n").unwrap();
        fs::write(
            issues.join("CR-1.md"),
            "# CR-1: Read the source\nseverity: high\nblocker: waiting\nstatus: ready\n",
        )
        .unwrap();
        let mut health = Vec::new();
        let records = tickets(root.path(), &mut health);
        assert!(health.is_empty());
        assert_eq!(records.len(), 1);
        let ticket = &records[0];
        assert_eq!(ticket.id, "CR-1");
        assert_eq!(ticket.summary, "Read the source");
        assert_eq!(ticket.severity.as_deref(), Some("high"));
        assert_eq!(ticket.blockers, ["waiting"]);
        assert!(ticket.ready);
    }

    #[test]
    fn stale_triage_is_excluded_from_the_derived_card() {
        let root = tempfile::tempdir().unwrap();
        let triage_dir = root.path().join(".crew/triage");
        fs::create_dir_all(&triage_dir).unwrap();
        let path = triage_dir.join("CR-1.md");
        fs::write(&path, "CR-1\nstate: stale\nblocker: obsolete\n").unwrap();
        let mut health = Vec::new();
        let records = triage(root.path(), &mut health);
        assert!(records["CR-1"].stale);
        assert!(health
            .iter()
            .any(|entry| entry.path == path.display().to_string()));
    }

    #[test]
    fn snapshot_joins_a_ticket_only_project_without_mutating_its_sources() {
        let root = tempfile::tempdir().unwrap();
        let issues = root.path().join("issues");
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("README.md"), "# Tickets\n").unwrap();
        let ticket_path = issues.join("CR-1.md");
        let ticket = "# CR-1: Read-only board\nseverity: high\nstatus: ready\n";
        fs::write(&ticket_path, ticket).unwrap();
        let core = crate::test_support::test_core_in(root.path().to_path_buf());
        let project = crate::repo::project::create(
            &core.db.get().unwrap(),
            "Board",
            root.path().to_string_lossy().as_ref(),
        )
        .unwrap();

        let snapshot = snapshot(&core, &project.id).unwrap();

        assert_eq!(snapshot.cards.len(), 1);
        let card = &snapshot.cards[0];
        assert_eq!(card.ticket_id, "CR-1");
        assert_eq!(card.lane, MissionControlLane::Ready);
        assert!(card.mission.is_none());
        assert_eq!(card.evidence[0].path, ticket_path.display().to_string());
        assert!(card.provenance.iter().any(|entry| entry.field == "summary"));
        assert!(card.provenance.iter().any(|entry| entry.field == "lane"));
        assert_eq!(fs::read_to_string(ticket_path).unwrap(), ticket);
    }

    #[test]
    fn ticket_lane_provenance_wins_over_unrelated_triage_evidence() {
        let root = tempfile::tempdir().unwrap();
        let issues = root.path().join("issues");
        let triage = root.path().join(".crew/triage");
        fs::create_dir_all(&triage).unwrap();
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("README.md"), "# Tickets\n").unwrap();
        let ticket_path = issues.join("CR-1.md");
        fs::write(&ticket_path, "# CR-1: Verify me\nstatus: verify\n").unwrap();
        fs::write(triage.join("CR-1.md"), "CR-1\nstate: ready\n").unwrap();
        let core = crate::test_support::test_core_in(root.path().to_path_buf());
        let project = crate::repo::project::create(
            &core.db.get().unwrap(),
            "Board",
            root.path().to_string_lossy().as_ref(),
        )
        .unwrap();

        let card = &snapshot(&core, &project.id).unwrap().cards[0];

        assert_eq!(card.lane, MissionControlLane::VerifyLand);
        assert_eq!(
            card.provenance
                .iter()
                .find(|entry| entry.field == "lane")
                .unwrap()
                .evidence
                .path,
            ticket_path.display().to_string()
        );
    }

    #[test]
    fn genuine_pending_question_sets_needs_you_with_runner_provenance() {
        use runner_core::model::{EventDraft, SignalType};

        let root = tempfile::tempdir().unwrap();
        let issues = root.path().join("issues");
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("README.md"), "# Tickets\n").unwrap();
        fs::write(
            issues.join("CR-1.md"),
            "# CR-1: Await input\nstatus: ready\n",
        )
        .unwrap();
        let core = crate::test_support::test_core_in(root.path().to_path_buf());
        let project = crate::repo::project::create(
            &core.db.get().unwrap(),
            "Board",
            root.path().to_string_lossy().as_ref(),
        )
        .unwrap();
        let mission = crate::ops::mission::start(
            &mut core.db.get().unwrap(),
            root.path(),
            runner_core::protocol::mission::StartMissionInput {
                project_id: Some(project.id.clone()),
                crew_id: None,
                title: "CR-1 review".into(),
                goal_override: None,
                cwd: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission;
        let log_dir = runner_core::event_log::mission_dir(root.path(), None, &mission.id);
        runner_core::event_log::EventLog::open(&log_dir)
            .unwrap()
            .append(EventDraft::signal(
                None,
                mission.id.clone(),
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"prompt": "Approve this?"}),
            ))
            .unwrap();

        let card = &snapshot(&core, &project.id).unwrap().cards[0];

        assert_eq!(card.lane, MissionControlLane::NeedsYou);
        assert_eq!(
            card.mission.as_ref().unwrap().pending_question.as_deref(),
            Some("Approve this?")
        );
        assert_eq!(
            card.provenance
                .iter()
                .find(|entry| entry.field == "lane")
                .unwrap()
                .evidence
                .kind,
            MissionControlSourceKind::Runner
        );
    }

    #[test]
    fn active_mission_session_sets_running_and_retains_worktree() {
        use crate::ops::{crew, role, slot};

        let root = tempfile::tempdir().unwrap();
        let issues = root.path().join("issues");
        fs::create_dir(&issues).unwrap();
        fs::write(issues.join("README.md"), "# Tickets\n").unwrap();
        fs::write(
            issues.join("CR-1.md"),
            "# CR-1: Active work\nstatus: ready\n",
        )
        .unwrap();
        let core = crate::test_support::test_core_in(root.path().to_path_buf());
        let project = crate::repo::project::create(
            &core.db.get().unwrap(),
            "Board",
            root.path().to_string_lossy().as_ref(),
        )
        .unwrap();
        let crew = crew::create(
            &core.db.get().unwrap(),
            runner_core::protocol::crew::CreateCrewInput {
                name: "Board crew".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let role = role::create(
            &core.db.get().unwrap(),
            runner_core::protocol::role::CreateRoleInput {
                handle: "coder".into(),
                display_name: "Coder".into(),
                runtime: crate::model::Runtime::Codex,
                command: "/bin/sh".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: crate::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap();
        let slot = slot::create(
            &mut core.db.get().unwrap(),
            &crew.id,
            &role.id,
            "coder",
            None,
            None,
        )
        .unwrap()
        .slot;
        let mission = crate::ops::mission::start(
            &mut core.db.get().unwrap(),
            root.path(),
            runner_core::protocol::mission::StartMissionInput {
                project_id: Some(project.id.clone()),
                crew_id: Some(crew.id),
                title: "CR-1 implementation".into(),
                goal_override: None,
                cwd: Some(root.path().display().to_string()),
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission;
        core.db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
                     VALUES ('active-slot', ?1, ?2, ?3, 'running', '2026-11-14T00:00:00Z')",
                [&mission.id, &role.id, &slot.id],
            )
            .unwrap();

        let card = &snapshot(&core, &project.id).unwrap().cards[0];

        assert_eq!(card.lane, MissionControlLane::Running);
        assert_eq!(card.mission.as_ref().unwrap().active_slots, 1);
        assert_eq!(
            card.mission.as_ref().unwrap().worktree.as_deref(),
            Some(root.path().to_string_lossy().as_ref())
        );
        assert!(card.provenance.iter().any(|entry| {
            entry.field == "lane" && entry.evidence.kind == MissionControlSourceKind::Runner
        }));
    }
}
