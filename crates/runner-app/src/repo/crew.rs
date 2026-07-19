// `crews` table — the top-level container for a team of runners.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named_with_fields};

use crate::model::{Crew, Timestamp};

use super::{de_err, insert_sql, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrewRow {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub goal: Option<String>,
    pub system_prompt_addendum: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub created_at: Timestamp,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub updated_at: Timestamp,
}

pub const COLUMNS: &[&str] = &[
    "id",
    "name",
    "purpose",
    "goal",
    "system_prompt_addendum",
    "created_at",
    "updated_at",
];

const UPDATE_FIELDS: &[&str] = &[
    "name",
    "purpose",
    "goal",
    "system_prompt_addendum",
    "updated_at",
    "id",
];

impl From<CrewRow> for Crew {
    fn from(r: CrewRow) -> Self {
        Crew {
            id: r.id,
            name: r.name,
            purpose: r.purpose,
            goal: r.goal,
            system_prompt_addendum: r.system_prompt_addendum,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

impl From<&Crew> for CrewRow {
    fn from(c: &Crew) -> Self {
        CrewRow {
            id: c.id.clone(),
            name: c.name.clone(),
            purpose: c.purpose.clone(),
            goal: c.goal.clone(),
            system_prompt_addendum: c.system_prompt_addendum.clone(),
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

pub fn insert(conn: &Connection, row: &CrewRow) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("crews", COLUMNS),
        to_params_named_with_fields(row, COLUMNS)
            .map_err(ser_err)?
            .to_slice()
            .as_slice(),
    )?;
    Ok(())
}

/// Full-row update of every writable column (the command layer resolves
/// leave-untouched semantics against the existing row before calling).
pub fn update(conn: &Connection, row: &CrewRow) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE crews
            SET name = :name,
                purpose = :purpose,
                goal = :goal,
                system_prompt_addendum = :system_prompt_addendum,
                updated_at = :updated_at
          WHERE id = :id",
        to_params_named_with_fields(row, UPDATE_FIELDS)
            .map_err(ser_err)?
            .to_slice()
            .as_slice(),
    )
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Crew>> {
    let sql = format!("SELECT {} FROM crews WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<CrewRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Crew::from))
}

/// Every crew with its slot count, ordered by creation time. Feeds
/// `CrewListItem`.
pub fn list_with_runner_count(conn: &Connection) -> rusqlite::Result<Vec<(Crew, i64)>> {
    let sql = format!(
        "SELECT {},
                (SELECT COUNT(*) FROM slots s WHERE s.crew_id = c.id) AS runner_count
           FROM crews c
         ORDER BY c.created_at ASC",
        super::qualified_select_list("c", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let crew = from_row::<CrewRow>(row).map_err(de_err)?;
        let runner_count: i64 = row.get("runner_count")?;
        Ok((Crew::from(crew), runner_count))
    })?;
    rows.collect()
}

/// One row per slot across every crew, in (crew_id, position) order —
/// the bulk member-preview feed for the Crews list cards.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MemberPreviewRow {
    pub crew_id: String,
    pub slot_handle: String,
    pub runner_handle: String,
    pub runtime: String,
    pub lead: bool,
}

pub fn list_member_previews(conn: &Connection) -> rusqlite::Result<Vec<MemberPreviewRow>> {
    // `runtime` is the slot's *effective* engine — the per-slot
    // override when set (feature 41), else the runner's default — so
    // the Crews cards label mixed-engine crews honestly.
    let mut stmt = conn.prepare(
        "SELECT s.crew_id, s.slot_handle, s.lead, r.handle AS runner_handle,
                COALESCE(s.runtime_override, r.runtime) AS runtime
           FROM slots s
           JOIN runners r ON r.id = s.runner_id
          ORDER BY s.crew_id ASC, s.position ASC",
    )?;
    let rows = stmt.query_map([], |row| from_row::<MemberPreviewRow>(row).map_err(de_err))?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM crews WHERE id = ?1", rusqlite::params![id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn full_row() -> CrewRow {
        let now = Utc::now();
        CrewRow {
            id: "c-full".into(),
            name: "Full crew".into(),
            purpose: Some("purpose".into()),
            goal: Some("goal".into()),
            system_prompt_addendum: Some("squash PRs against main".into()),
            created_at: now,
            updated_at: now,
        }
    }

    fn minimal_row() -> CrewRow {
        let now = Utc::now();
        CrewRow {
            id: "c-min".into(),
            name: "Minimal".into(),
            purpose: None,
            goal: None,
            system_prompt_addendum: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            let read = get(&conn, &row.id).unwrap().unwrap();
            assert_eq!(CrewRow::from(&read), row);
        }
    }

    #[test]
    fn legacy_rows_read_cleanly_including_z_timestamps() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        // Raw SQL in today's exact stored formats: mixed `Z` (seed shape)
        // and `+00:00` timestamp spellings must both parse.
        conn.execute(
            r#"INSERT INTO crews
                (id, name, purpose, goal, system_prompt_addendum,
                 created_at, updated_at)
             VALUES ('c-legacy', 'Legacy', NULL, 'ship it', NULL,
                     '2026-05-03T00:00:00Z', '2026-05-03T00:00:00+00:00')"#,
            [],
        )
        .unwrap();

        let crew = get(&conn, "c-legacy").unwrap().unwrap();
        assert_eq!(
            crew.created_at, crew.updated_at,
            "both spellings, same instant"
        );
        assert_eq!(crew.goal.as_deref(), Some("ship it"));
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = full_row();
        insert(&conn, &row).unwrap();
        let (created_raw, updated_raw): (String, String) = conn
            .query_row(
                "SELECT created_at, updated_at FROM crews WHERE id = 'c-full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(created_raw, row.created_at.to_rfc3339());
        assert_eq!(updated_raw, row.updated_at.to_rfc3339());
    }

    #[test]
    fn list_with_runner_count_and_member_previews_match_todays_shapes() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert(&conn, &full_row()).unwrap();
        insert(&conn, &minimal_row()).unwrap();
        conn.execute(
            "INSERT INTO runners (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES ('r1', 'lead', 'Lead', 'shell', 'sh', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('s1', 'c-full', 'r1', 'lead-slot', 0, 1, '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        // Overridden slot: the preview must report the effective
        // engine, not the runner default (feature 41).
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead,
                 runtime_override, added_at)
             VALUES ('s2', 'c-full', 'r1', 'override-slot', 1, 0,
                     'claude-code', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();

        let listed = list_with_runner_count(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        let full = listed.iter().find(|(c, _)| c.id == "c-full").unwrap();
        assert_eq!(full.1, 2);
        let min = listed.iter().find(|(c, _)| c.id == "c-min").unwrap();
        assert_eq!(min.1, 0);

        let previews = list_member_previews(&conn).unwrap();
        assert_eq!(
            previews,
            vec![
                MemberPreviewRow {
                    crew_id: "c-full".into(),
                    slot_handle: "lead-slot".into(),
                    runner_handle: "lead".into(),
                    runtime: "shell".into(),
                    lead: true,
                },
                MemberPreviewRow {
                    crew_id: "c-full".into(),
                    slot_handle: "override-slot".into(),
                    runner_handle: "lead".into(),
                    runtime: "claude-code".into(),
                    lead: false,
                },
            ]
        );
    }
}
