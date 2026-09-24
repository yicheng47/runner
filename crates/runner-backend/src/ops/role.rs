// Role CRUD — global scope (C5.5).
//
// A role is a reusable definition (handle, runtime, command, system
// prompt, ...) that can be referenced by zero or more crews via the
// `slots` join table (see commands/slot.rs). The handle is
// globally unique: @impl means the same role everywhere it appears in
// the event log.
//
// Lead/position invariants are per-crew and live in crew_role.rs. This
// module only owns the role rows themselves.

use crate::model::Runtime;
use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Role, Timestamp},
    repo, AppCore,
};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateRoleInput {
    pub handle: String,
    pub display_name: String,
    pub runtime: Runtime,
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
    /// Permission mode the role-edit form's dropdown chose. Mapped
    /// to concrete flags on the row's `args` column at create time
    /// via `router::runtime::apply_permission_mode`. Defaults to
    /// `Auto`. The new-role form defaults to codex, where Auto
    /// maps to `--ask-for-approval on-request --sandbox workspace-write`.
    /// For claude-code, Auto is still plan/model-gated; callers that
    /// default to claude-code should send AcceptEdits explicitly.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: crate::router::runtime::PermissionMode,
}

/// Default permission mode for new roles — `Auto`. Matches the
/// frontend's dropdown default and the seed's role args.
/// Pulled out so serde's `#[serde(default = "...")]` can name it.
pub(crate) fn default_permission_mode() -> crate::router::runtime::PermissionMode {
    crate::router::runtime::PermissionMode::Auto
}

