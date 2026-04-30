// Slot CRUD — manages the `slots` join table.
//
// A Slot is a position in a crew that references a Runner template
// and carries its own in-crew identity (`slot_handle`). Runner CRUD
// is in commands/runner.rs — a runner exists globally and can be
// referenced by zero or more slots across any number of crews. The
// same runner template can fill multiple slots in the same crew with
// different slot_handles.
//
// Invariants enforced here:
//   - A crew with ≥1 slot has exactly one `lead = 1` row. We enforce
//     this in `create` / `set_lead` (clear-others-then-set inside a
//     transaction) — no schema-level partial unique index.
//   - First slot added to a crew is auto-lead.
//   - Removing the lead while other slots remain auto-promotes the
//     slot at the lowest `position`.
//   - `position` is dense within a crew (0, 1, 2, ...) and enforced
//     unique by the schema.
//   - `slot_handle` is unique within a crew (schema-enforced).

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;
use ulid::Ulid as UlidGen;

use crate::{
    commands::runner,
    error::{Error, Result},
    model::{Slot, SlotWithRunner, Timestamp},
    AppState,
};

/// One crew that a given runner template is referenced by, plus the
/// slot's lead flag and added-at timestamp. Returned by
/// `runner_crews_list` to render the "Crews using this runner" panel
/// on Runner Detail.
#[derive(Debug, Clone, Serialize)]
pub struct CrewMembership {
    pub crew_id: String,
    pub crew_name: String,
    pub slot_id: String,
    pub slot_handle: String,
    pub lead: bool,
    pub position: i64,
    pub added_at: Timestamp,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateSlotInput {
    pub slot_handle: Option<String>,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

fn crew_exists(conn: &Connection, crew_id: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM crews WHERE id = ?1", params![crew_id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(found.is_some())
}

fn runner_exists(conn: &Connection, runner_id: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM runners WHERE id = ?1",
            params![runner_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// Renumber a crew's surviving slots so `position` is dense (0..N-1)
/// in the current display order. Same two-pass idiom as before:
/// `UNIQUE(crew_id, position)` would transiently violate during a
/// shift, so park each survivor at a negative slot first then
/// rewrite the final positions.
pub(super) fn repack_positions(conn: &Connection, crew_id: &str) -> Result<()> {
    let ordered: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM slots
              WHERE crew_id = ?1
              ORDER BY position ASC",
        )?;
        let rows = stmt.query_map(params![crew_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (i, id) in ordered.iter().enumerate() {
        conn.execute(
            "UPDATE slots SET position = ?1 WHERE id = ?2",
            params![-(i as i64) - 1, id],
        )?;
    }
    for (position, id) in ordered.iter().enumerate() {
        conn.execute(
            "UPDATE slots SET position = ?1 WHERE id = ?2",
            params![position as i64, id],
        )?;
    }
    Ok(())
}

fn row_to_slot(row: &rusqlite::Row<'_>) -> rusqlite::Result<Slot> {
    let lead: i64 = row.get("lead")?;
    let added_at_raw: String = row.get("added_at")?;
    let added_at: Timestamp = added_at_raw.parse().map_err(|e: chrono::ParseError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Slot {
        id: row.get("id")?,
        crew_id: row.get("crew_id")?,
        runner_id: row.get("runner_id")?,
        slot_handle: row.get("slot_handle")?,
        position: row.get("position")?,
        lead: lead != 0,
        added_at,
    })
}

fn get_slot_internal(conn: &Connection, slot_id: &str) -> Result<Slot> {
    conn.query_row(
        "SELECT id, crew_id, runner_id, slot_handle, position, lead, added_at
           FROM slots WHERE id = ?1",
        params![slot_id],
        row_to_slot,
    )
    .optional()?
    .ok_or_else(|| Error::msg(format!("slot not found: {slot_id}")))
}

/// Return the slots that belong to a crew, ordered by position, each
/// joined with its referenced Runner template. Two queries — one for
/// the slot rows, one for each unique runner template — instead of a
/// big alias-mangled JOIN. Pre-release crews are tiny; readability
/// wins.
pub fn list(conn: &Connection, crew_id: &str) -> Result<Vec<SlotWithRunner>> {
    let mut stmt = conn.prepare(
        "SELECT id, crew_id, runner_id, slot_handle, position, lead, added_at
           FROM slots
          WHERE crew_id = ?1
          ORDER BY position ASC",
    )?;
    let slots: Vec<Slot> = stmt
        .query_map(params![crew_id], row_to_slot)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(slots.len());
    for slot in slots {
        let runner = runner::get(conn, &slot.runner_id)?;
        out.push(SlotWithRunner { slot, runner });
    }
    Ok(out)
}

/// Inverse of `list`: every slot that references this runner template,
/// across every crew. Drives the Runner Detail "Crews using this
/// runner" panel.
pub fn list_crews_for_runner(conn: &Connection, runner_id: &str) -> Result<Vec<CrewMembership>> {
    let mut stmt = conn.prepare(
        "SELECT c.id AS crew_id, c.name AS crew_name,
                sl.id AS slot_id, sl.slot_handle AS slot_handle,
                sl.lead AS lead,
                sl.position AS position, sl.added_at AS added_at
           FROM slots sl
           JOIN crews c ON c.id = sl.crew_id
          WHERE sl.runner_id = ?1
          ORDER BY sl.added_at DESC",
    )?;
    let rows = stmt.query_map(params![runner_id], |row| {
        let added_at_raw: String = row.get("added_at")?;
        let added_at: Timestamp = added_at_raw.parse().map_err(|e: chrono::ParseError| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let lead_int: i64 = row.get("lead")?;
        Ok(CrewMembership {
            crew_id: row.get("crew_id")?,
            crew_name: row.get("crew_name")?,
            slot_id: row.get("slot_id")?,
            slot_handle: row.get("slot_handle")?,
            lead: lead_int != 0,
            position: row.get("position")?,
            added_at,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Append a new slot to `crew_id`'s roster at the next position. The
/// same runner template can be referenced by multiple slots in the
/// same crew as long as their `slot_handle` values differ.
pub fn create(
    conn: &mut Connection,
    crew_id: &str,
    runner_id: &str,
    slot_handle: &str,
) -> Result<SlotWithRunner> {
    if !crew_exists(conn, crew_id)? {
        return Err(Error::msg(format!("crew not found: {crew_id}")));
    }
    if !runner_exists(conn, runner_id)? {
        return Err(Error::msg(format!("runner not found: {runner_id}")));
    }
    let slot_handle = slot_handle.trim();
    if slot_handle.is_empty() {
        return Err(Error::msg("slot_handle must not be empty"));
    }

    let id = new_id();
    let ts = now().to_rfc3339();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM slots WHERE crew_id = ?1",
        params![crew_id],
        |r| r.get(0),
    )?;
    let next_position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM slots WHERE crew_id = ?1",
        params![crew_id],
        |r| r.get(0),
    )?;
    let is_first = count == 0;
    let lead: i64 = if is_first { 1 } else { 0 };

    tx.execute(
        "INSERT INTO slots
            (id, crew_id, runner_id, slot_handle, position, lead, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, crew_id, runner_id, slot_handle, next_position, lead, ts],
    )
    .map_err(|e| match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::ConstraintViolation) => Error::msg(format!(
            "slot_handle '{slot_handle}' is already used in this crew"
        )),
        _ => e.into(),
    })?;

    tx.commit()?;

    list(conn, crew_id)?
        .into_iter()
        .find(|s| s.slot.id == id)
        .ok_or_else(|| Error::msg("slot_create: inserted row vanished"))
}

/// Edit a slot's `slot_handle`. Trims and rejects empty values. Slot
/// id, crew membership, runner template ref, position, and lead flag
/// are unchanged.
pub fn update(conn: &mut Connection, slot_id: &str, input: UpdateSlotInput) -> Result<SlotWithRunner> {
    let existing = get_slot_internal(conn, slot_id)?;

    let slot_handle = match input.slot_handle {
        Some(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                return Err(Error::msg("slot_handle must not be empty"));
            }
            trimmed.to_string()
        }
        None => existing.slot_handle.clone(),
    };

    conn.execute(
        "UPDATE slots SET slot_handle = ?1 WHERE id = ?2",
        params![slot_handle, slot_id],
    )
    .map_err(|e| match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::ConstraintViolation) => Error::msg(format!(
            "slot_handle '{slot_handle}' is already used in this crew"
        )),
        _ => e.into(),
    })?;

    list(conn, &existing.crew_id)?
        .into_iter()
        .find(|s| s.slot.id == slot_id)
        .ok_or_else(|| Error::msg("slot_update: row vanished mid-call"))
}

/// Remove a slot. Promotes the lowest-position surviving slot to lead
/// if we just removed the lead, and repacks positions.
pub fn delete(conn: &mut Connection, slot_id: &str) -> Result<()> {
    let existing = get_slot_internal(conn, slot_id)?;
    let crew_id = existing.crew_id.clone();
    let was_lead = existing.lead;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let affected = tx.execute("DELETE FROM slots WHERE id = ?1", params![slot_id])?;
    if affected != 1 {
        return Err(Error::msg(format!("slot not found: {slot_id}")));
    }

    if was_lead {
        let promote: Option<String> = tx
            .query_row(
                "SELECT id FROM slots
                  WHERE crew_id = ?1
                  ORDER BY position ASC LIMIT 1",
                params![crew_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(new_lead) = promote {
            tx.execute(
                "UPDATE slots SET lead = 1 WHERE id = ?1",
                params![new_lead],
            )?;
        }
    }

    repack_positions(&tx, &crew_id)?;

    tx.commit()?;
    Ok(())
}

/// Atomically transfer leadership within a crew. No-op if the target
/// slot is already lead. Errors if the slot doesn't exist.
pub fn set_lead(conn: &mut Connection, slot_id: &str) -> Result<SlotWithRunner> {
    let existing = get_slot_internal(conn, slot_id)?;
    let crew_id = existing.crew_id.clone();

    if existing.lead {
        return list(conn, &crew_id)?
            .into_iter()
            .find(|s| s.slot.id == slot_id)
            .ok_or_else(|| Error::msg("slot_set_lead: slot vanished mid-call"));
    }

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // Clear the old lead first so no schema-level uniqueness check
    // ever sees two lead=1 rows in the same crew (we removed the
    // partial unique index, but the invariant lives here).
    tx.execute(
        "UPDATE slots SET lead = 0 WHERE crew_id = ?1 AND lead = 1",
        params![crew_id],
    )?;
    let affected = tx.execute(
        "UPDATE slots SET lead = 1 WHERE id = ?1",
        params![slot_id],
    )?;
    if affected != 1 {
        return Err(Error::msg(format!("slot not found: {slot_id}")));
    }

    tx.commit()?;

    list(conn, &crew_id)?
        .into_iter()
        .find(|s| s.slot.id == slot_id)
        .ok_or_else(|| Error::msg("slot_set_lead: slot vanished mid-call"))
}

/// Reorder a crew's slots. `ordered_slot_ids` must be a permutation
/// of the crew's current slot ids — no adds or removes allowed.
/// Positions are rewritten 0..N in the given order.
pub fn reorder(
    conn: &mut Connection,
    crew_id: &str,
    ordered_slot_ids: Vec<String>,
) -> Result<Vec<SlotWithRunner>> {
    let mut seen = std::collections::HashSet::new();
    for id in &ordered_slot_ids {
        if !seen.insert(id.clone()) {
            return Err(Error::msg("slot_reorder: ordered_slot_ids contains duplicates"));
        }
    }

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    let current: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM slots WHERE crew_id = ?1")?;
        let rows = stmt.query_map(params![crew_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if current.len() != ordered_slot_ids.len() {
        return Err(Error::msg(
            "slot_reorder: ordered_slot_ids must contain every slot exactly once",
        ));
    }
    for id in &current {
        if !seen.contains(id) {
            return Err(Error::msg(format!(
                "slot_reorder: ordered_slot_ids missing slot {id}"
            )));
        }
    }

    // Two-pass to avoid transient violations of UNIQUE(crew_id, position).
    for (i, id) in current.iter().enumerate() {
        tx.execute(
            "UPDATE slots SET position = ?1 WHERE id = ?2",
            params![-(i as i64) - 1, id],
        )?;
    }
    for (position, id) in ordered_slot_ids.iter().enumerate() {
        let affected = tx.execute(
            "UPDATE slots SET position = ?1 WHERE id = ?2",
            params![position as i64, id],
        )?;
        if affected != 1 {
            return Err(Error::msg(format!(
                "slot_reorder: slot {id} not in crew {crew_id}"
            )));
        }
    }

    tx.commit()?;
    list(conn, crew_id)
}

// ---------------------------------------------------------------------
// Tauri command wrappers
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn slot_list(
    state: State<'_, AppState>,
    crew_id: String,
) -> Result<Vec<SlotWithRunner>> {
    let conn = state.db.get()?;
    list(&conn, &crew_id)
}

#[tauri::command]
pub async fn runner_crews_list(
    state: State<'_, AppState>,
    runner_id: String,
) -> Result<Vec<CrewMembership>> {
    let conn = state.db.get()?;
    list_crews_for_runner(&conn, &runner_id)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateSlotInput {
    pub crew_id: String,
    pub runner_id: String,
    pub slot_handle: String,
}

#[tauri::command]
pub async fn slot_create(
    state: State<'_, AppState>,
    input: CreateSlotInput,
) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    create(
        &mut conn,
        &input.crew_id,
        &input.runner_id,
        &input.slot_handle,
    )
}

#[tauri::command]
pub async fn slot_update(
    state: State<'_, AppState>,
    slot_id: String,
    input: UpdateSlotInput,
) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    update(&mut conn, &slot_id, input)
}

#[tauri::command]
pub async fn slot_delete(state: State<'_, AppState>, slot_id: String) -> Result<()> {
    let mut conn = state.db.get()?;
    delete(&mut conn, &slot_id)
}

#[tauri::command]
pub async fn slot_set_lead(
    state: State<'_, AppState>,
    slot_id: String,
) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    set_lead(&mut conn, &slot_id)
}

#[tauri::command]
pub async fn slot_reorder(
    state: State<'_, AppState>,
    crew_id: String,
    ordered_slot_ids: Vec<String>,
) -> Result<Vec<SlotWithRunner>> {
    let mut conn = state.db.get()?;
    reorder(&mut conn, &crew_id, ordered_slot_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::crew, db};
    use std::collections::HashMap;

    fn pool() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    fn seed_crew(conn: &Connection, name: &str) -> String {
        crew::create(
            conn,
            crew::CreateCrewInput {
                name: name.into(),
                purpose: None,
                goal: None,
            },
        )
        .unwrap()
        .id
    }

    fn seed_runner(conn: &Connection, handle: &str) -> String {
        runner::create(
            conn,
            runner::CreateRunnerInput {
                handle: handle.into(),
                display_name: format!("{handle} display"),
                runtime: "shell".into(),
                command: "sh".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
            model: None,
            effort: None,
            },
        )
        .unwrap()
        .id
    }

    #[test]
    fn first_slot_added_becomes_lead() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "Alpha");
        let r = seed_runner(&conn, "lead-template");
        let added = create(&mut conn, &c, &r, "lead-slot").unwrap();
        assert!(added.slot.lead);
        assert_eq!(added.slot.position, 0);
        assert_eq!(added.slot.slot_handle, "lead-slot");
    }

    #[test]
    fn second_slot_is_not_lead() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "Alpha");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        create(&mut conn, &c, &r1, "alpha").unwrap();
        let second = create(&mut conn, &c, &r2, "beta").unwrap();
        assert!(!second.slot.lead);
        assert_eq!(second.slot.position, 1);
    }

    #[test]
    fn same_runner_can_fill_two_slots_in_same_crew() {
        // The defining feature of slots — same template, two roles.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "Alpha");
        let r = seed_runner(&conn, "claude");
        create(&mut conn, &c, &r, "architect").unwrap();
        create(&mut conn, &c, &r, "reviewer").unwrap();
        let roster = list(&conn, &c).unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].slot.runner_id, roster[1].slot.runner_id);
        assert_ne!(roster[0].slot.slot_handle, roster[1].slot.slot_handle);
    }

