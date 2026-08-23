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

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Slot, SlotWithRunner, Timestamp},
    ops::runner,
    repo, AppCore,
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

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UpdateSlotInput {
    pub slot_handle: Option<String>,
    /// Per-slot engine choice. Omit to preserve, pass `null` to clear
    /// (back to the runner's own runtime), pass a registry runtime
    /// name to override. Validated against the runtime registry.
    #[serde(default, deserialize_with = "double_option")]
    pub runtime_override: Option<Option<String>>,
    /// Per-slot model. Omit to preserve, pass `null` or blank to
    /// inherit, or pass a model name to override.
    #[serde(default, deserialize_with = "double_option")]
    pub model_override: Option<Option<String>>,
    /// Per-slot thinking effort. Omit to preserve, pass `null` or
    /// blank to inherit, or pass an effort level to override.
    #[serde(default, deserialize_with = "double_option")]
    pub effort_override: Option<Option<String>>,
}

/// Present-vs-missing deserializer for the clear/preserve/set field.
/// With plain serde, `Option<Option<T>>` swallows an explicit JSON
/// `null` into the *outer* `None`, making "clear" arrive as
/// "preserve". Any present value — including `null` — lands here and
/// wraps in `Some`; only a missing key falls through to
/// `#[serde(default)]`.
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Normalize + validate a runtime-override value against the runtime
/// registry. Blank (after trim) collapses to None — the "Runner
/// default" sentinel.
fn validate_runtime_override(value: Option<&str>) -> Result<Option<String>> {
    let Some(name) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if crate::router::runtime::runtime_definition(name).is_none() {
        return Err(Error::msg(format!(
            "unknown runtime '{name}' — valid runtimes: {}",
            crate::router::runtime::runtime_definitions()
                .iter()
                .map(|r| r.name)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(Some(name.to_string()))
}

fn normalize_override_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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
        repo::slot::set_position(conn, id, -(i as i64) - 1)?;
    }
    for (position, id) in ordered.iter().enumerate() {
        repo::slot::set_position(conn, id, position as i64)?;
    }
    Ok(())
}

fn get_slot_internal(conn: &Connection, slot_id: &str) -> Result<Slot> {
    repo::slot::get(conn, slot_id)?.ok_or_else(|| Error::msg(format!("slot not found: {slot_id}")))
}

/// Return the slots that belong to a crew, ordered by position, each
/// joined with its referenced Runner template. The roster is loaded in two
/// queries: one for slots and one for the crew's unique runner templates.
pub fn list(conn: &Connection, crew_id: &str) -> Result<Vec<SlotWithRunner>> {
    let slots = repo::slot::list_for_crew(conn, crew_id)?;
    let runners = repo::runner::list_for_crew(conn, crew_id)?;
    let runners_by_id: HashMap<_, _> = runners
        .into_iter()
        .map(|runner| (runner.id.clone(), runner))
        .collect();
    let mut out = Vec::with_capacity(slots.len());
    for slot in slots {
        let runner = runners_by_id
            .get(&slot.runner_id)
            .cloned()
            .ok_or_else(|| Error::msg(format!("runner not found: {}", slot.runner_id)))?;
        out.push(SlotWithRunner { slot, runner });
    }
    Ok(out)
}

/// Inverse of `list`: every slot that references this runner template,
/// across every crew. Drives the Runner Detail "Crews using this
/// runner" panel.
pub fn list_crews_for_runner(conn: &Connection, runner_id: &str) -> Result<Vec<CrewMembership>> {
    let rows = repo::slot::list_for_runner_with_crew_name(conn, runner_id)?;
    Ok(rows
        .into_iter()
        .map(|(slot, crew_name)| CrewMembership {
            crew_id: slot.crew_id,
            crew_name,
            slot_id: slot.id,
            slot_handle: slot.slot_handle,
            lead: slot.lead,
            position: slot.position,
            added_at: slot.added_at,
        })
        .collect())
}

/// Append a new slot to `crew_id`'s roster at the next position. The
/// same runner template can be referenced by multiple slots in the
/// same crew as long as their `slot_handle` values differ.
pub fn create(
    conn: &mut Connection,
    crew_id: &str,
    runner_id: &str,
    slot_handle: &str,
    runtime_override: Option<&str>,
    model_override: Option<&str>,
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
    let runtime_override = validate_runtime_override(runtime_override)?;
    let model_override = normalize_override_value(model_override);

    let id = new_id();
    let added_at = now();
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

    repo::slot::insert(
        &tx,
        &repo::slot::SlotRow {
            id: id.clone(),
            crew_id: crew_id.to_string(),
            runner_id: runner_id.to_string(),
            slot_handle: slot_handle.to_string(),
            position: next_position,
            lead: is_first,
            runtime_override,
            model_override,
            effort_override: None,
            added_at,
        },
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

/// Edit a slot's handle and/or agent overrides. Engine changes clear
/// stale model and effort values unless the caller supplies replacements
/// in the same patch. Slot id, crew membership, runner template ref,
/// position, and lead flag are unchanged.
pub fn update(
    conn: &mut Connection,
    slot_id: &str,
    input: UpdateSlotInput,
) -> Result<SlotWithRunner> {
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
    let runtime_override = input
        .runtime_override
        .map(|v| validate_runtime_override(v.as_deref()))
        .transpose()?;
    let model_override = input
        .model_override
        .map(|value| normalize_override_value(value.as_deref()));
    let effort_override = input
        .effort_override
        .map(|value| normalize_override_value(value.as_deref()));
    let runner = runner::get(conn, &existing.runner_id)?;
    let final_runtime_override = runtime_override
        .clone()
        .unwrap_or_else(|| existing.runtime_override.clone());
    let current_runtime = existing
        .runtime_override
        .as_deref()
        .unwrap_or(&runner.runtime);
    let next_runtime = final_runtime_override.as_deref().unwrap_or(&runner.runtime);
    let engine_changed = current_runtime != next_runtime;
    let final_model_override = model_override.clone().unwrap_or_else(|| {
        if engine_changed {
            None
        } else {
            existing.model_override.clone()
        }
    });
    let final_effort_override = effort_override.clone().unwrap_or_else(|| {
        if engine_changed {
            None
        } else {
            existing.effort_override.clone()
        }
    });

    // Both fields commit atomically: a handle collision must not
    // leave a half-applied runtime change behind (or vice versa).
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(runtime_override) = &runtime_override {
        repo::slot::set_runtime_override(&tx, slot_id, runtime_override.as_deref())?;
    }
    if model_override.is_some() || engine_changed {
        repo::slot::set_model_override(&tx, slot_id, final_model_override.as_deref())?;
    }
    if effort_override.is_some() || engine_changed {
        repo::slot::set_effort_override(&tx, slot_id, final_effort_override.as_deref())?;
    }
    repo::slot::set_slot_handle(&tx, slot_id, &slot_handle).map_err(|e| {
        match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::ConstraintViolation) => Error::msg(format!(
                "slot_handle '{slot_handle}' is already used in this crew"
            )),
            _ => e.into(),
        }
    })?;
    tx.commit()?;

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

    let affected = repo::slot::delete(&tx, slot_id)?;
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
            repo::slot::promote_to_lead(&tx, &new_lead)?;
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
    repo::slot::clear_crew_lead(&tx, &crew_id)?;
    let affected = repo::slot::promote_to_lead(&tx, slot_id)?;
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
            return Err(Error::msg(
                "slot_reorder: ordered_slot_ids contains duplicates",
            ));
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
        repo::slot::set_position(&tx, id, -(i as i64) - 1)?;
    }
    for (position, id) in ordered_slot_ids.iter().enumerate() {
        let affected = repo::slot::set_position(&tx, id, position as i64)?;
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
// State-level command bodies
// ---------------------------------------------------------------------

pub fn slot_list(state: &AppCore, crew_id: &str) -> Result<Vec<SlotWithRunner>> {
    let conn = state.db.get()?;
    list(&conn, crew_id)
}

pub fn runner_crews_list(state: &AppCore, runner_id: &str) -> Result<Vec<CrewMembership>> {
    let conn = state.db.get()?;
    list_crews_for_runner(&conn, runner_id)
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateSlotInput {
    pub crew_id: String,
    pub runner_id: String,
    pub slot_handle: String,
    /// Optional per-slot engine choice. Omit (or blank) for the
    /// "Runner default" behavior; otherwise a runtime registry name.
    #[serde(default)]
    pub runtime_override: Option<String>,
    /// Optional model pinned to the selected runtime. Blank or omitted
    /// inherits from the runner template.
    #[serde(default)]
    pub model_override: Option<String>,
}

pub fn slot_create(state: &AppCore, input: CreateSlotInput) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    create(
        &mut conn,
        &input.crew_id,
        &input.runner_id,
        &input.slot_handle,
        input.runtime_override.as_deref(),
        input.model_override.as_deref(),
    )
}

pub fn slot_update(
    state: &AppCore,
    slot_id: &str,
    input: UpdateSlotInput,
) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    update(&mut conn, slot_id, input)
}

pub fn slot_delete(state: &AppCore, slot_id: &str) -> Result<()> {
    let mut conn = state.db.get()?;
    delete(&mut conn, slot_id)
}

pub fn slot_set_lead(state: &AppCore, slot_id: &str) -> Result<SlotWithRunner> {
    let mut conn = state.db.get()?;
    set_lead(&mut conn, slot_id)
}

pub fn slot_reorder(
    state: &AppCore,
    crew_id: &str,
    ordered_slot_ids: Vec<String>,
) -> Result<Vec<SlotWithRunner>> {
    let mut conn = state.db.get()?;
    reorder(&mut conn, crew_id, ordered_slot_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, ops::crew};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ROSTER_SELECT_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn count_roster_selects(sql: &str) {
        if sql.trim_start().starts_with("SELECT") {
            ROSTER_SELECT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn pool() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    fn seed_crew(conn: &Connection, name: &str) -> String {
        crew::create(
            conn,
            crew::CreateCrewInput {
                name: name.into(),
                ..Default::default()
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
                permission_mode: crate::router::runtime::PermissionMode::Auto,
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
        let added = create(&mut conn, &c, &r, "lead-slot", None, None).unwrap();
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
        create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        let second = create(&mut conn, &c, &r2, "beta", None, None).unwrap();
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
        create(&mut conn, &c, &r, "architect", None, None).unwrap();
        create(&mut conn, &c, &r, "reviewer", None, None).unwrap();
        let roster = list(&conn, &c).unwrap();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].slot.runner_id, roster[1].slot.runner_id);
        assert_ne!(roster[0].slot.slot_handle, roster[1].slot.slot_handle);
    }

    #[test]
    fn list_loads_slots_and_unique_runners_in_two_queries() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew = seed_crew(&conn, "Alpha");
        let shared = seed_runner(&conn, "shared");
        let other = seed_runner(&conn, "other");
        create(&mut conn, &crew, &shared, "lead", None, None).unwrap();
        create(&mut conn, &crew, &shared, "reviewer", None, None).unwrap();
        create(&mut conn, &crew, &other, "coder", None, None).unwrap();

        ROSTER_SELECT_COUNT.store(0, Ordering::Relaxed);
        conn.trace(Some(count_roster_selects));
        let roster = list(&conn, &crew).unwrap();
        conn.trace(None);

        assert_eq!(roster.len(), 3);
        assert_eq!(ROSTER_SELECT_COUNT.load(Ordering::Relaxed), 2);
        assert_eq!(roster[0].runner.id, shared);
        assert_eq!(roster[1].runner.id, shared);
        assert_eq!(roster[2].runner.id, other);
    }

    #[test]
    fn list_errors_instead_of_dropping_slot_with_missing_runner() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let crew = seed_crew(&conn, "Alpha");
        let runner = seed_runner(&conn, "missing");
        create(&mut conn, &crew, &runner, "lead", None, None).unwrap();

        conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        conn.execute("DELETE FROM runners WHERE id = ?1", params![runner])
            .unwrap();

        let error = list(&conn, &crew).unwrap_err();
        assert_eq!(error.to_string(), format!("runner not found: {runner}"));
    }

    #[test]
    fn shared_runner_can_belong_to_multiple_crews() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c1 = seed_crew(&conn, "A");
        let c2 = seed_crew(&conn, "B");
        let r = seed_runner(&conn, "shared");
        create(&mut conn, &c1, &r, "shared-a", None, None).unwrap();
        create(&mut conn, &c2, &r, "shared-b", None, None).unwrap();
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
        create(&mut conn, &c, &r1, "shared-handle", None, None).unwrap();
        let err = create(&mut conn, &c, &r2, "shared-handle", None, None).unwrap_err();
        assert!(err.to_string().contains("already used"));
    }

    #[test]
    fn set_lead_reassigns_atomically() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "one");
        let r2 = seed_runner(&conn, "two");
        let s1 = create(&mut conn, &c, &r1, "one", None, None).unwrap();
        let s2 = create(&mut conn, &c, &r2, "two", None, None).unwrap();

        let promoted = set_lead(&mut conn, &s2.slot.id).unwrap();
        assert!(promoted.slot.lead);

        let roster = list(&conn, &c).unwrap();
        let leads = roster.iter().filter(|m| m.slot.lead).count();
        assert_eq!(leads, 1, "exactly one lead per crew");
        assert!(
            !roster
                .iter()
                .find(|m| m.slot.id == s1.slot.id)
                .unwrap()
                .slot
                .lead
        );
        assert!(
            roster
                .iter()
                .find(|m| m.slot.id == s2.slot.id)
                .unwrap()
                .slot
                .lead
        );
    }

    #[test]
    fn remove_lead_auto_promotes_lowest_position() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        let r3 = seed_runner(&conn, "gamma");
        let s1 = create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        create(&mut conn, &c, &r2, "beta", None, None).unwrap();
        let s3 = create(&mut conn, &c, &r3, "gamma", None, None).unwrap();
        set_lead(&mut conn, &s3.slot.id).unwrap();

        delete(&mut conn, &s3.slot.id).unwrap();
        let roster = list(&conn, &c).unwrap();
        assert!(
            roster
                .iter()
                .find(|m| m.slot.id == s1.slot.id)
                .unwrap()
                .slot
                .lead
        );
    }

    #[test]
    fn removing_last_member_leaves_empty_crew() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "only");
        let s = create(&mut conn, &c, &r, "only", None, None).unwrap();
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
        let s1 = create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta", None, None).unwrap();
        let s3 = create(&mut conn, &c, &r3, "gamma", None, None).unwrap();

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
        assert!(
            roster
                .iter()
                .find(|m| m.slot.id == s1.slot.id)
                .unwrap()
                .slot
                .lead
        );
    }

    #[test]
    fn removing_middle_slot_keeps_positions_dense() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        let r3 = seed_runner(&conn, "gamma");
        create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta", None, None).unwrap();
        create(&mut conn, &c, &r3, "gamma", None, None).unwrap();

        delete(&mut conn, &s2.slot.id).unwrap();

        let roster = list(&conn, &c).unwrap();
        let positions: Vec<i64> = roster.iter().map(|m| m.slot.position).collect();
        assert_eq!(
            positions,
            vec![0, 1],
            "positions must be dense after middle removal"
        );

        let r4 = seed_runner(&conn, "delta");
        let added = create(&mut conn, &c, &r4, "delta", None, None).unwrap();
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
        create(&mut conn, &c1, &a2, "a2", None, None).unwrap();
        create(&mut conn, &c1, &shared, "shared-a", None, None).unwrap();
        create(&mut conn, &c2, &b1, "b1", None, None).unwrap();
        create(&mut conn, &c2, &shared, "shared-b", None, None).unwrap();
        create(&mut conn, &c2, &b2, "b2", None, None).unwrap();

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
        let s = create(&mut conn, &c, &r, "old", None, None).unwrap();
        let updated = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                slot_handle: Some("new".into()),
                ..Default::default()
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
        create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        let s2 = create(&mut conn, &c, &r2, "beta", None, None).unwrap();
        let err = update(
            &mut conn,
            &s2.slot.id,
            UpdateSlotInput {
                slot_handle: Some("alpha".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already used"));
    }

    #[test]
    fn create_persists_valid_runtime_override_and_blank_collapses_to_none() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");

        let with_override = create(
            &mut conn,
            &c,
            &r1,
            "alpha",
            Some("claude-code"),
            Some("  opus  "),
        )
        .unwrap();
        assert_eq!(
            with_override.slot.runtime_override.as_deref(),
            Some("claude-code")
        );
        assert_eq!(with_override.slot.model_override.as_deref(), Some("opus"));

        let blank = create(&mut conn, &c, &r2, "beta", Some("   "), None).unwrap();
        assert_eq!(blank.slot.runtime_override, None);
    }

    #[test]
    fn create_rejects_unknown_runtime_override() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let err = create(&mut conn, &c, &r, "alpha", Some("aider-future"), None).unwrap_err();
        assert!(
            err.to_string().contains("unknown runtime 'aider-future'"),
            "got: {err}",
        );
        assert!(
            err.to_string().contains("qoder"),
            "valid-runtime list must include qoder: {err}",
        );
        assert!(
            err.to_string().contains("trae"),
            "valid-runtime list must include trae: {err}",
        );
        assert!(list(&conn, &c).unwrap().is_empty(), "no row on rejection");
    }

    #[test]
    fn create_allows_model_override_without_runtime_override() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let created = create(&mut conn, &c, &r, "alpha", None, Some(" opus ")).unwrap();
        assert_eq!(created.slot.runtime_override, None);
        assert_eq!(created.slot.model_override.as_deref(), Some("opus"));
    }

    #[test]
    fn update_sets_preserves_and_clears_runtime_override() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let s = create(&mut conn, &c, &r, "alpha", None, None).unwrap();

        // Set.
        let set = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                runtime_override: Some(Some("codex".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(set.slot.runtime_override.as_deref(), Some("codex"));

        let model_set = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                model_override: Some(Some(" gpt-slot ".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(model_set.slot.model_override.as_deref(), Some("gpt-slot"));

        let effort_set = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                effort_override: Some(Some(" xhigh ".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(effort_set.slot.effort_override.as_deref(), Some("xhigh"));

        // Omitted field preserves.
        let preserved = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                slot_handle: Some("renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(preserved.slot.runtime_override.as_deref(), Some("codex"));
        assert_eq!(preserved.slot.model_override.as_deref(), Some("gpt-slot"));
        assert_eq!(preserved.slot.effort_override.as_deref(), Some("xhigh"));
        assert_eq!(preserved.slot.slot_handle, "renamed");

        // Explicit null clears back to Runner default.
        let cleared = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                runtime_override: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cleared.slot.runtime_override, None);
        assert_eq!(cleared.slot.model_override, None);
        assert_eq!(cleared.slot.effort_override, None);
    }

    #[test]
    fn update_round_trips_model_and_effort_without_runtime_override() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let s = create(&mut conn, &c, &r, "alpha", None, None).unwrap();

        let updated = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                model_override: Some(Some("slot-model".into())),
                effort_override: Some(Some("high".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.slot.runtime_override, None);
        assert_eq!(updated.slot.model_override.as_deref(), Some("slot-model"));
        assert_eq!(updated.slot.effort_override.as_deref(), Some("high"));

        let stored = list(&conn, &c).unwrap().pop().unwrap();
        assert_eq!(stored.slot.model_override.as_deref(), Some("slot-model"));
        assert_eq!(stored.slot.effort_override.as_deref(), Some("high"));
    }

    #[test]
    fn clearing_matching_runtime_override_preserves_model_and_effort() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        conn.execute(
            "UPDATE runners SET runtime = 'codex', command = 'codex' WHERE id = ?1",
            params![r],
        )
        .unwrap();
        let s = create(
            &mut conn,
            &c,
            &r,
            "alpha",
            Some("codex"),
            Some("slot-model"),
        )
        .unwrap();
        update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                effort_override: Some(Some("high".into())),
                ..Default::default()
            },
        )
        .unwrap();

        let cleared = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                runtime_override: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cleared.slot.runtime_override, None);
        assert_eq!(cleared.slot.model_override.as_deref(), Some("slot-model"));
        assert_eq!(cleared.slot.effort_override.as_deref(), Some("high"));
    }

    #[test]
    fn update_slot_input_wire_shape_distinguishes_missing_null_and_value() {
        // IPC/MCP callers speak JSON. The three wire shapes must map
        // to the three actions: missing key = preserve, explicit
        // null = clear, string = set. Plain serde would fold the
        // null into the outer Option and turn "clear" into a no-op —
        // the `double_option` deserializer is what keeps this test
        // green.
        let missing: UpdateSlotInput = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(missing.runtime_override, None);
        assert_eq!(missing.model_override, None);
        assert_eq!(missing.effort_override, None);

        let null: UpdateSlotInput = serde_json::from_str(r#"{"runtime_override": null}"#).unwrap();
        assert_eq!(null.runtime_override, Some(None));

        let set: UpdateSlotInput =
            serde_json::from_str(r#"{"runtime_override": "codex"}"#).unwrap();
        assert_eq!(set.runtime_override, Some(Some("codex".into())));

        let model_null: UpdateSlotInput =
            serde_json::from_str(r#"{"model_override": null}"#).unwrap();
        assert_eq!(model_null.model_override, Some(None));

        let model_set: UpdateSlotInput =
            serde_json::from_str(r#"{"model_override": "gpt-slot"}"#).unwrap();
        assert_eq!(model_set.model_override, Some(Some("gpt-slot".into())));

        let effort_null: UpdateSlotInput =
            serde_json::from_str(r#"{"effort_override": null}"#).unwrap();
        assert_eq!(effort_null.effort_override, Some(None));

        let effort_set: UpdateSlotInput =
            serde_json::from_str(r#"{"effort_override": "high"}"#).unwrap();
        assert_eq!(effort_set.effort_override, Some(Some("high".into())));
    }

    #[test]
    fn update_clears_override_from_wire_null() {
        // End-to-end for the clear action as the frontend/MCP send
        // it: a JSON body with an explicit null must clear the
        // stored override.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let s = create(&mut conn, &c, &r, "alpha", Some("codex"), None).unwrap();

        let input: UpdateSlotInput = serde_json::from_str(r#"{"runtime_override": null}"#).unwrap();
        let cleared = update(&mut conn, &s.slot.id, input).unwrap();
        assert_eq!(cleared.slot.runtime_override, None);
    }

    #[test]
    fn update_is_atomic_when_handle_collides() {
        // A combined patch (valid runtime + colliding handle) must
        // fail as a unit: the runtime change must not persist after
        // the handle update errors.
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r1 = seed_runner(&conn, "alpha");
        let r2 = seed_runner(&conn, "beta");
        create(&mut conn, &c, &r1, "alpha", None, None).unwrap();
        let b = create(&mut conn, &c, &r2, "beta", None, None).unwrap();

        let err = update(
            &mut conn,
            &b.slot.id,
            UpdateSlotInput {
                slot_handle: Some("alpha".into()),
                runtime_override: Some(Some("codex".into())),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already used"), "got: {err}");

        let roster = list(&conn, &c).unwrap();
        let b_after = roster.iter().find(|s| s.slot.id == b.slot.id).unwrap();
        assert_eq!(b_after.slot.slot_handle, "beta", "handle unchanged");
        assert_eq!(
            b_after.slot.runtime_override, None,
            "runtime change must roll back with the failed handle update",
        );
    }

    #[test]
    fn update_rejects_unknown_runtime_override() {
        let pool = pool();
        let mut conn = pool.get().unwrap();
        let c = seed_crew(&conn, "A");
        let r = seed_runner(&conn, "alpha");
        let s = create(&mut conn, &c, &r, "alpha", Some("codex"), None).unwrap();
        let err = update(
            &mut conn,
            &s.slot.id,
            UpdateSlotInput {
                runtime_override: Some(Some("aider-future".into())),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown runtime"), "got: {err}");
        // Rejection leaves the stored override untouched.
        let roster = list(&conn, &c).unwrap();
        assert_eq!(roster[0].slot.runtime_override.as_deref(), Some("codex"));
    }
}
