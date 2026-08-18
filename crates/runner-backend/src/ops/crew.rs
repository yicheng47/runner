// Crew CRUD — the top-level container for a team of runners.
//
// Signal-type validation is no longer crew-scoped: the CLI checks
// incoming `runner signal <type>` against the closed
// `runner_core::model::KnownSignalType` enum, so the per-crew column
// + sidecar that used to feed it are gone (feature 20).

use chrono::Utc;
use rusqlite::{params, Connection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Crew, Timestamp},
    repo, AppCore,
};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CreateCrewInput {
    pub name: String,
    pub purpose: Option<String>,
    pub goal: Option<String>,
    /// Optional team-conventions text. Empty after trim → stored as NULL.
    /// Plain Option (not Option<Option>) because create has no "leave
    /// existing" semantic. See #54.
    #[serde(default)]
    pub system_prompt_addendum: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UpdateCrewInput {
    pub name: Option<String>,
    pub purpose: Option<Option<String>>,
    pub goal: Option<Option<String>>,
    /// Outer None = leave existing untouched; outer Some(inner) =
    /// write inner. Inner Some("") / whitespace-only collapses to
    /// NULL.
    pub system_prompt_addendum: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewListItem {
    #[serde(flatten)]
    pub crew: Crew,
    pub runner_count: i64,
    /// Member preview for the Crews list cards: one entry per slot,
    /// in `position` order, carrying just the labels the card pills
    /// need (`@slot_handle` + `runtime-runner_handle`). Sourced
    /// inline so the frontend doesn't N+1 `slot_list` for each crew
    /// on every page load.
    pub members: Vec<CrewMemberPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrewMemberPreview {
    pub slot_handle: String,
    pub runner_handle: String,
    pub runtime: String,
    pub lead: bool,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

pub fn list(conn: &Connection) -> Result<Vec<CrewListItem>> {
    let rows = repo::crew::list_with_runner_count(conn)?;

    // Bulk-fetch slot pills for every crew in a single query so the
    // Crews page renders without an N+1 lookup. Ordered by
    // (crew_id, position) so we can group sequentially.
    let mut members_by_crew: std::collections::HashMap<String, Vec<CrewMemberPreview>> =
        std::collections::HashMap::new();
    for preview in repo::crew::list_member_previews(conn)? {
        members_by_crew
            .entry(preview.crew_id)
            .or_default()
            .push(CrewMemberPreview {
                slot_handle: preview.slot_handle,
                runner_handle: preview.runner_handle,
                runtime: preview.runtime,
                lead: preview.lead,
            });
    }

    Ok(rows
        .into_iter()
        .map(|(crew, runner_count)| {
            let members = members_by_crew.remove(&crew.id).unwrap_or_default();
            CrewListItem {
                crew,
                runner_count,
                members,
            }
        })
        .collect())
}

pub fn get(conn: &Connection, id: &str) -> Result<Crew> {
    repo::crew::get(conn, id)?.ok_or_else(|| Error::msg(format!("crew not found: {id}")))
}

/// Reject `crew.goal` payloads that would push the composed lead
/// launch prompt past `router::runtime::FIRST_TURN_ARGV_MAX_BYTES`
/// once layered with `system_prompt` + roster + coordination block.
/// `mission_start` uses the per-mission `goal_override` when set,
/// else this default; capping at the same `MAX_MISSION_GOAL_BYTES`
/// limit at both layers makes the invariant uniform.
fn validate_crew_goal(goal: Option<&str>) -> Result<()> {
    if let Some(g) = goal {
        if g.len() > crate::ops::mission::MAX_MISSION_GOAL_BYTES {
            return Err(Error::msg(format!(
                "crew goal is {} bytes; max {} ({} KB). Trim the goal text or move \
                 long-form context into the runner brief / per-task messages.",
                g.len(),
                crate::ops::mission::MAX_MISSION_GOAL_BYTES,
                crate::ops::mission::MAX_MISSION_GOAL_BYTES / 1024,
            )));
        }
    }
    Ok(())
}

/// Trim a text-with-default-NULL field. `None`, all-whitespace, or
/// the empty string all collapse to `None` so the column never
/// stores a degenerate "" value. Used for `system_prompt_addendum`;
/// other text fields (purpose / goal) predate this helper and keep
/// their raw-pass-through semantics for now.
fn normalize_addendum(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn create(conn: &Connection, input: CreateCrewInput) -> Result<Crew> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(Error::msg("crew name must not be empty"));
    }
    validate_crew_goal(input.goal.as_deref())?;
    let id = new_id();
    let ts = now();
    let addendum = normalize_addendum(input.system_prompt_addendum);
    repo::crew::insert(
        conn,
        &repo::crew::CrewRow {
            id: id.clone(),
            name: name.to_string(),
            purpose: input.purpose,
            goal: input.goal,
            system_prompt_addendum: addendum,
            created_at: ts,
            updated_at: ts,
        },
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: UpdateCrewInput) -> Result<Crew> {
    let existing = get(conn, id)?;

    let name = match input.name.as_ref() {
        Some(n) => {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return Err(Error::msg("crew name must not be empty"));
            }
            trimmed.to_string()
        }
        None => existing.name,
    };
    let purpose = input.purpose.unwrap_or(existing.purpose);
    let goal = input.goal.unwrap_or(existing.goal);
    validate_crew_goal(goal.as_deref())?;
    let system_prompt_addendum = match input.system_prompt_addendum {
        Some(inner) => normalize_addendum(inner),
        None => existing.system_prompt_addendum,
    };

    repo::crew::update(
        conn,
        &repo::crew::CrewRow {
            id: id.to_string(),
            name,
            purpose,
            goal,
            system_prompt_addendum,
            created_at: existing.created_at,
            updated_at: now(),
        },
    )?;
    get(conn, id)
}

fn non_archived_mission_ids_for_crew(
    conn: &Connection,
    crew_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id
           FROM missions
          WHERE crew_id = ?1
            AND archived_at IS NULL
          ORDER BY started_at ASC",
    )?;
    let ids = stmt
        .query_map(params![crew_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

pub fn delete(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mission_ids = non_archived_mission_ids_for_crew(&tx, id)?;
    if !mission_ids.is_empty() {
        return Err(Error::msg(format!(
            "crew {id} has non-archived missions; archive them before deleting this crew: {}",
            mission_ids.join(", ")
        )));
    }
    tx.execute(
        "DELETE FROM sessions
          WHERE mission_id IN (
              SELECT id FROM missions WHERE crew_id = ?1
          )",
        params![id],
    )?;
    let affected = repo::crew::delete(&tx, id)?;
    if affected == 0 {
        return Err(Error::msg(format!("crew not found: {id}")));
    }
    tx.commit()?;
    Ok(())
}

pub fn crew_list(state: &AppCore) -> Result<Vec<CrewListItem>> {
    let conn = state.db.get()?;
    list(&conn)
}

pub fn crew_get(state: &AppCore, id: &str) -> Result<Crew> {
    let conn = state.db.get()?;
    get(&conn, id)
}

pub fn crew_create(state: &AppCore, input: CreateCrewInput) -> Result<Crew> {
    let conn = state.db.get()?;
    create(&conn, input)
}

pub fn crew_update(state: &AppCore, id: &str, input: UpdateCrewInput) -> Result<Crew> {
    let conn = state.db.get()?;
    update(&conn, id, input)
}

pub fn crew_delete(state: &AppCore, id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    delete(&mut conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use rusqlite::params;

    fn ctx() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    #[test]
    fn create_rejects_goal_over_cap() {
        // Plan 0007: validation at persist time keeps the composed
        // launch prompt under the runtime argv ceiling. crew.goal
        // feeds into the lead's launch prompt at mission_start (the
        // mission_goal event uses goal_override || crew.goal), so
        // the same cap applies here.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let oversized = "Y".repeat(crate::ops::mission::MAX_MISSION_GOAL_BYTES + 1);
        let err = create(
            &conn,
            CreateCrewInput {
                name: "Big".into(),
                goal: Some(oversized),
                ..Default::default()
            },
        )
        .expect_err("oversize crew.goal must be rejected");
        assert!(err.to_string().contains("goal"));
    }

    #[test]
    fn update_rejects_goal_over_cap() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Victim".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let oversized = "Y".repeat(crate::ops::mission::MAX_MISSION_GOAL_BYTES + 1);
        let err = update(
            &conn,
            &crew.id,
            UpdateCrewInput {
                goal: Some(Some(oversized)),
                ..Default::default()
            },
        )
        .expect_err("oversize crew.goal must be rejected on update");
        assert!(err.to_string().contains("goal"));
    }

    #[test]
    fn list_returns_crews_with_runner_counts() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let a = create(
            &conn,
            CreateCrewInput {
                name: "A".into(),
                ..Default::default()
            },
        )
        .unwrap();
        create(
            &conn,
            CreateCrewInput {
                name: "B".into(),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                created_at, updated_at
             ) VALUES ('r1', 'lead', 'Lead', 'shell', 'sh',
                       '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('s1', ?1, 'r1', 'lead', 0, 1, '2026-04-22T00:00:00Z')",
            params![a.id],
        )
        .unwrap();

        let items = list(&conn).unwrap();
        assert_eq!(items.len(), 2);
        let a_item = items.iter().find(|i| i.crew.id == a.id).unwrap();
        assert_eq!(a_item.runner_count, 1);
        let b_item = items.iter().find(|i| i.crew.name == "B").unwrap();
        assert_eq!(b_item.runner_count, 0);
    }

    #[test]
    fn update_preserves_unset_fields() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Original".into(),
                purpose: Some("keep me".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let updated = update(
            &conn,
            &crew.id,
            UpdateCrewInput {
                name: Some("Renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.purpose.as_deref(), Some("keep me"));
    }

    #[test]
    fn delete_cascades_to_slot_rows_but_spares_runner_row() {
        // Runners are global (C5.5). Deleting a crew should strip the
        // slot rows but leave the runner intact for other crews (or a
        // future direct chat).
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Doomed".into(),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                created_at, updated_at
             ) VALUES ('r1', 'lead', 'Lead', 'shell', 'sh',
                       '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('s1', ?1, 'r1', 'lead', 0, 1, '2026-04-22T00:00:00Z')",
            params![crew.id],
        )
        .unwrap();

        delete(&mut conn, &crew.id).unwrap();
        let slot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM slots WHERE crew_id = ?1",
                params![crew.id],
                |r| r.get(0),
            )
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners WHERE id = 'r1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(slot_count, 0);
        assert_eq!(runner_count, 1, "runner outlives the crew");
    }

    #[test]
    fn delete_refuses_crew_with_any_non_archived_mission() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Busy".into(),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m-live', ?1, 'Live', 'running', '2026-04-22T00:00:00Z')",
            params![crew.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, started_at, stopped_at)
             VALUES ('m-aborted', ?1, 'Aborted', 'aborted',
                     '2026-04-22T00:01:00Z', '2026-04-22T00:02:00Z')",
            params![crew.id],
        )
        .unwrap();

        let err = delete(&mut conn, &crew.id).unwrap_err().to_string();
        assert!(err.contains("non-archived missions"));
        assert!(err.contains("m-live"));
        assert!(err.contains("m-aborted"));
        let crew_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crews WHERE id = ?1",
                params![crew.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(crew_count, 1, "crew must survive a refused delete");
    }

    #[test]
    fn delete_removes_archived_mission_sessions_before_deleting_crew() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Archived".into(),
                ..Default::default()
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                created_at, updated_at
             ) VALUES ('r1', 'reviewer', 'Reviewer', 'shell', 'sh',
                       '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions
                (id, crew_id, title, status, started_at, stopped_at, archived_at)
             VALUES ('m-archived', ?1, 'Done', 'completed',
                     '2026-04-22T00:00:00Z', '2026-04-22T01:00:00Z',
                     '2026-04-22T01:00:00Z')",
            params![crew.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, slot_id, status, started_at)
             VALUES ('s-archived', 'm-archived', 'r1', 'slot-reviewer',
                     'stopped', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();

        delete(&mut conn, &crew.id).unwrap();
        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 's-archived'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mission_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM missions WHERE id = 'm-archived'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 0, "app layer deletes mission sessions first");
        assert_eq!(
            mission_count, 0,
            "crew delete still removes archived missions"
        );
    }

    #[test]
    fn empty_name_is_rejected() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let err = create(
            &conn,
            CreateCrewInput {
                name: "   ".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn create_and_get_roundtrips_system_prompt_addendum() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Conventional".into(),
                system_prompt_addendum: Some("squash PRs against main".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            crew.system_prompt_addendum.as_deref(),
            Some("squash PRs against main"),
        );
        let reread = get(&conn, &crew.id).unwrap();
        assert_eq!(
            reread.system_prompt_addendum.as_deref(),
            Some("squash PRs against main"),
        );
    }

    #[test]
    fn create_collapses_empty_addendum_to_null() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Blank".into(),
                system_prompt_addendum: Some("   \n  ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(crew.system_prompt_addendum, None);
    }

    #[test]
    fn update_some_some_empty_string_clears_addendum_to_null() {
        // Wire contract for the CrewEditor "Save" action: when the
        // operator clears the field, the frontend sends `null`. But
        // an older or alternate caller sending `Some("")` must also
        // collapse to NULL so the column never stores a degenerate
        // empty string. Issue #54 spelled this out explicitly.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Pre-filled".into(),
                system_prompt_addendum: Some("convention".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let updated = update(
            &conn,
            &crew.id,
            UpdateCrewInput {
                system_prompt_addendum: Some(Some(String::new())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.system_prompt_addendum, None);
    }

    #[test]
    fn update_outer_none_preserves_existing_addendum() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let crew = create(
            &conn,
            CreateCrewInput {
                name: "Keeper".into(),
                system_prompt_addendum: Some("keep this".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let updated = update(
            &conn,
            &crew.id,
            UpdateCrewInput {
                name: Some("Renamed Keeper".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.name, "Renamed Keeper");
        assert_eq!(updated.system_prompt_addendum.as_deref(), Some("keep this"));
    }
}
