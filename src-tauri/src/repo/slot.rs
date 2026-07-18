// `slots` table — one position in a crew, referencing a Runner template.
//
// Invariant enforcement (one lead per crew, dense positions, auto-promote
// on delete) stays in `commands::slot`; this module owns the row struct,
// the column list, and the statements.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named};

use crate::model::{Slot, Timestamp};

use super::{de_err, insert_sql, qualified_select_list, select_list, ser_err};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotRow {
    pub id: String,
    pub crew_id: String,
    pub runner_id: String,
    pub slot_handle: String,
    pub position: i64,
    pub lead: bool,
    pub runtime_override: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339")]
    pub added_at: Timestamp,
}

pub const COLUMNS: &[&str] = &[
    "id",
    "crew_id",
    "runner_id",
    "slot_handle",
    "position",
    "lead",
    "runtime_override",
    "added_at",
];

impl From<SlotRow> for Slot {
    fn from(r: SlotRow) -> Self {
        Slot {
            id: r.id,
            crew_id: r.crew_id,
            runner_id: r.runner_id,
            slot_handle: r.slot_handle,
            position: r.position,
            lead: r.lead,
            runtime_override: r.runtime_override,
            added_at: r.added_at,
        }
    }
}

impl From<&Slot> for SlotRow {
    fn from(s: &Slot) -> Self {
        SlotRow {
            id: s.id.clone(),
            crew_id: s.crew_id.clone(),
            runner_id: s.runner_id.clone(),
            slot_handle: s.slot_handle.clone(),
            position: s.position,
            lead: s.lead,
            runtime_override: s.runtime_override.clone(),
            added_at: s.added_at,
        }
    }
}

pub fn insert(conn: &Connection, row: &SlotRow) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("slots", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Slot>> {
    let sql = format!("SELECT {} FROM slots WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<SlotRow>(row).map_err(de_err)
    })
    .optional()
    .map(|opt| opt.map(Slot::from))
}

pub fn list_for_crew(conn: &Connection, crew_id: &str) -> rusqlite::Result<Vec<Slot>> {
    let sql = format!(
        "SELECT {} FROM slots WHERE crew_id = ?1 ORDER BY position ASC",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![crew_id], |row| {
        from_row::<SlotRow>(row).map_err(de_err)
    })?;
    rows.map(|r| r.map(Slot::from)).collect()
}

/// Every slot that references `runner_id`, across every crew, joined with
/// the crew name. Ordered by `added_at` DESC — drives the Runner Detail
/// "Crews using this runner" panel.
pub fn list_for_runner_with_crew_name(
    conn: &Connection,
    runner_id: &str,
) -> rusqlite::Result<Vec<(Slot, String)>> {
    let sql = format!(
        "SELECT {}, c.name AS crew_name
           FROM slots sl
           JOIN crews c ON c.id = sl.crew_id
          WHERE sl.runner_id = ?1
          ORDER BY sl.added_at DESC",
        qualified_select_list("sl", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![runner_id], |row| {
        let slot = from_row::<SlotRow>(row).map_err(de_err)?;
        let crew_name: String = row.get("crew_name")?;
        Ok((Slot::from(slot), crew_name))
    })?;
    rows.collect()
}

pub fn set_slot_handle(conn: &Connection, id: &str, slot_handle: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots SET slot_handle = ?1 WHERE id = ?2",
        rusqlite::params![slot_handle, id],
    )
}

pub fn set_runtime_override(
    conn: &Connection,
    id: &str,
    runtime_override: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots SET runtime_override = ?1 WHERE id = ?2",
        rusqlite::params![runtime_override, id],
    )
}

pub fn set_position(conn: &Connection, id: &str, position: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots SET position = ?1 WHERE id = ?2",
        rusqlite::params![position, id],
    )
}

pub fn promote_to_lead(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots SET lead = 1 WHERE id = ?1",
        rusqlite::params![id],
    )
}

/// Clear the current lead flag for a crew. Run before `promote_to_lead`
/// inside the caller's transaction so no reader ever sees two leads.
pub fn clear_crew_lead(conn: &Connection, crew_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE slots SET lead = 0 WHERE crew_id = ?1 AND lead = 1",
        rusqlite::params![crew_id],
    )
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM slots WHERE id = ?1", rusqlite::params![id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn seed_crew(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![id, format!("crew-{id}"), "2026-04-22T00:00:00Z"],
        )
        .unwrap();
    }

    fn seed_runner(conn: &Connection, id: &str, handle: &str) {
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'shell', 'sh', ?4, ?4)",
            rusqlite::params![
                id,
                handle,
                format!("{handle} display"),
                "2026-04-22T00:00:00Z"
            ],
        )
        .unwrap();
    }

    fn full_row() -> SlotRow {
        SlotRow {
            id: "s-full".into(),
            crew_id: "c1".into(),
            runner_id: "r1".into(),
            slot_handle: "architect".into(),
            position: 3,
            lead: true,
            runtime_override: Some("claude-code".into()),
            added_at: Utc::now(),
        }
    }

    fn minimal_row() -> SlotRow {
        SlotRow {
            id: "s-min".into(),
            crew_id: "c1".into(),
            runner_id: "r1".into(),
            slot_handle: "worker".into(),
            position: 0,
            lead: false,
            runtime_override: None,
            added_at: Utc::now(),
        }
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        seed_runner(&conn, "r1", "alpha");

        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            let read = get(&conn, &row.id).unwrap().unwrap();
            assert_eq!(SlotRow::from(&read), row);
        }
    }

    #[test]
    fn legacy_rows_in_both_timestamp_spellings_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        seed_runner(&conn, "r1", "alpha");
        // Raw SQL in today's exact stored formats: `Z` (seeds / fixtures)
        // and `+00:00` (every to_rfc3339 write).
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('s-z', 'c1', 'r1', 'zulu', 0, 1, '2026-04-22T00:00:00Z'),
                    ('s-o', 'c1', 'r1', 'offset', 1, 0, '2026-04-22T00:00:00+00:00')",
            [],
        )
        .unwrap();

        let zulu = get(&conn, "s-z").unwrap().unwrap();
        let offset = get(&conn, "s-o").unwrap().unwrap();
        assert_eq!(zulu.added_at, offset.added_at);
        assert!(zulu.lead);
        assert!(!offset.lead);
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        seed_runner(&conn, "r1", "alpha");
        let row = full_row();
        insert(&conn, &row).unwrap();

        let (added_at_raw, lead_type, lead_raw): (String, String, i64) = conn
            .query_row(
                "SELECT added_at, typeof(lead), lead FROM slots WHERE id = 's-full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(added_at_raw, row.added_at.to_rfc3339());
        assert_eq!(lead_type, "integer");
        assert_eq!(lead_raw, 1);
    }

    #[test]
    fn list_for_runner_with_crew_name_joins_and_orders() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_crew(&conn, "c1");
        seed_crew(&conn, "c2");
        seed_runner(&conn, "r1", "shared");
        conn.execute(
            "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES ('s1', 'c1', 'r1', 'older', 0, 1, '2026-04-22T00:00:00Z'),
                    ('s2', 'c2', 'r1', 'newer', 0, 1, '2026-04-23T00:00:00Z')",
            [],
        )
        .unwrap();

        let rows = list_for_runner_with_crew_name(&conn, "r1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0.slot_handle, "newer");
        assert_eq!(rows[0].1, "crew-c2");
        assert_eq!(rows[1].0.slot_handle, "older");
        assert_eq!(rows[1].1, "crew-c1");
    }
}