    #[test]
    fn shared_runner_can_belong_to_multiple_crews() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c1 = seed_crew(&conn, "A");
        let c2 = seed_crew(&conn, "B");
        let r = seed_runner(&conn, "shared");
        create(&mut conn, &c1, &r, "shared-a").unwrap();
        create(&mut conn, &c2, &r, "shared-b").unwrap();
        let in_c1 = list(&conn, &c1).unwrap();
        let in_c2 = list(&conn, &c2).unwrap();
        assert_eq!(in_c1.len(), 1);
        assert_eq!(in_c2.len(), 1);
        assert_eq!(in_c1[0].slot.runner_id, in_c2[0].slot.runner_id);
        assert!(in_c1[0].slot.lead);
        assert!(in_c2[0].slot.lead);
    }

    #[test]
    fn duplicate_slot_handle_in_same_crew_errors() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        create(&mut conn, &c, &r1, "shared-handle").unwrap();
        let err = create(&mut conn, &c, &r2, "shared-handle").unwrap_err();
        assert!(err.to_string().contains("already used"));
    }

    #[test]
    fn set_lead_reassigns_atomically() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "one");
        let r2 = seed_runner(&conn, "two");
        let s1 = create(&mut conn, &c, &r1, "one").unwrap();
        let s2 = create(&mut conn, &c, &r2, "two").unwrap();

        let promoted = set_lead(&mut conn, &s2.slot.id).unwrap();
        assert!(promoted.slot.lead);

        let roster = list(&conn, &c).unwrap();
        let leads = roster.iter().filter(|m| m.slot.lead).count();
        assert_eq!(leads, 1, "exactly one lead per crew");
        assert!(!roster.iter().find(|m| m.slot.id == s1.slot.id).unwrap().slot.lead);
        assert!(roster.iter().find(|m| m.slot.id == s2.slot.id).unwrap().slot.lead);
    }

    #[test]
    fn remove_lead_auto_promotes_lowest_position() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        let r3 = seed_runner(&conn, "gamma");
        let s1 = create(&mut conn, &c, &r1, "alpha").unwrap();
        create(&mut conn, &c, &r2, "beta").unwrap();
        let s3 = create(&mut conn, &c, &r3, "gamma").unwrap();
        set_lead(&mut conn, &s3.slot.id).unwrap();

        delete(&mut conn, &s3.slot.id).unwrap();
        let roster = list(&conn, &c).unwrap();
        assert!(roster.iter().find(|m| m.slot.id == s1.slot.id).unwrap().slot.lead);
    }

    #[test]
    fn removing_last_member_leaves_empty_crew() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "only");
        let s = create(&mut conn, &c, &r, "only").unwrap();
        delete(&mut conn, &s.slot.id).unwrap();
        assert!(list(&conn, &c).unwrap().is_empty());
    }

    #[test]
    fn reorder_rewrites_positions_and_preserves_lead() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        let r3 = seed_runner(&conn, "gamma");
        let s1 = create(&mut conn, &c, &r1, "alpha").unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta").unwrap();
        let s3 = create(&mut conn, &c, &r3, "gamma").unwrap();

        let roster = reorder(
            &mut conn,
            &c,
            vec![s3.slot.id.clone(), s1.slot.id.clone(), s2.slot.id.clone()],
        )
        .unwrap();
        assert_eq!(roster[0].slot.id, s3.slot.id);
        assert_eq!(roster[0].slot.position, 0);
        assert_eq!(roster[1].slot.id, s1.slot.id);
        assert_eq!(roster[1].slot.position, 1);
        assert_eq!(roster[2].slot.id, s2.slot.id);
        assert_eq!(roster[2].slot.position, 2);

        // s1 was the original lead — position changes, but lead doesn't.
        assert!(roster.iter().find(|m| m.slot.id == s1.slot.id).unwrap().slot.lead);
    }

    #[test]
    fn removing_middle_slot_keeps_positions_dense() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        let r3 = seed_runner(&conn, "gamma");
        create(&mut conn, &c, &r1, "alpha").unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta").unwrap();
        create(&mut conn, &c, &r3, "gamma").unwrap();

        delete(&mut conn, &s2.slot.id).unwrap();

        let roster = list(&conn, &c).unwrap();
        let positions: Vec<i64> = roster.iter().map(|m| m.slot.position).collect();
        assert_eq!(
            positions,
            vec![0, 1],
            "positions must be dense after middle removal"
        );

        let r4 = seed_runner(&conn, "delta");
        let added = create(&mut conn, &c, &r4, "delta").unwrap();
        assert_eq!(
            added.slot.position, 2,
            "new slot appends at the dense next position"
        );
    }

    #[test]
    fn deleting_runner_cascades_slots_and_repacks_other_crews() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c1 = seed_crew(&conn, "A");
        let c2 = seed_crew(&conn, "B");
        let shared = seed_runner(&conn, "shared");
        let a2 = seed_runner(&conn, "a2");
        let b1 = seed_runner(&conn, "b1");
        let b2 = seed_runner(&conn, "b2");
        create(&mut conn, &c1, &a2, "a2").unwrap();
        create(&mut conn, &c1, &shared, "shared-a").unwrap();
        create(&mut conn, &c2, &b1, "b1").unwrap();
        create(&mut conn, &c2, &shared, "shared-b").unwrap();
        create(&mut conn, &c2, &b2, "b2").unwrap();

        runner::delete(&mut conn, &shared).unwrap();

        let in_a = list(&conn, &c1).unwrap();
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].slot.position, 0);

        let in_b = list(&conn, &c2).unwrap();
        let positions: Vec<i64> = in_b.iter().map(|m| m.slot.position).collect();
        assert_eq!(positions, vec![0, 1], "crew B dense after cascade + repack");
    }

    #[test]
    fn update_renames_slot_handle() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let s = create(&mut conn, &c, &r, "old").unwrap();
        let updated = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                slot_handle: Some("new".into()),
            },
        )
        .unwrap();
        assert_eq!(updated.slot.slot_handle, "new");
    }

    #[test]
    fn update_rejects_handle_collision_in_same_crew() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        create(&mut conn, &c, &r1, "alpha").unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta").unwrap();
        let err = update(
            &mut conn,
            &s2.slot.id,
            UpdateSlotInput {
                slot_handle: Some("alpha".into()),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already used"));
    }
}
