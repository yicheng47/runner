#[cfg(test)]
use crate::model::CodexSpeed;
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
use crate::router::runtime::PermissionMode;
use std::collections::HashMap;

use chrono::Utc;
use rusqlite::Connection;
use ulid::Ulid as UlidGen;

use crate::{
    error::{Error, Result},
    model::{Role, Timestamp},
    repo, AppCore,
};

pub use runner_core::protocol::role::CreateRoleInput;

pub use runner_core::protocol::role::default_permission_mode;

pub use runner_core::protocol::role::UpdateRoleInput;

pub use runner_core::protocol::role::RoleActivity;

pub use runner_core::protocol::role::RoleWithActivity;

fn new_id() -> String {
    UlidGen::new().to_string()
}

fn now() -> Timestamp {
    Utc::now()
}

/// Cap on `system_prompt` byte length. The composed first-user-turn
/// body (`compose_member_first_turn`) wraps this field plus ~1-3 KB
/// of preamble + goal + team conventions. The defense-in-depth ceiling in
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

/// A role's runtime, and a slot's override of it, must be an agent runtime:
/// one with a runtime definition. `shell` has none, because terminals build
/// their role in memory and never store one.
pub fn validate_agent_runtime(name: &str) -> Result<Runtime> {
    Runtime::parse(name)
        .filter(|runtime| crate::runtimes::adapter(*runtime).catalog().is_some())
        .ok_or_else(|| {
            Error::msg(format!(
                "unknown runtime '{name}' — valid runtimes: {}",
                crate::runtimes::catalogs()
                    .iter()
                    .map(|r| r.name.key())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
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
    validate_agent_runtime(input.runtime.key())?;
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
    let args = crate::runtimes::adapter(input.runtime)
        .permissions()
        .apply(&input.args, input.permission_mode);

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
            codex_speed: (crate::runtimes::adapter(input.runtime)
                .capabilities()
                .codex_speed)
                .then_some(input.codex_speed)
                .flatten(),
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
    if let Some(runtime) = input.runtime {
        validate_agent_runtime(runtime.key())?;
    }

    let display_name = input.display_name.unwrap_or(existing.display_name);
    // Snapshot the prior runtime *before* unwrap_or moves it, so a
    // runtime change can rewrite the old runtime's permission flags.
    let prior_runtime = existing.runtime.clone();
    let runtime = input
        .runtime
        .map(|runtime| runtime.to_string())
        .unwrap_or(existing.runtime);
    let runtime_changed = prior_runtime != runtime;
    let command = input.command.unwrap_or(existing.command);
    let prior_permissions = crate::runtimes::for_key(&prior_runtime).permissions();
    let permissions = crate::runtimes::for_key(&runtime).permissions();
    // A runtime change that sends neither a mode nor args keeps the mode the role shows,
    // falling back to Default where either runtime lacks it, as the Roles form does on a
    // switch (#1155). Args sent without a mode are stored as sent.
    let mode = input.permission_mode.or_else(|| {
        (runtime_changed && input.args.is_none()).then(|| {
            let mode = prior_permissions.infer(&existing.args);
            if prior_permissions.offered.contains(&mode) && permissions.offered.contains(&mode) {
                mode
            } else {
                PermissionMode::Default
            }
        })
    });
    let base = input.args.unwrap_or(existing.args);
    let args = match mode {
        Some(mode) => {
            // Before the prior runtime's strip, which can split another runtime's flags:
            // Grok's or Cursor's table takes `--sandbox workspace-write` out of Codex's.
            let cleaned = without_other_runtimes_mode_args(base, &runtime);
            let cleared = if runtime_changed {
                prior_permissions.strip(&cleaned)
            } else {
                cleaned
            };
            permissions.apply(&cleared, mode)
        }
        None => base,
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
    let codex_speed = if crate::runtimes::for_key(&runtime)
        .capabilities()
        .codex_speed
    {
        input.codex_speed.unwrap_or(existing.codex_speed)
    } else {
        None
    };

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
            codex_speed,
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

/// Drops other runtimes' permission flags exactly as their `apply` writes them, which a
/// runtime change without a mode used to leave behind (#1155). Matching the written flags,
/// not another runtime's strip table, keeps a flag this runtime accepts itself under a
/// shared name: Antigravity's bare `--sandbox`, or Copilot's and pi's `--mode <value>`.
fn without_other_runtimes_mode_args(mut args: Vec<String>, runtime: &str) -> Vec<String> {
    for other in Runtime::ALL
        .into_iter()
        .filter(|other| other.key() != runtime)
    {
        let permissions = crate::runtimes::adapter(other).permissions();
        for &mode in permissions.offered {
            let written = permissions.mode_args(mode);
            if written.is_empty() {
                continue;
            }
            while let Some(at) = args
                .windows(written.len())
                .position(|window| window == written.as_slice())
            {
                args.drain(at..at + written.len());
            }
        }
    }
    args
}

pub(crate) fn ensure_delete_allowed(conn: &Connection, id: &str) -> Result<()> {
    let handle = get(conn, id)?.handle;
    let crews = repo::role::crew_names(conn, id)?;
    if !crews.is_empty() {
        return Err(Error::msg(format!(
            "role @{handle} is used by crews; remove it from them before deleting this role: {}",
            crews.join(", ")
        )));
    }
    let session_ids = repo::role::unarchived_direct_session_ids(conn, id)?;
    if !session_ids.is_empty() {
        return Err(Error::msg(format!(
            "role @{handle} has unarchived chats; archive them before deleting this role: {}",
            session_ids.join(", ")
        )));
    }
    let missions = repo::role::unarchived_mission_names(conn, id)?;
    if !missions.is_empty() {
        return Err(Error::msg(format!(
            "role @{handle} is used by unarchived missions; archive them before deleting this role: {}",
            missions.join(", ")
        )));
    }
    Ok(())
}

// Global delete: refused while any crew has a slot for the role, so it
// never reshapes a crew. The row takes its remaining sessions with it:
// archived chats and archived missions.
pub fn delete(conn: &mut Connection, id: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure_delete_allowed(&tx, id)?;
    super::session::delete_rows(&tx, &repo::session::ids_for_role(&tx, id)?)?;
    if repo::role::delete(&tx, id)? != 1 {
        return Err(Error::msg(format!("role not found: {id}")));
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
mod tests;
