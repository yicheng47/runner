// Runner CRUD — global scope (C5.5).
//
// A runner is a reusable definition (handle, runtime, command, system
// prompt, ...) that can be referenced by zero or more crews via the
// `slots` join table (see commands/slot.rs). The handle is
// globally unique: @impl means the same runner everywhere it appears in
// the event log.
//
// Lead/position invariants are per-crew and live in crew_runner.rs. This
// module only owns the runner rows themselves.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use tauri::State;
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Runner, Timestamp},
    AppState,
};

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRunnerInput {
    pub handle: String,
    pub display_name: String,
    pub runtime: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Permission mode the runner-edit form's dropdown chose. Mapped
    /// to concrete flags on the row's `args` column at create time
    /// via `router::runtime::apply_permission_mode`. Defaults to
    /// `AcceptEdits`:
    /// - claude-code → `--permission-mode acceptEdits`. Works on
    ///   every plan and avoids the consent dialog.
    /// - codex → no flag (codex has no edits-only middle on the
    ///   wire; `AcceptEdits` is treated as `Default` there).
    ///
    /// `Auto` is opt-in for claude-code because the real `auto`
    /// mode is plan/model-gated (Max/Team/Enterprise/API + a
    /// supported model). `Bypass` is opt-in because claude-code
    /// shows a one-time consent dialog the first time per user
    /// account. Hidden in the form for runtimes without a
    /// permission concept (shell / unknown).
    #[serde(default = "default_permission_mode")]
    pub permission_mode: crate::router::runtime::PermissionMode,
}

/// Default permission mode for new runners — `AcceptEdits`. Matches
/// the frontend's dropdown default and the seed's runner args.
/// Pulled out so serde's `#[serde(default = "...")]` can name it.
fn default_permission_mode() -> crate::router::runtime::PermissionMode {
    crate::router::runtime::PermissionMode::AcceptEdits
}

// `handle` is intentionally excluded from updates: per arch §2.2 and §5.2
// the handle is the runner template's identity in events, CLI
// addressing, and policy rules. Renaming after creation would break
// historical event attribution and any persisted policy references.
// Users who want a different handle delete the runner and create a
// new one. (Per-slot in-crew identity lives on `slots.slot_handle`
// and is renameable.)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateRunnerInput {
    pub display_name: Option<String>,
    pub runtime: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub working_dir: Option<Option<String>>,
    pub system_prompt: Option<Option<String>>,
    pub env: Option<HashMap<String, String>>,
    pub model: Option<Option<String>>,
    pub effort: Option<Option<String>>,
    /// Form's "Permission mode" segmented control. `Some(mode)`
    /// rewrites the runtime's permission flags to the canonical args
    /// for that mode (replacing any prior occurrence so duplicates
    /// can't accumulate). `None` preserves the args as-is — callers
    /// that don't surface the control (CLI patches, programmatic
    /// updates) shouldn't have to reason about it. See
    /// `router::runtime::apply_permission_mode`.
    pub permission_mode: Option<crate::router::runtime::PermissionMode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerActivity {
    pub runner_id: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    pub last_started_at: Option<Timestamp>,
    /// Most recent running direct-chat session for this runner, if any.
    /// Lets the sidebar's SESSION list re-attach to a live PTY across page
    /// reloads — without this, the frontend `activeSessions` map starts
    /// empty on reload and we'd fall back to the runner detail page.
    pub direct_session_id: Option<String>,
}