// `handle` is intentionally excluded from updates: per arch §2.2 and §5.2
// the handle is the role template's identity in events, CLI
// addressing, and policy rules. Renaming after creation would break
// historical event attribution and any persisted policy references.
// Users who want a different handle delete the role and create a
// new one. (Per-slot in-crew identity lives on `slots.slot_handle`
// and is renameable.)
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UpdateRoleInput {
    pub display_name: Option<String>,
    pub runtime: Option<Runtime>,
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
pub struct RoleActivity {
    pub role_id: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    pub last_started_at: Option<Timestamp>,
    /// Most recent running direct-chat session for this role, if any.
    /// Lets the sidebar's SESSION list re-attach to a live PTY across page
    /// reloads — without this, the frontend `activeSessions` map starts
    /// empty on reload and we'd fall back to the role detail page.
    pub direct_session_id: Option<String>,
}

/// Role row plus its `RoleActivity`. Returned by `role_list_with_activity`
/// so the Roles list page can render every card's badges in one IPC round-
/// trip — without this the page would do N+1 calls (one `role_list` and
/// one `role_activity` per row), which also produces a flicker as
/// counters fill in.
#[derive(Debug, Clone, Serialize)]
pub struct RoleWithActivity {
    #[serde(flatten)]
    pub role: Role,
    #[serde(flatten)]
    pub activity: RoleActivity,
}

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

/// Cap on `system_prompt` byte length. The composed first-user-turn
/// body (`compose_launch_prompt` for a lead, `compose_worker_first_turn`
/// for a non-lead) wraps this field plus ~1-3 KB of preamble + goal +
/// roster + coordination. The defense-in-depth ceiling in
/// `router::runtime::first_turn_argv` is 32 KB; keeping
/// `system_prompt` ≤ 16 KB plus `mission_goal` ≤ 8 KB leaves the
/// composed body well under that ceiling so spawn-time argv
/// delivery is guaranteed to fit. See
/// `docs/impls/archive/0007-spawn-time-prompt-delivery.md`.
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 16 * 1024;

/// Reject `system_prompt` payloads that would exceed the
/// `first_turn_argv` budget once wrapped in the composed launch /
/// worker / direct-chat body. Persist-time validation keeps the
/// first-turn delivery path argv-only — there is no paste fallback
/// to rescue an oversized row, so the boundary check has to live
/// here.
pub(super) fn validate_system_prompt(prompt: Option<&str>) -> Result<()> {
    if let Some(p) = prompt {
        if p.len() > MAX_SYSTEM_PROMPT_BYTES {
            return Err(Error::msg(format!(
                "system_prompt is {} bytes; max {} ({} KB). Trim the brief or move \
                 long-form content into per-task instructions.",
                p.len(),
                MAX_SYSTEM_PROMPT_BYTES,
                MAX_SYSTEM_PROMPT_BYTES / 1024,
            )));
        }
    }
    Ok(())
}

// Handle validation: lowercase ASCII slug, 1..=32 chars, [a-z0-9] start,
// body [a-z0-9_-]. See `docs/arch/arch.md` §3.2 (Role — handle).
pub(super) fn validate_handle(handle: &str) -> Result<()> {
    if handle.is_empty() || handle.len() > 32 {
        return Err(Error::msg("role handle must be 1-32 chars"));
    }
    let bytes = handle.as_bytes();
    let first_ok = bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit();
    if !first_ok {
        return Err(Error::msg(
            "role handle must start with a lowercase letter or digit",
        ));
    }
    for b in bytes {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_';
        if !ok {
            return Err(Error::msg(
                "role handle must be lowercase letters, digits, '-' or '_'",
            ));
        }
    }
    Ok(())
}

/// Reject env var names that aren't POSIX shell identifiers. The native PTY
/// spawn path forwards every env entry to the child process. Validating at
/// persist time keeps bad names from entering the DB; the runtime re-checks
/// defensively so legacy rows surface a clear error instead of failing spawn.
pub(super) fn validate_env_keys<S: std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Result<()> {
    for k in env.keys() {
        if !crate::session::launch::is_valid_env_name(k) {
            return Err(Error::msg(format!(
                "env var name {k:?} is invalid: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        if crate::session::launch::is_reserved_env_name(k) {
            return Err(Error::msg(format!(
                "env var name {k:?} is reserved by the Runner launcher \
                 (the deterministic PATH must win over any per-role override; \
                 if you need extra dirs on PATH, configure them in your shell rc)"
            )));
        }
    }
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<Role>> {
    repo::role::list(conn).map_err(Into::into)
}

pub fn list_with_activity_page(
    conn: &Connection,
    page: i64,
    page_size: i64,
    query: &str,
) -> Result<super::ListPage<RoleWithActivity>> {
    let pattern = super::escaped_like_pattern(query);
    let total_count = repo::role::count(conn)?;
    let filtered_count = repo::role::count_matching(conn, &pattern)?;
    let (limit, offset) = super::page_limit_offset(page, page_size, filtered_count);
    let roles = repo::role::list_page(conn, &pattern, limit, offset)?;
    let mut items = Vec::with_capacity(roles.len());
    for role in roles {
        let activity = activity(conn, &role.id)?;
        items.push(RoleWithActivity { role, activity });
    }
    Ok(super::ListPage {
        items,
        total_count,
        filtered_count,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Role> {
    repo::role::get(conn, id)?.ok_or_else(|| Error::msg(format!("role not found: {id}")))
}

/// Look up a role by its `handle`. Used by `/roles/:handle` so the URL
/// stays stable across role-id rotations (the user thinks in handles,
/// not ULIDs). Handles are globally unique by schema, so this is exactly
/// 0 or 1 rows.
pub fn get_by_handle(conn: &Connection, handle: &str) -> Result<Role> {
    repo::role::get_by_handle(conn, handle)?
        .ok_or_else(|| Error::msg(format!("role not found: @{handle}")))
}

pub fn create(conn: &Connection, input: CreateRoleInput) -> Result<Role> {
    validate_handle(&input.handle)?;
    if input.display_name.trim().is_empty() {
        return Err(Error::msg("display_name must not be empty"));
    }
    validate_env_keys(&input.env)?;
    validate_system_prompt(input.system_prompt.as_deref())?;

    let id = new_id();
    let ts = now();
    // Apply the form's "Permission mode" segmented control to the
    // args column at create time so the canonical mode flags are
    // persisted on the row, not derived at spawn time. See
    // `router::runtime::apply_permission_mode`. No-op for runtimes
    // without a permission concept (the helper returns input
    // unchanged for shell/unknown).
    let args = crate::router::runtime::apply_permission_mode(
        Some(input.runtime),
        &input.args,
        input.permission_mode,
    );

    repo::role::insert(
        conn,
        &repo::role::RoleRow {
            id: id.clone(),
            handle: input.handle,
            display_name: input.display_name,
            runtime: input.runtime.to_string(),
            command: input.command,
            args_json: Some(args),
            working_dir: input.working_dir,
            system_prompt: input.system_prompt,
            env_json: Some(input.env),
            model: input
                .model
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            effort: input
                .effort
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            created_at: ts,
            updated_at: ts,
        },
    )?;
    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, input: UpdateRoleInput) -> Result<Role> {
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
    let runtime = input
        .runtime
        .map(|runtime| runtime.to_string())
        .unwrap_or(existing.runtime);
    let runtime_changed = prior_runtime != runtime;
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
            let cleared = if runtime_changed {
                crate::router::runtime::strip_permission_flags(
                    Runtime::parse(&prior_runtime),
                    &base,
                )
            } else {
                base
            };
            crate::router::runtime::apply_permission_mode(Runtime::parse(&runtime), &cleared, mode)
        }
        None => input.args.unwrap_or(existing.args),
    };
    let working_dir = input.working_dir.unwrap_or(existing.working_dir);
    let system_prompt = input.system_prompt.unwrap_or(existing.system_prompt);
    validate_system_prompt(system_prompt.as_deref())?;
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

    let tx = conn.unchecked_transaction()?;
    repo::role::update(
        &tx,
        &repo::role::RoleRow {
            id: id.to_string(),
            handle: existing.handle,
            display_name,
            runtime,
            command,
            args_json: Some(args),
            working_dir,
            system_prompt,
            env_json: Some(env),
            model,
            effort,
            created_at: existing.created_at,
            updated_at: now(),
        },
    )?;
    if runtime_changed {
        repo::role::clear_inheriting_slot_agent_overrides(&tx, id)?;
    }
    tx.commit()?;
    get(conn, id)
}

pub(crate) fn ensure_delete_allowed(conn: &Connection, id: &str) -> Result<()> {
    let session_ids = repo::role::unarchived_direct_session_ids(conn, id)?;
    if !session_ids.is_empty() {
        return Err(Error::msg(format!(
            "role {id} has unarchived chats; archive them before deleting this role: {}",
            session_ids.join(", ")
        )));
    }
    Ok(())
}

// Global delete: removes the role template row and lets the
// `ON DELETE CASCADE` on `slots` strip every slot that referenced
// the role. A single role template might have been referenced by
// multiple slots in the same crew (post-slot-redesign), so the
// cleanup runs per-crew, not per-slot.
//
// For any crew where one of the deleted slots was lead, auto-promote
// the lowest-position surviving slot so non-empty crews never end up
// leaderless. Then repack positions per-crew so survivors stay dense
// (0..N-1).
pub fn delete(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure_delete_allowed(&tx, id)?;

    // Distinct crews that referenced this role, plus whether ANY of
    // its slots in that crew was lead (so we know to auto-promote
    // after the cascade). Collected before the DELETE so we still
    // have the membership info.
    let affected_crews = repo::role::affected_crews(&tx, id)?;

    repo::role::delete_sessions(&tx, id)?;
    let affected = repo::role::delete(&tx, id)?;
    if affected != 1 {
        return Err(Error::msg(format!("role not found: {id}")));
    }
    // CASCADE fired: every slot row referencing this role is gone.

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
                repo::slot::promote_to_lead(&tx, &new_lead)?;
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

/// Activity stats for a role — how many sessions and missions it's
/// currently participating in, and when it last started a session. Used by
/// the Roles page to render "2 sessions · 1 mission" badges. Missions
/// are counted distinctly because a single role might have multiple
/// sessions in the same mission historically; in MVP that never happens
/// but the COUNT(DISTINCT) keeps us honest if it ever does.
pub fn activity(conn: &Connection, role_id: &str) -> Result<RoleActivity> {
    // Role must exist — fail loud so the caller's UI can render a proper
    // error rather than silently showing zero.
    get(conn, role_id)?;

    let activity = repo::role::activity(conn, role_id)?;
    let last_started_at =
        match activity.last_started_at {
            Some(s) => Some(s.parse::<Timestamp>().map_err(|e| {
                Error::msg(format!("failed to parse last_started_at timestamp: {e}"))
            })?),
            None => None,
        };

    Ok(RoleActivity {
        role_id: role_id.to_string(),
        active_sessions: activity.active_sessions,
        active_missions: activity.active_missions,
        crew_count: activity.crew_count,
        last_started_at,
        direct_session_id: activity.direct_session_id,
    })
}

// ---------------------------------------------------------------------
// State-level command bodies
// ---------------------------------------------------------------------

pub fn role_list(state: &AppCore) -> Result<Vec<Role>> {
    let conn = state.db.get()?;
    list(&conn)
}

pub fn role_list_with_activity(
    state: &AppCore,
    page: i64,
    page_size: i64,
    query: &str,
) -> Result<super::ListPage<RoleWithActivity>> {
    let conn = state.db.get()?;
    list_with_activity_page(&conn, page, page_size, query)
}

pub fn role_get(state: &AppCore, id: &str) -> Result<Role> {
    let conn = state.db.get()?;
    get(&conn, id)
}

pub fn role_get_by_handle(state: &AppCore, handle: &str) -> Result<Role> {
    let conn = state.db.get()?;
    get_by_handle(&conn, handle)
}

pub fn role_create(state: &AppCore, input: CreateRoleInput) -> Result<Role> {
    let conn = state.db.get()?;
    create(&conn, input)
}

pub fn role_update(state: &AppCore, id: &str, input: UpdateRoleInput) -> Result<Role> {
    let conn = state.db.get()?;
    update(&conn, id, input)
}

pub fn role_delete(state: &AppCore, id: &str) -> Result<()> {
    {
        let conn = state.db.get()?;
        ensure_delete_allowed(&conn, id)?;
    }
    // Reap every live PTY for this role before the DB delete. The
    // command-layer delete removes session rows explicitly, but the
    // in-memory SessionManager still owns child processes and reader
    // threads until they are killed.
    state.sessions.kill_all_for_role(id)?;
    let mut conn = state.db.get()?;
    delete(&mut conn, id)
}

pub fn role_activity(state: &AppCore, id: &str) -> Result<RoleActivity> {
    let conn = state.db.get()?;
    activity(&conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::router::runtime::PermissionMode;

    fn ctx() -> db::DbPool {
        db::open_in_memory().unwrap()
    }

    fn make(conn: &Connection, handle: &str) -> Role {
        create(
            conn,
            CreateRoleInput {
                handle: handle.into(),
                display_name: format!("{handle} display"),
                runtime: crate::model::Runtime::Shell,
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
    fn create_inserts_global_role_without_crew() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        assert_eq!(r.handle, "alpha");
    }

    #[test]
    fn runtime_json_inputs_reject_unknown_names() {
        let input = serde_json::json!({
            "handle": "agent", "display_name": "Agent", "command": "custom-cli",
            "runtime": "aider-future"
        });
        assert!(serde_json::from_value::<CreateRoleInput>(input).is_err());
        assert!(
            serde_json::from_value::<UpdateRoleInput>(serde_json::json!({
                "runtime": "aider-future"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<crate::mcp::tools::session::StartDirectSessionArgs>(
                serde_json::json!({
                    "role_id": "agent",
                    "runtime": "aider-future"
                })
            )
            .is_err()
        );
    }

    #[test]
    fn get_and_list_keep_runtime_json_strings() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let role = make(&conn, "agent");
        update(
            &conn,
            &role.id,
            UpdateRoleInput {
                runtime: Some(Runtime::Codex),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(get(&conn, &role.id).unwrap()).unwrap()["runtime"],
            "codex"
        );
        assert_eq!(
            serde_json::to_value(list(&conn).unwrap()).unwrap()[0]["runtime"],
            "codex"
        );
    }

    #[test]
    fn list_returns_all_roles_alphabetical() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        make(&conn, "bravo");
        make(&conn, "alpha");
        let roles = list(&conn).unwrap();
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0].handle, "alpha");
        assert_eq!(roles[1].handle, "bravo");
    }

    #[test]
    fn list_page_filters_handle_and_display_name_case_insensitively() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let role = create(
            &conn,
            CreateRoleInput {
                handle: "handle-needle".into(),
                display_name: "Display Needle".into(),
                runtime: Runtime::Codex,
                command: "/bin/command-needle".into(),
                args: vec!["--args-needle".into()],
                working_dir: Some("/tmp/working-dir-needle".into()),
                system_prompt: Some("System Prompt Needle".into()),
                env: HashMap::new(),
                model: Some("model-needle".into()),
                effort: Some("effort-needle".into()),
                permission_mode: PermissionMode::Auto,
            },
        )
        .unwrap();
        let mut row = repo::role::RoleRow::from(&role);
        row.runtime = "Runtime-Needle".into();
        repo::role::update(&conn, &row).unwrap();
        make(&conn, "decoy");

        for query in ["HANDLE-NEEDLE", "display needle"] {
            let page = list_with_activity_page(&conn, 1, 8, query).unwrap();
            assert_eq!(page.filtered_count, 1, "query {query:?}");
            assert_eq!(page.items[0].role.id, role.id, "query {query:?}");
        }

        for query in [
            "runtime-needle",
            "command-needle",
            "args-needle",
            "model-needle",
            "effort-needle",
            "working-dir-needle",
            "system prompt needle",
        ] {
            let page = list_with_activity_page(&conn, 1, 8, query).unwrap();
            assert_eq!(page.filtered_count, 0, "query {query:?}");
            assert!(page.items.is_empty(), "query {query:?}");
        }
    }

    #[test]
    fn list_page_treats_like_wildcards_as_literals() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let percent = make(&conn, "percent");
        update(
            &conn,
            &percent.id,
            UpdateRoleInput {
                display_name: Some("100% literal".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let underscore = make(&conn, "underscore");
        update(
            &conn,
            &underscore.id,
            UpdateRoleInput {
                display_name: Some("under_score literal".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let backslash = make(&conn, "backslash");
        update(
            &conn,
            &backslash.id,
            UpdateRoleInput {
                display_name: Some("back\\slash literal".into()),
                ..Default::default()
            },
        )
        .unwrap();
        make(&conn, "plain");

        let percent_page = list_with_activity_page(&conn, 1, 8, "%").unwrap();
        assert_eq!(percent_page.filtered_count, 1);
        assert_eq!(percent_page.items[0].role.id, percent.id);
        let underscore_page = list_with_activity_page(&conn, 1, 8, "_").unwrap();
        assert_eq!(underscore_page.filtered_count, 1);
        assert_eq!(underscore_page.items[0].role.id, underscore.id);
        let backslash_page = list_with_activity_page(&conn, 1, 8, "\\").unwrap();
        assert_eq!(backslash_page.filtered_count, 1);
        assert_eq!(backslash_page.items[0].role.id, backslash.id);
    }

    #[test]
    fn list_page_applies_limit_offset_and_clamps_empty_pages() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        for index in 0..10 {
            make(&conn, &format!("role-{index:02}"));
        }

        let second = list_with_activity_page(&conn, 2, 4, "").unwrap();
        assert_eq!(second.total_count, 10);
        assert_eq!(second.filtered_count, 10);
        assert_eq!(second.items.len(), 4);
        assert_eq!(second.items[0].role.handle, "role-04");
        assert_eq!(second.items[3].role.handle, "role-07");

        let clamped = list_with_activity_page(&conn, 99, 8, "").unwrap();
        assert_eq!(clamped.items.len(), 2);
        assert_eq!(clamped.items[0].role.handle, "role-08");
    }

    #[test]
    fn unique_handle_globally() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        make(&conn, "shared");
        let err = create(
            &conn,
            CreateRoleInput {
                handle: "shared".into(),
                display_name: "Dup".into(),
                runtime: crate::model::Runtime::Shell,
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
            UpdateRoleInput {
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
    fn update_runtime_clears_only_inheriting_slot_agent_overrides() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        crate::test_support::insert_test_slot(&conn, "s-inherit", "c1", &r.id, "inherit", 0, true);
        crate::test_support::insert_test_slot(&conn, "s-pinned", "c1", &r.id, "pinned", 1, false);
        repo::slot::set_model_override(&conn, "s-inherit", Some("old-model")).unwrap();
        repo::slot::set_effort_override(&conn, "s-inherit", Some("high")).unwrap();
        repo::slot::set_runtime_override(&conn, "s-pinned", Some("claude-code")).unwrap();
        repo::slot::set_model_override(&conn, "s-pinned", Some("opus")).unwrap();
        repo::slot::set_effort_override(&conn, "s-pinned", Some("max")).unwrap();

        update(
            &conn,
            &r.id,
            UpdateRoleInput {
                runtime: Some(crate::model::Runtime::Codex),
                command: Some("codex".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let overrides_for = |slot_id: &str| -> (Option<String>, Option<String>) {
            let slot = repo::slot::get(&conn, slot_id).unwrap().unwrap();
            (slot.model_override, slot.effort_override)
        };
        assert_eq!(overrides_for("s-inherit"), (None, None));
        assert_eq!(
            overrides_for("s-pinned"),
            (Some("opus".into()), Some("max".into())),
        );
    }

    #[test]
    fn delete_removes_row() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        delete(&mut conn, &r.id).unwrap();
        let count = repo::role::count(&conn).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_refuses_role_with_unarchived_direct_chat() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        let mut session = crate::test_support::test_session_row(
            "direct-live",
            crate::model::SessionStatus::Stopped,
        );
        session.role_id = Some(r.id.clone());
        repo::session::insert(&conn, &session).unwrap();

        let err = delete(&mut conn, &r.id).unwrap_err().to_string();
        assert!(err.contains("unarchived chats"));
        let role_count = i64::from(repo::role::get(&conn, &r.id).unwrap().is_some());
        let session_count = i64::from(
            repo::session::get_row(&conn, "direct-live")
                .unwrap()
                .is_some(),
        );
        assert_eq!(role_count, 1, "role must survive refused delete");
        assert_eq!(session_count, 1, "chat must survive refused delete");
    }

    #[test]
    fn delete_removes_role_sessions_before_role_row() {
        let pool = ctx();
        let mut conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-1', 'Crew', '2026-04-22T00:00:00Z',
                     '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission-1', 'crew-1', 'Mission', 'running',
                     '2026-04-22T00:00:00Z')",
            [],
        )
        .unwrap();
        let mut mission_session = crate::test_support::test_session_row(
            "mission-session",
            crate::model::SessionStatus::Running,
        );
        mission_session.mission_id = Some("mission-1".into());
        mission_session.role_id = Some(r.id.clone());
        mission_session.slot_id = Some("slot-1".into());
        repo::session::insert(&conn, &mission_session).unwrap();
        let mut archived_direct = crate::test_support::test_session_row(
            "archived-direct",
            crate::model::SessionStatus::Stopped,
        );
        archived_direct.role_id = Some(r.id.clone());
        archived_direct.archived_at = Some(chrono::Utc::now());
        repo::session::insert(&conn, &archived_direct).unwrap();

        delete(&mut conn, &r.id).unwrap();
        let session_count = ["mission-session", "archived-direct"]
            .into_iter()
            .filter(|id| repo::session::get_row(&conn, id).unwrap().is_some())
            .count();
        let mission_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM missions WHERE id = 'mission-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(session_count, 0, "role sessions are hard-deleted");
        assert_eq!(mission_count, 1, "role delete does not delete missions");
    }

    #[test]
    fn activity_direct_session_id_ignores_slot_bound_orphans() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        let mut session = crate::test_support::test_session_row(
            "slot-orphan",
            crate::model::SessionStatus::Running,
        );
        session.role_id = Some(r.id.clone());
        session.slot_id = Some("slot-old".into());
        repo::session::insert(&conn, &session).unwrap();

        let got = activity(&conn, &r.id).unwrap();
        assert_eq!(
            got.direct_session_id, None,
            "slot-bound orphan must not be treated as direct chat activity"
        );
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
        // role template stable if the recommended default ever
        // shifts (per #45 task 2).
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRoleInput {
                handle: "codex-tester".into(),
                display_name: "C".into(),
                runtime: crate::model::Runtime::Codex,
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
            CreateRoleInput {
                handle: "claude-tester".into(),
                display_name: "Claude".into(),
                runtime: crate::model::Runtime::ClaudeCode,
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
    fn create_trae_auto_omits_flags_and_update_bakes_bypass() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRoleInput {
                handle: "trae-tester".into(),
                display_name: "TRAE".into(),
                runtime: crate::model::Runtime::Trae,
                command: "traecli".into(),
                args: vec!["--debug".into()],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Auto,
            },
        )
        .unwrap();
        assert_eq!(r.args, vec!["--debug".to_string()]);

        let r = update(
            &conn,
            &r.id,
            UpdateRoleInput {
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            r.args,
            vec![
                "--debug".to_string(),
                "--permission-mode".to_string(),
                "bypass_permissions".to_string(),
            ],
        );
    }

    #[test]
    fn copilot_create_and_update_bake_only_the_selected_permission_mode() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let role = create(
            &conn,
            CreateRoleInput {
                handle: "copilot-tester".into(),
                display_name: "Copilot".into(),
                runtime: crate::model::Runtime::Copilot,
                command: "copilot".into(),
                args: vec![
                    "--debug".into(),
                    "--yolo".into(),
                    "--allow-tool".into(),
                    "shell".into(),
                ],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::AcceptEdits,
            },
        )
        .unwrap();
        assert_eq!(role.args, ["--debug", "--allow-tool=write"]);
        for (mode, expected) in [
            (PermissionMode::Bypass, vec!["--debug", "--yolo"]),
            (PermissionMode::Default, vec!["--debug"]),
            (PermissionMode::Auto, vec!["--debug"]),
        ] {
            let updated = update(
                &conn,
                &role.id,
                UpdateRoleInput {
                    permission_mode: Some(mode),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(updated.args, expected);
        }
    }

    #[test]
    fn antigravity_create_and_update_bake_only_the_selected_permission_mode() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let role = create(
            &conn,
            CreateRoleInput {
                handle: "agy-tester".into(),
                display_name: "Antigravity".into(),
                runtime: crate::model::Runtime::Antigravity,
                command: "agy".into(),
                args: vec![
                    "--sandbox".into(),
                    "-mode=plan".into(),
                    "-dangerously-skip-permissions".into(),
                ],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::AcceptEdits,
            },
        )
        .unwrap();
        assert_eq!(role.args, ["--sandbox", "--mode", "accept-edits"]);
        for (mode, expected) in [
            (
                PermissionMode::Bypass,
                vec!["--sandbox", "--dangerously-skip-permissions"],
            ),
            (PermissionMode::Default, vec!["--sandbox"]),
            (PermissionMode::Auto, vec!["--sandbox"]),
        ] {
            let updated = update(
                &conn,
                &role.id,
                UpdateRoleInput {
                    permission_mode: Some(mode),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(updated.args, expected);
        }
    }

    #[test]
    fn opencode_create_and_update_bake_only_auto_for_bypass() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let role = create(
            &conn,
            CreateRoleInput {
                handle: "opencode-tester".into(),
                display_name: "OpenCode".into(),
                runtime: crate::model::Runtime::OpenCode,
                command: "opencode".into(),
                args: vec![
                    "--agent".into(),
                    "build".into(),
                    "--yolo".into(),
                    "--dangerously-skip-permissions=true".into(),
                    "--no-auto".into(),
                ],
                working_dir: None,
                system_prompt: None,
                env: HashMap::new(),
                model: None,
                effort: None,
                permission_mode: PermissionMode::Bypass,
            },
        )
        .unwrap();
        assert_eq!(role.args, ["--agent", "build", "--auto"]);
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
        ] {
            let updated = update(
                &conn,
                &role.id,
                UpdateRoleInput {
                    permission_mode: Some(mode),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(updated.args, ["--agent", "build"], "{mode:?}");
        }
    }

    #[test]
    fn create_omits_bypass_flags_when_toggle_off() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = create(
            &conn,
            CreateRoleInput {
                handle: "paranoid".into(),
                display_name: "P".into(),
                runtime: crate::model::Runtime::Codex,
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
            CreateRoleInput {
                handle: "explicit".into(),
                display_name: "E".into(),
                runtime: crate::model::Runtime::ClaudeCode,
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
    fn create_no_op_for_runtimes_without_permission_modes() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        for (handle, runtime, command) in [
            ("shell-tester", crate::model::Runtime::Shell, "/bin/sh"),
            ("pi-tester", crate::model::Runtime::Pi, "pi"),
        ] {
            let r = create(
                &conn,
                CreateRoleInput {
                    handle: handle.into(),
                    display_name: handle.into(),
                    runtime,
                    command: command.into(),
                    args: vec!["--custom".into()],
                    working_dir: None,
                    system_prompt: None,
                    env: HashMap::new(),
                    model: None,
                    effort: None,
                    permission_mode: PermissionMode::Bypass,
                },
            )
            .unwrap();
            assert_eq!(r.args, vec!["--custom".to_string()]);
        }
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
            CreateRoleInput {
                handle: "codex-rt".into(),
                display_name: "C".into(),
                runtime: crate::model::Runtime::Codex,
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
            UpdateRoleInput {
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
            UpdateRoleInput {
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
            UpdateRoleInput {
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
            CreateRoleInput {
                handle: "claude-rt".into(),
                display_name: "C".into(),
                runtime: crate::model::Runtime::ClaudeCode,
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
            UpdateRoleInput {
                permission_mode: Some(PermissionMode::Default),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.args, vec!["--mcp-debug".to_string()]);

        let r = update(
            &conn,
            &r.id,
            UpdateRoleInput {
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
            CreateRoleInput {
                handle: "preserve".into(),
                display_name: "P".into(),
                runtime: crate::model::Runtime::Codex,
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
            UpdateRoleInput {
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
            CreateRoleInput {
                handle: "switcher".into(),
                display_name: "S".into(),
                runtime: crate::model::Runtime::ClaudeCode,
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
            UpdateRoleInput {
                runtime: Some(crate::model::Runtime::Codex),
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
    fn activity_counts_zero_for_brand_new_role() {
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
    fn create_rejects_system_prompt_over_cap() {
        // Plan 0007: validation at persist time keeps the composed
        // launch / worker / direct-chat body under
        // `router::runtime::FIRST_TURN_ARGV_MAX_BYTES` so spawn-time
        // argv delivery is guaranteed to fit. Reject oversized
        // `system_prompt` payloads here instead of relying on a
        // post-spawn paste fallback (which the plan retired).
        let pool = ctx();
        let conn = pool.get().unwrap();
        let oversized = "X".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
        let err = create(
            &conn,
            CreateRoleInput {
                handle: "too-long".into(),
                display_name: "T".into(),
                runtime: crate::model::Runtime::ClaudeCode,
                command: "claude".into(),
                args: vec![],
                working_dir: None,
                system_prompt: Some(oversized),
                env: Default::default(),
                model: None,
                effort: None,
                permission_mode: crate::router::runtime::PermissionMode::AcceptEdits,
            },
        )
        .expect_err("oversize system_prompt must be rejected");
        assert!(
            err.to_string().contains("system_prompt"),
            "error should mention system_prompt; got {err}"
        );
    }

    #[test]
    fn update_rejects_system_prompt_over_cap() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "victim");
        let oversized = "X".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
        let err = update(
            &conn,
            &r.id,
            UpdateRoleInput {
                display_name: None,
                runtime: None,
                command: None,
                args: None,
                working_dir: None,
                system_prompt: Some(Some(oversized)),
                env: None,
                model: None,
                effort: None,
                permission_mode: None,
            },
        )
        .expect_err("oversize system_prompt must be rejected on update");
        assert!(err.to_string().contains("system_prompt"));
    }

    #[test]
    fn activity_counts_running_sessions() {
        let pool = ctx();
        let conn = pool.get().unwrap();
        let r = make(&conn, "alpha");
        // Insert a running session by hand — C6 will own this path later.
        let mut session =
            crate::test_support::test_session_row("s1", crate::model::SessionStatus::Running);
        session.role_id = Some(r.id.clone());
        session.cwd = Some("/tmp".into());
        repo::session::insert(&conn, &session).unwrap();
        let a = activity(&conn, &r.id).unwrap();
        assert_eq!(a.active_sessions, 1);
        assert_eq!(a.active_missions, 0, "direct session has no mission");
        assert!(a.last_started_at.is_some());
    }
}