/// Runner row plus its `RunnerActivity`. Returned by `runner_list_with_activity`
/// so the Runners list page can render every card's badges in one IPC round-
/// trip — without this the page would do N+1 calls (one `runner_list` and
/// one `runner_activity` per row), which also produces a flicker as
/// counters fill in.
#[derive(Debug, Clone, Serialize)]
pub struct RunnerWithActivity {
    #[serde(flatten)]
    pub runner: Runner,
    #[serde(flatten)]
    pub activity: RunnerActivity,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

// Handle validation: lowercase ASCII slug, 1..=32 chars, [a-z0-9] start,
// body [a-z0-9_-]. Matches PRD §4 handle rules.
pub(super) fn validate_handle(handle: &str) -> Result<()> {
    if handle.is_empty() || handle.len() > 32 {
        return Err(Error::msg("runner handle must be 1-32 chars"));
    }
    let bytes = handle.as_bytes();
    let first_ok = bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit();
    if !first_ok {
        return Err(Error::msg(
            "runner handle must start with a lowercase letter or digit",
        ));
    }
    for b in bytes {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_';
        if !ok {
            return Err(Error::msg(
                "runner handle must be lowercase letters, digits, '-' or '_'",
            ));
        }
    }
    Ok(())
}

/// Reject env var names that aren't POSIX shell identifiers. The
/// session launch script (`session::launch::render_launch_script`)
/// emits `export <name>=<value>` for every env entry, and bash
/// errors out under `set -e` if `<name>` isn't `[A-Za-z_][A-Za-z0-9_]*`.
/// Validating at persist time keeps bad names from ever entering
/// the DB; the launch-script renderer also re-checks defensively
/// so legacy rows that pre-date this validation surface a clear
/// error rather than crashing the spawn.
pub(super) fn validate_env_keys<S: std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<()> {
    for k in env.keys() {
        if !crate::session::launch::is_valid_env_name(k) {
            return Err(Error::msg(format!(
                "env var name {k:?} is invalid: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
    }
    Ok(())
}

pub(super) fn row_to_runner(row: &Row<'_>) -> rusqlite::Result<Runner> {
    let args_raw: Option<String> = row.get("args_json")?;
    let env_raw: Option<String> = row.get("env_json")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    Ok(Runner {
        id: row.get("id")?,
        handle: row.get("handle")?,
        display_name: row.get("display_name")?,
        runtime: row.get("runtime")?,
        command: row.get("command")?,
        args: match args_raw {
            Some(s) => serde_json::from_str(&s).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            None => Vec::new(),
        },
        working_dir: row.get("working_dir")?,
        system_prompt: row.get("system_prompt")?,
        env: match env_raw {
            Some(s) => serde_json::from_str(&s).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            None => HashMap::new(),
        },
        model: row.get("model")?,
        effort: row.get("effort")?,
        created_at: created_at.parse().map_err(|e: chrono::ParseError| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
        updated_at: updated_at.parse().map_err(|e: chrono::ParseError| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
    })
}

pub(super) const SELECT_COLS: &str = "id, handle, display_name, runtime, command,
                                       args_json, working_dir, system_prompt, env_json,
                                       model, effort,
                                       created_at, updated_at";

pub fn list(conn: &Connection) -> Result<Vec<Runner>> {
    let sql = format!("SELECT {SELECT_COLS} FROM runners ORDER BY handle ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_runner)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// `list()` + `activity()` for every runner, in one IPC call. The Runners
/// list page calls this on mount so each card's "N sessions / M missions"
/// badge renders without a second-pass flicker. Activity is computed
/// per row rather than via one giant JOIN — there are at most a few
/// dozen runners and the queries are indexed; a JOIN would obscure the
/// fact that `activity()` is the canonical aggregation and the two paths
/// would drift over time.
pub fn list_with_activity(conn: &Connection) -> Result<Vec<RunnerWithActivity>> {
    let runners = list(conn)?;
    let mut out = Vec::with_capacity(runners.len());
    for runner in runners {
        let activity = activity(conn, &runner.id)?;
        out.push(RunnerWithActivity { runner, activity });
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Runner> {
    let sql = format!("SELECT {SELECT_COLS} FROM runners WHERE id = ?1");
    conn.query_row(&sql, params![id], row_to_runner)
        .optional()?
        .ok_or_else(|| Error::msg(format!("runner not found: {id}")))
}

/// Look up a runner by its `handle`. Used by `/runners/:handle` so the URL
/// stays stable across runner-id rotations (the user thinks in handles,
/// not ULIDs). Handles are globally unique by schema, so this is exactly
/// 0 or 1 rows.
pub fn get_by_handle(conn: &Connection, handle: &str) -> Result<Runner> {
    let sql = format!("SELECT {SELECT_COLS} FROM runners WHERE handle = ?1");
    conn.query_row(&sql, params![handle], row_to_runner)
        .optional()?
        .ok_or_else(|| Error::msg(format!("runner not found: @{handle}")))
}

pub fn create(conn: &Connection, input: CreateRunnerInput) -> Result<Runner> {
    validate_handle(&input.handle)?;
    if input.display_name.trim().is_empty() {
        return Err(Error::msg("display_name must not be empty"));
    }
    validate_env_keys(&input.env)?;

    let id = new_id();
    let ts = now().to_rfc3339();
    // Apply the form's "Permission mode" segmented control to the
    // args column at create time so the canonical mode flags are
    // persisted on the row, not derived at spawn time. See
    // `router::runtime::apply_permission_mode`. No-op for runtimes
    // without a permission concept (the helper returns input
    // unchanged for shell/unknown).
    let args = crate::router::runtime::apply_permission_mode(
        &input.runtime,
        &input.args,
        input.permission_mode,
    );
    let args_json = serde_json::to_string(&args)?;
    let env_json = serde_json::to_string(&input.env)?;

    conn.execute(
        "INSERT INTO runners (
            id, handle, display_name, runtime, command,
            args_json, working_dir, system_prompt, env_json,
            model, effort,
            created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        params![
            id,
            input.handle,
            input.display_name,
            input.runtime,
            input.command,
            args_json,
            input.working_dir,
            input.system_prompt,
            env_json,
            input
                .model
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
            input
                .effort
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty()),
            ts,
        ],
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: UpdateRunnerInput) -> Result<Runner> {
    let existing = get(conn, id)?;
    if let Some(ref n) = input.display_name {
        if n.trim().is_empty() {
            return Err(Error::msg("display_name must not be empty"));
        }
    }

    let display_name = input.display_name.unwrap_or(existing.display_name);
    // Snapshot the prior runtime *before* unwrap_or moves it, so
    // we can strip the old runtime's bypass flags when the patch
    // changes runtime alongside the toggle.
    let prior_runtime = existing.runtime.clone();
    let runtime = input.runtime.unwrap_or(existing.runtime);
    let command = input.command.unwrap_or(existing.command);
    // Compose the new args from the user-provided list (or the
    // existing one when the patch omits `args`) and the form's
    // "Permission mode" segmented control. `None` mode = leave args
    // alone so non-form callers don't have to think about
    // permission flags. When the mode is provided AND the runtime
    // is being changed in the same patch, we also strip the *prior*
    // runtime's permission flags so a switch doesn't leave orphans
    // (the control owns these flags per-runtime). See
    // `router::runtime::apply_permission_mode`.
    let args = match input.permission_mode {
        Some(mode) => {
            let base = input.args.unwrap_or(existing.args);
            let cleared = if prior_runtime != runtime {
                crate::router::runtime::strip_permission_flags(&prior_runtime, &base)
            } else {
                base
            };
            crate::router::runtime::apply_permission_mode(&runtime, &cleared, mode)
        }
        None => input.args.unwrap_or(existing.args),
    };
    let working_dir = input.working_dir.unwrap_or(existing.working_dir);
    let system_prompt = input.system_prompt.unwrap_or(existing.system_prompt);
    let env = match input.env {
        Some(new_env) => {
            validate_env_keys(&new_env)?;
            new_env
        }
        None => existing.env,
    };
    // Trim + collapse blank strings to NULL: the editor's text inputs
    // produce `Some("")` when the user clears the field, and we want
    // that to read as "inherit the agent's default" — same semantic
    // as the column being NULL.
    let model = input
        .model
        .map(|opt| {
            opt.and_then(|s| {
                let t = s.trim().to_string();
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            })
        })
        .unwrap_or(existing.model);
    let effort = input
        .effort
        .map(|opt| {
            opt.and_then(|s| {
                let t = s.trim().to_string();
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            })
        })
        .unwrap_or(existing.effort);

    let args_json = serde_json::to_string(&args)?;
    let env_json = serde_json::to_string(&env)?;
    let ts = now().to_rfc3339();

    conn.execute(
        "UPDATE runners
            SET display_name = ?1,
                runtime = ?2,
                command = ?3,
                args_json = ?4,
                working_dir = ?5,
                system_prompt = ?6,
                env_json = ?7,
                model = ?8,
                effort = ?9,
                updated_at = ?10
          WHERE id = ?11",
        params![
            display_name,
            runtime,
            command,
            args_json,
            working_dir,
            system_prompt,
            env_json,
            model,
            effort,
            ts,
            id,
        ],
    )?;
    get(conn, id)
}

// Global delete: removes the runner template row and lets the
// `ON DELETE CASCADE` on `slots` strip every slot that referenced
// the runner. A single runner template might have been referenced by
// multiple slots in the same crew (post-slot-redesign), so the
// cleanup runs per-crew, not per-slot.
//
// For any crew where one of the deleted slots was lead, auto-promote
// the lowest-position surviving slot so non-empty crews never end up
// leaderless. Then repack positions per-crew so survivors stay dense
// (0..N-1).
pub fn delete(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // Distinct crews that referenced this runner, plus whether ANY of
    // its slots in that crew was lead (so we know to auto-promote
    // after the cascade). Collected before the DELETE so we still
    // have the membership info.
    let affected_crews: Vec<(String, bool)> = {
        let mut stmt = tx.prepare(
            "SELECT crew_id, MAX(lead)
               FROM slots
              WHERE runner_id = ?1
              GROUP BY crew_id",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let affected = tx.execute("DELETE FROM runners WHERE id = ?1", params![id])?;
    if affected != 1 {
        return Err(Error::msg(format!("runner not found: {id}")));
    }
    // CASCADE fired: every slot row referencing this runner is gone.

    for (crew_id, had_lead) in affected_crews {
        if had_lead {
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
                tx.execute("UPDATE slots SET lead = 1 WHERE id = ?1", params![new_lead])?;
            }
        }
        // Close the position gap the cascade left for this crew so
        // survivors stay dense (0..N-1) and the next `slot::create`
        // lands at a contiguous position.
        super::slot::repack_positions(&tx, &crew_id)?;
    }

    tx.commit()?;
    Ok(())
}

/// Activity stats for a runner — how many sessions and missions it's
/// currently participating in, and when it last started a session. Used by
/// the Runners page to render "2 sessions · 1 mission" badges. Missions
/// are counted distinctly because a single runner might have multiple
/// sessions in the same mission historically; in MVP that never happens
/// but the COUNT(DISTINCT) keeps us honest if it ever does.
pub fn activity(conn: &Connection, runner_id: &str) -> Result<RunnerActivity> {
    // Runner must exist — fail loud so the caller's UI can render a proper
    // error rather than silently showing zero.
    get(conn, runner_id)?;

    let active_sessions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE runner_id = ?1 AND status = 'running'",
        params![runner_id],
        |r| r.get(0),
    )?;
    let active_missions: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT mission_id) FROM sessions
          WHERE runner_id = ?1 AND status = 'running' AND mission_id IS NOT NULL",
        params![runner_id],
        |r| r.get(0),
    )?;
    let crew_count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT crew_id) FROM slots WHERE runner_id = ?1",
        params![runner_id],
        |r| r.get(0),
    )?;
    let last_started_at_raw: Option<String> = conn
        .query_row(
            "SELECT MAX(started_at) FROM sessions WHERE runner_id = ?1",
            params![runner_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let last_started_at =
        match last_started_at_raw {
            Some(s) => Some(s.parse::<Timestamp>().map_err(|e| {
                Error::msg(format!("failed to parse last_started_at timestamp: {e}"))
            })?),
            None => None,
        };
    let direct_session_id: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions
              WHERE runner_id = ?1 AND status = 'running' AND mission_id IS NULL
              ORDER BY started_at DESC
              LIMIT 1",
            params![runner_id],
            |r| r.get(0),
        )
        .optional()?;

    Ok(RunnerActivity {
        runner_id: runner_id.to_string(),
        active_sessions,
        active_missions,
        crew_count,
        last_started_at,
        direct_session_id,
    })
}

// ---------------------------------------------------------------------
// Tauri command wrappers
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn runner_list(state: State<'_, AppState>) -> Result<Vec<Runner>> {
    let conn = state.db.get()?;
    list(&conn)
}

#[tauri::command]
pub async fn runner_list_with_activity(
    state: State<'_, AppState>,
) -> Result<Vec<RunnerWithActivity>> {
    let conn = state.db.get()?;
    list_with_activity(&conn)
}

#[tauri::command]
pub async fn runner_get(state: State<'_, AppState>, id: String) -> Result<Runner> {
    let conn = state.db.get()?;
    get(&conn, &id)
}

#[tauri::command]
pub async fn runner_get_by_handle(state: State<'_, AppState>, handle: String) -> Result<Runner> {
    let conn = state.db.get()?;
    get_by_handle(&conn, &handle)
}

#[tauri::command]
pub async fn runner_create(state: State<'_, AppState>, input: CreateRunnerInput) -> Result<Runner> {
    let conn = state.db.get()?;
    create(&conn, input)
}

#[tauri::command]
pub async fn runner_update(
    state: State<'_, AppState>,
    id: String,
    input: UpdateRunnerInput,
) -> Result<Runner> {
    let conn = state.db.get()?;
    update(&conn, &id, input)
}

#[tauri::command]
pub async fn runner_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    // Reap every live PTY for this runner BEFORE the DB delete.
    // `sessions.runner_id` is `ON DELETE CASCADE`, so the row drop nukes
    // the session record — but the in-memory SessionManager still holds
    // the live child + reader thread. Without `kill_all_for_runner`, the
    // PTY lingers as a daemon attached to nothing and the Mac's TTY count
    // climbs every time the user deletes a runner with an open chat.
    state.sessions.kill_all_for_runner(&id)?;
    let mut conn = state.db.get()?;
    delete(&mut conn, &id)
}

#[tauri::command]
pub async fn runner_activity(state: State<'_, AppState>, id: String) -> Result<RunnerActivity> {
    let conn = state.db.get()?;
    activity(&conn, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::router::runtime::PermissionMode;

    fn ctx() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    fn make(conn: &Connection, handle: &str) -> Runner {
        create(
            conn,
            CreateRunnerInput {
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
                // Auto is the form default, but a no-op for shell — the
                // runtime adapter has no permission concept here, so
                // existing tests that expect `args == []` keep passing.
                permission_mode: PermissionMode::Auto,
            },
        )
        .unwrap()
    }

    #[test]
    fn create_inserts_global_runner_without_crew() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        assert_eq!(r.handle, "alpha");
    }

    #[test]
    fn list_returns_all_runners_alphabetical() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        make(&conn, "bravo");
        make(&conn, "alpha");
        let runners = list(&conn).unwrap();
        assert_eq!(runners.len(), 2);
        assert_eq!(runners[0].handle, "alpha");
        assert_eq!(runners[1].handle, "bravo");
    }

    #[test]
    fn unique_handle_globally() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        make(&conn, "shared");
        let err = create(
            &conn,
            CreateRunnerInput {
                handle: "shared".into(),
                display_name: "Dup".into(),
                runtime: "shell".into(),
                command: "sh".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Auto,
            },
        )
        .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("unique"));
    }

    #[test]
    fn update_preserves_unset_fields() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        let updated = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                display_name: Some("renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.display_name, "renamed");
        assert_eq!(updated.handle, r.handle, "handle is unaffected by update");
        assert_eq!(updated.runtime, r.runtime, "unchanged field preserved");
    }

    #[test]
    fn delete_removes_row() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        delete(&mut conn, &r.id).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_on_missing_id_errors_cleanly() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let err = delete(&mut conn, "does-not-exist").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn handle_must_be_lowercase_slug() {
        assert!(validate_handle("lead").is_ok());
        assert!(validate_handle("impl-1").is_ok());
        assert!(validate_handle("worker_2").is_ok());
        assert!(validate_handle("0worker").is_ok());

        assert!(validate_handle("").is_err());
        assert!(validate_handle("Lead").is_err());
        assert!(validate_handle("lead bot").is_err());
        assert!(validate_handle("lead!").is_err());
        assert!(validate_handle("-lead").is_err());
        assert!(validate_handle(&"x".repeat(33)).is_err());
    }

    #[test]
    fn create_applies_codex_bypass_flags_by_default() {
        // Form's "Skip approval prompts" toggle defaults to on.
        // For codex, that means the canonical
        // `--ask-for-approval never --sandbox workspace-write` pair
        // lands on the `args` column at create time — keeping the
        // runner template stable if the recommended default ever
        // shifts (per #45 task 2).
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "codex-tester".into(),
                display_name: "C".into(),
                runtime: "codex".into(),
                command: "codex".into(),
                args: vec!["--debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--debug".to_string(),
                "--ask-for-approval".to_string(),
                "never".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );
    }

    #[test]
    fn create_applies_claude_code_bypass_flag_by_default() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "claude-tester".into(),
                display_name: "Claude".into(),
                runtime: "claude-code".into(),
                command: "claude".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );
    }

    #[test]
    fn create_omits_bypass_flags_when_toggle_off() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "paranoid".into(),
                display_name: "P".into(),
                runtime: "codex".into(),
                command: "codex".into(),
                args: vec!["--debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Default,
            },
        )
        .unwrap();
        assert_eq!(r.args, vec!["--debug".to_string()]);
    }

    #[test]
    fn create_does_not_duplicate_existing_bypass_flags() {
        // CLI / API users who pass a permission flag themselves AND
        // pick a permission_mode shouldn't end up with both shapes on
        // the row. The strip-and-replace round-trip drops the
        // user-supplied flag (including the legacy
        // `--dangerously-skip-permissions` shape) and writes the
        // canonical `--permission-mode <value>` form for the chosen
        // mode.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "explicit".into(),
                display_name: "E".into(),
                runtime: "claude-code".into(),
                command: "claude".into(),
                args: vec!["--dangerously-skip-permissions".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
            "legacy flag stripped; canonical --permission-mode shape written",
        );
    }

    #[test]
    fn create_no_op_for_runtime_without_bypass_concept() {
        // shell has no bypass flags. Toggle on is a no-op — the args
        // column matches what the caller passed verbatim.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "shell-tester".into(),
                display_name: "Sh".into(),
                runtime: "shell".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "echo hi".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Auto,
            },
        )
        .unwrap();
        assert_eq!(r.args, vec!["-c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn update_skip_approval_toggle_round_trips_for_codex() {
        // Off → strips both halves of the codex bypass pair without
        // touching the unrelated user arg. On → re-adds them.
        // No duplicates accumulate across multiple round-trips.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "codex-rt".into(),
                display_name: "C".into(),
                runtime: "codex".into(),
                command: "codex".into(),
                args: vec!["--debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert!(r.args.contains(&"--ask-for-approval".to_string()));

        // Cycle to Default — strips both halves of the codex pair
        // without touching the unrelated user arg.
        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec!["--debug".to_string()],
            "Default mode must strip both --ask-for-approval and --sandbox cleanly",
        );

        // Cycle back to Bypass.
        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--debug".to_string(),
                "--ask-for-approval".to_string(),
                "never".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );

        // Re-applying Bypass a second time must not double up.
        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args
                .iter()
                .filter(|a| a.as_str() == "--ask-for-approval")
                .count(),
            1,
            "re-applying Bypass must not duplicate flags: {:?}",
            r.args,
        );
    }

    #[test]
    fn update_skip_approval_toggle_round_trips_for_claude_code() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "claude-rt".into(),
                display_name: "C".into(),
                runtime: "claude-code".into(),
                command: "claude".into(),
                args: vec!["--mcp-debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );

        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.args, vec!["--mcp-debug".to_string()]);

        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );
    }

    #[test]
    fn update_without_toggle_field_preserves_args_verbatim() {
        // Programmatic patches that don't surface the toggle (e.g. a
        // CLI patch that only updates `display_name`) must not
        // accidentally rewrite the args column. `None` toggle = no-op.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "preserve".into(),
                display_name: "P".into(),
                runtime: "codex".into(),
                command: "codex".into(),
                args: vec!["--debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        let before = r.args.clone();

        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                display_name: Some("renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args, before,
            "args must be untouched when no toggle is sent"
        );
        assert_eq!(r.display_name, "renamed");
    }

    #[test]
    fn update_runtime_switch_strips_prior_bypass_flags() {
        // Switching runtime alongside the toggle must also clean up
        // the old runtime's bypass flags so they don't survive as
        // orphans on the new runtime.
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRunnerInput {
                handle: "switcher".into(),
                display_name: "S".into(),
                runtime: "claude-code".into(),
                command: "claude".into(),
                args: vec![],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );

        // Switch to codex with the same mode. Old (claude-code)
        // flag pair must be stripped, new (codex) flag pair must be
        // applied.
        let r = update(
            &conn,
            &r.id,
            UpdateRunnerInput {
                runtime: Some("codex".into()),
                command: Some("codex".into()),
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !r.args.contains(&"--permission-mode".to_string()),
            "claude-code's flag must be stripped on runtime switch: {:?}",
            r.args,
        );
        assert!(
            r.args.contains(&"--ask-for-approval".to_string()),
            "codex bypass pair must be applied on runtime switch: {:?}",
            r.args,
        );
    }

    #[test]
    fn activity_counts_zero_for_brand_new_runner() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        let a = activity(&conn, &r.id).unwrap();
        assert_eq!(a.active_sessions, 0);
        assert_eq!(a.active_missions, 0);
        assert_eq!(a.crew_count, 0);
        assert!(a.last_started_at.is_none());
    }

    #[test]
    fn activity_counts_running_sessions() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        // Insert a running session by hand — C6 will own this path later.
        conn.execute(
            "INSERT INTO sessions (id, mission_id, runner_id, cwd, status, started_at)
             VALUES ('s1', NULL, ?1, '/tmp', 'running', '2026-04-23T00:00:00Z')",
            params![r.id],
        )
        .unwrap();
        let a = activity(&conn, &r.id).unwrap();
        assert_eq!(a.active_sessions, 1);
        assert_eq!(a.active_missions, 0, "direct session has no mission");
        assert!(a.last_started_at.is_some());
    }
}
