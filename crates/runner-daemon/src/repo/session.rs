// `sessions` table — one PTY run (mission slot or direct chat).
//
// Named `SessionRowDb` to stay clear of the `ops::session::SessionRow`
// IPC DTO. The launch definition is mapped to versioned JSON only here.
//
// The write functions mirror the legacy statements one-for-one — status
// flips, pid/stopped_at updates, key capture — because several run on the
// PTY hot path where statement shape and timing are load-bearing. Do not
// consolidate them.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_rusqlite::{from_row, to_params_named};

use crate::model::{CodexSpeed, LaunchDefinition, Session, SessionStatus, Timestamp};

use super::{de_err, insert_sql, qualified_select_list, select_list, ser_err};

#[derive(Serialize, Deserialize)]
struct LaunchDefinitionRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude_account: Option<runner_core::protocol::ClaudeSessionAccount>,
    version: u32,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    model: Option<String>,
    effort: Option<String>,
    runtime_options: serde_json::Value,
    system_prompt: Option<String>,
}

impl From<&LaunchDefinition> for LaunchDefinitionRow {
    fn from(definition: &LaunchDefinition) -> Self {
        Self {
            version: 1,
            claude_account: definition.claude_account.clone(),
            command: definition.command.clone(),
            args: definition.args.clone(),
            env: definition.env.clone(),
            model: definition.model.clone(),
            effort: definition.effort.clone(),
            runtime_options: super::serde::runtime_speed_json::value(definition.codex_speed)
                .map(|json| serde_json::from_str(&json).expect("runtime options serialize"))
                .unwrap_or_else(|| serde_json::json!({})),
            system_prompt: definition.system_prompt.clone(),
        }
    }
}

impl TryFrom<LaunchDefinitionRow> for LaunchDefinition {
    type Error = String;

    fn try_from(row: LaunchDefinitionRow) -> Result<Self, Self::Error> {
        if row.version != 1 {
            return Err(format!(
                "unsupported launch definition version: {}",
                row.version
            ));
        }
        let codex_speed: Option<CodexSpeed> = serde_json::from_value(
            row.runtime_options
                .get("codex")
                .and_then(|codex| codex.get("speed"))
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            claude_account: row.claude_account,
            command: row.command,
            args: row.args,
            env: row.env,
            model: row.model,
            effort: row.effort,
            codex_speed,
            system_prompt: row.system_prompt,
        })
    }
}

mod launch_definition_json {
    use super::{LaunchDefinition, LaunchDefinitionRow};
    use serde::{de, ser, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<LaunchDefinition>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let json = value
            .as_ref()
            .map(|definition| serde_json::to_string(&LaunchDefinitionRow::from(definition)))
            .transpose()
            .map_err(ser::Error::custom)?;
        serde::Serialize::serialize(&json, serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<LaunchDefinition>, D::Error> {
        let json = Option::<String>::deserialize(deserializer)?;
        json.map(|json| {
            let row: LaunchDefinitionRow =
                serde_json::from_str(&json).map_err(de::Error::custom)?;
            LaunchDefinition::try_from(row).map_err(de::Error::custom)
        })
        .transpose()
    }
}

const RESUME_ON_LAUNCH_PENDING: i64 = 1;
const RESUME_ON_LAUNCH_CLAIMED: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeOnLaunchClaim {
    pub session_id: String,
    pub shell: bool,
}
const RESUMABLE_AT_QUIT_PREDICATE: &str = "
    archived_at IS NULL
    AND (
        mission_id IS NULL
        OR (
            slot_id IS NOT NULL
            AND EXISTS (
                SELECT 1
                  FROM missions
                 WHERE missions.id = sessions.mission_id
                   AND missions.status = 'running'
            )
        )
    )";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRowDb {
    pub id: String,
    pub mission_id: Option<String>,
    pub project_id: Option<String>,
    pub role_id: Option<String>,
    pub slot_id: Option<String>,
    pub cwd: Option<String>,
    pub status: SessionStatus,
    pub pid: Option<i64>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub started_at: Option<Timestamp>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub stopped_at: Option<Timestamp>,
    pub agent_session_key: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub archived_at: Option<Timestamp>,
    pub title: Option<String>,
    pub live_title: Option<String>,
    #[serde(with = "crate::repo::serde::rfc3339_opt")]
    pub pinned_at: Option<Timestamp>,
    pub agent_runtime: Option<String>,
    pub agent_command: Option<String>,
    #[serde(rename = "launch_definition_json", with = "launch_definition_json")]
    pub launch_definition: Option<LaunchDefinition>,
    pub last_cols: Option<u16>,
    pub last_rows: Option<u16>,
    /// Nonzero for both pending and in-flight launch-resume work.
    pub resume_on_launch: bool,
}

impl SessionRowDb {
    /// Fresh `running` row with every optional column NULL — spawn sites
    /// fill in what they persist and leave the rest.
    pub fn new_running(id: String) -> Self {
        SessionRowDb {
            id,
            mission_id: None,
            project_id: None,
            role_id: None,
            slot_id: None,
            cwd: None,
            status: SessionStatus::Running,
            pid: None,
            started_at: None,
            stopped_at: None,
            agent_session_key: None,
            archived_at: None,
            title: None,
            live_title: None,
            pinned_at: None,
            agent_runtime: None,
            agent_command: None,
            launch_definition: None,
            last_cols: None,
            last_rows: None,
            resume_on_launch: false,
        }
    }
}

pub const COLUMNS: &[&str] = &[
    "id",
    "mission_id",
    "project_id",
    "role_id",
    "slot_id",
    "cwd",
    "status",
    "pid",
    "started_at",
    "stopped_at",
    "agent_session_key",
    "archived_at",
    "title",
    "live_title",
    "pinned_at",
    "agent_runtime",
    "agent_command",
    "launch_definition_json",
    "last_cols",
    "last_rows",
    "resume_on_launch",
];

pub fn insert(conn: &Connection, row: &SessionRowDb) -> rusqlite::Result<()> {
    conn.execute(
        &insert_sql("sessions", COLUMNS),
        to_params_named(row).map_err(ser_err)?.to_slice().as_slice(),
    )?;
    Ok(())
}

pub fn exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
        rusqlite::params![id],
        |row| row.get(0),
    )
}

pub fn get_row(conn: &Connection, id: &str) -> rusqlite::Result<Option<SessionRowDb>> {
    let sql = format!(
        "SELECT {} FROM sessions WHERE id = ?1",
        select_list(COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![id], |row| {
        from_row::<SessionRowDb>(row).map_err(de_err)
    })
    .optional()
}

/// Runtime identity recorded when the session was launched.
pub fn effective_runtime(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT s.agent_runtime
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.id = ?1",
        rusqlite::params![id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(Option::flatten)
}

pub fn delete_many(conn: &Connection, ids: &[String]) -> rusqlite::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    conn.execute(
        &format!("DELETE FROM sessions WHERE id IN ({placeholders})"),
        rusqlite::params_from_iter(ids),
    )
}

pub fn ids_for_missions(
    conn: &Connection,
    mission_ids: &[String],
) -> rusqlite::Result<Vec<String>> {
    if mission_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; mission_ids.len()].join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM sessions WHERE mission_id IN ({placeholders})"
    ))?;
    let ids = stmt.query_map(rusqlite::params_from_iter(mission_ids), |row| row.get(0))?;
    ids.collect()
}

pub fn ids_for_role(conn: &Connection, role_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM sessions WHERE role_id = ?1")?;
    let ids = stmt.query_map(rusqlite::params![role_id], |row| row.get(0))?;
    ids.collect()
}

pub fn unbind_project(conn: &Connection, project_id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
        rusqlite::params![project_id],
    )
}

/// The newest unarchived session a mission slot has had.
pub fn latest_unarchived_id_for_slot(
    conn: &Connection,
    mission_id: &str,
    slot_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM sessions
          WHERE mission_id = ?1 AND slot_id = ?2 AND archived_at IS NULL
          ORDER BY started_at DESC
          LIMIT 1",
        rusqlite::params![mission_id, slot_id],
        |row| row.get(0),
    )
    .optional()
}

/// How many of a mission's unarchived sessions there are, and how many of
/// them are running.
pub fn unarchived_counts_for_mission(
    conn: &Connection,
    mission_id: &str,
) -> rusqlite::Result<(usize, usize)> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0)
           FROM sessions
          WHERE mission_id = ?1
            AND archived_at IS NULL",
        rusqlite::params![mission_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// An unarchived session's persisted lifecycle and attention stamps.
pub struct StatusRow {
    pub id: String,
    pub status: String,
    pub stopped_at: Option<String>,
    pub unread_since: Option<i64>,
    pub error_acknowledged_at: Option<String>,
}

pub fn status_rows(conn: &Connection) -> rusqlite::Result<Vec<StatusRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.status, s.stopped_at, a.unread_since, a.error_acknowledged_at
           FROM sessions s
           LEFT JOIN session_attention a ON a.session_id = s.id
          WHERE s.archived_at IS NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(StatusRow {
            id: row.get(0)?,
            status: row.get(1)?,
            stopped_at: row.get(2)?,
            unread_since: row.get(3)?,
            error_acknowledged_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn set_launch(
    conn: &Connection,
    id: &str,
    runtime: &str,
    definition: &LaunchDefinition,
) -> rusqlite::Result<usize> {
    let json = serde_json::to_string(&LaunchDefinitionRow::from(definition))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        "UPDATE sessions SET agent_runtime = ?2, launch_definition_json = ?3 WHERE id = ?1",
        rusqlite::params![id, runtime, json],
    )
}

pub fn record_process(
    conn: &Connection,
    id: &str,
    pid: Option<i32>,
    executable: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET pid = ?2, agent_command = ?3 WHERE id = ?1",
        rusqlite::params![id, pid, executable],
    )
}

pub fn update_last_size(
    conn: &Connection,
    id: &str,
    cols: u16,
    rows: u16,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET last_cols = ?2,
                last_rows = ?3
          WHERE id = ?1",
        rusqlite::params![id, cols, rows],
    )
}

/// Resume an existing row in place: same id, same conversation thread.
/// A key the resume plan assigned wins; otherwise the stored key is kept.
/// A replacement launch is committed with its key and generation before spawn.
pub fn resume_in_place(
    conn: &Connection,
    id: &str,
    started_at: Timestamp,
    assigned_key: Option<&str>,
    fresh: bool,
    size: (u16, u16),
    launch: Option<(&str, &LaunchDefinition)>,
) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let updated = tx.execute(
        "UPDATE sessions
            SET status = 'running',
                pid = NULL,
                started_at = ?2,
                stopped_at = NULL,
                agent_session_key = CASE WHEN ?6 THEN ?3 ELSE COALESCE(?3, agent_session_key) END,
                live_title = CASE WHEN ?6 THEN NULL ELSE live_title END,
                last_cols = ?4,
                last_rows = ?5
          WHERE id = ?1",
        rusqlite::params![
            id,
            started_at.to_rfc3339(),
            assigned_key,
            size.0,
            size.1,
            fresh
        ],
    )?;
    if let Some((runtime, definition)) = launch {
        set_launch(&tx, id, runtime, definition)?;
    }
    tx.commit()?;
    Ok(updated)
}

/// Terminal status flip after a PTY exits (or a pending spawn is
/// cancelled / fails). `status` is `stopped` or `crashed`.
pub fn set_exit_status(
    conn: &Connection,
    id: &str,
    status: SessionStatus,
    stopped_at: Timestamp,
) -> rusqlite::Result<usize> {
    let status = match status {
        SessionStatus::Running => "running",
        SessionStatus::Stopped => "stopped",
        SessionStatus::Crashed => "crashed",
    };
    conn.execute(
        "UPDATE sessions
            SET status = ?1, stopped_at = ?2
          WHERE id = ?3",
        rusqlite::params![status, stopped_at.to_rfc3339(), id],
    )
}

/// Resume-failure flip: the prior conversation was rejected, so the row
/// crashes AND forgets its key so the next launch starts fresh.
pub fn set_crashed_clearing_key(
    conn: &Connection,
    id: &str,
    stopped_at: Timestamp,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET status = ?1, stopped_at = ?2,
                agent_session_key = NULL
          WHERE id = ?3",
        rusqlite::params!["crashed", stopped_at.to_rfc3339(), id],
    )
}

/// Guarded capture of a codex conversation key. `agent_session_key IS
/// NULL` so a concurrent writer's key is never clobbered; `started_at`
/// equality so a stale watcher from a prior incarnation of the row can't
/// write into a later stop/resume. Returns whether the row was updated.
pub fn capture_agent_session_key(
    conn: &Connection,
    id: &str,
    agent_session_key: &str,
    expected_row_started_at: &str,
) -> rusqlite::Result<bool> {
    conn.execute(
        "UPDATE sessions
            SET agent_session_key = ?2
          WHERE id = ?1
            AND agent_session_key IS NULL
            AND started_at = ?3",
        rusqlite::params![id, agent_session_key, expected_row_started_at],
    )
    .map(|updated| updated > 0)
}

/// Replace a live agent key after the runtime changes conversation identity.
/// The running-status and started-at guards reject stopped rows and stale
/// reports from a prior incarnation; unlike initial capture, an existing key
/// is expected and may be replaced.
pub fn rekey_agent_session_key(
    conn: &Connection,
    id: &str,
    agent_session_key: &str,
    expected_row_started_at: &str,
) -> rusqlite::Result<bool> {
    set_live_agent_session_key(conn, id, Some(agent_session_key), expected_row_started_at)
}

pub(crate) fn set_live_agent_session_key(
    conn: &Connection,
    id: &str,
    agent_session_key: Option<&str>,
    expected_row_started_at: &str,
) -> rusqlite::Result<bool> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let previous = get_row(&tx, id)?;
    let updated = tx.execute(
        "UPDATE sessions
            SET agent_session_key = ?2
          WHERE id = ?1
            AND started_at = ?3
            AND status = 'running'",
        rusqlite::params![id, agent_session_key, expected_row_started_at],
    )? > 0;
    if updated {
        if let (Some(row), Some(key)) = (previous, agent_session_key) {
            if row.agent_runtime.as_deref() == Some("claude-code")
                && row.agent_session_key.as_deref() != Some(key)
            {
                if let Some(mut definition) = row.launch_definition {
                    if let Some(account) = &mut definition.claude_account {
                        // A new conversation belongs to the running account; an existing
                        // conversation selected again can still live in the old folder.
                        let exists = crate::claude_accounts::transcript(
                            &account.origin_folder,
                            row.cwd.as_deref(),
                            key,
                        )
                        .is_some_and(|path| path.is_file());
                        if account.origin_folder != account.folder && !exists {
                            account.origin_folder.clone_from(&account.folder);
                            set_launch(&tx, id, "claude-code", &definition)?;
                        }
                    }
                }
            }
        }
    }
    tx.commit()?;
    Ok(updated)
}

/// Startup cleanup: demote rows still marked `running` from a prior app
/// process to `stopped`, preserving any prior `stopped_at`. Requeue a
/// launch-resume claim interrupted by an ungraceful exit. This must run
/// before the frontend consumes queued launch-resume work.
pub fn cleanup_stale_running(conn: &Connection, now: Timestamp) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE sessions
            SET resume_on_launch = ?1
          WHERE resume_on_launch = ?2",
        rusqlite::params![RESUME_ON_LAUNCH_PENDING, RESUME_ON_LAUNCH_CLAIMED],
    )?;
    let updated = tx.execute(
        "UPDATE sessions
            SET status = 'stopped',
                stopped_at = COALESCE(stopped_at, ?1)
            WHERE status = 'running'",
        rusqlite::params![now.to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(updated)
}

/// Add the sessions live at this graceful quit to the launch-resume set:
/// direct chats plus slots belonging to running missions. Any pending work
/// remains queued, and an in-flight claim is requeued before shutdown.
/// The caller kills the returned live ids only after this transaction commits.
pub fn mark_running_for_resume_on_launch(conn: &mut Connection) -> rusqlite::Result<Vec<String>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let ids = mark_running_for_resume(&tx)?;
    tx.commit()?;
    Ok(ids)
}

#[cfg(windows)]
pub(crate) fn mark_running_and_exited_for_resume_on_launch(
    conn: &mut Connection,
    exited: &[SessionRowDb],
) -> rusqlite::Result<Vec<String>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut ids = mark_running_for_resume(&tx)?;
    for row in exited {
        let changed = tx.execute(
            &format!(
                "UPDATE sessions
                    SET resume_on_launch = ?1,
                        agent_session_key = COALESCE(agent_session_key, ?2)
                  WHERE id = ?3 AND started_at IS ?4
                    AND {RESUMABLE_AT_QUIT_PREDICATE}"
            ),
            rusqlite::params![
                RESUME_ON_LAUNCH_PENDING,
                row.agent_session_key,
                row.id,
                row.started_at.map(|time| time.to_rfc3339()),
            ],
        )?;
        if changed > 0 && !ids.contains(&row.id) {
            ids.push(row.id.clone());
        }
    }
    tx.commit()?;
    Ok(ids)
}

fn mark_running_for_resume(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<Vec<String>> {
    tx.execute(
        "UPDATE sessions
            SET resume_on_launch = ?1
          WHERE resume_on_launch = ?2",
        rusqlite::params![RESUME_ON_LAUNCH_PENDING, RESUME_ON_LAUNCH_CLAIMED],
    )?;
    tx.execute(
        &format!(
            "UPDATE sessions
                SET resume_on_launch = ?1
              WHERE status = 'running' AND {RESUMABLE_AT_QUIT_PREDICATE}"
        ),
        [RESUME_ON_LAUNCH_PENDING],
    )?;
    let ids = {
        let mut stmt = tx.prepare(&format!(
            "SELECT id
               FROM sessions
              WHERE resume_on_launch = ?1
                AND status = 'running' AND {RESUMABLE_AT_QUIT_PREDICATE}
              ORDER BY started_at, id"
        ))?;
        stmt.query_map([RESUME_ON_LAUNCH_PENDING], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(ids)
}

/// The sessions with launch-resume work queued or in flight.
pub fn resume_on_launch_ids(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM sessions WHERE resume_on_launch != 0")?;
    stmt.query_map([], |row| row.get(0))?.collect()
}

/// Atomically claim the next quit-time stamp outside `excluded`. Excluded
/// claims stay pending so a surface can resume them when it becomes visible.
/// Shells need no prior conversation key because relaunching one always starts
/// a fresh process; chat rows that can no longer resume are cleared and skipped.
pub fn take_resume_on_launch_excluding(
    conn: &mut Connection,
    excluded: &HashSet<String>,
) -> rusqlite::Result<Option<ResumeOnLaunchClaim>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let next = loop {
        let row = {
            let mut stmt = tx.prepare(
                "SELECT id,
                        status IN ('stopped', 'crashed'),
                        agent_session_key IS NOT NULL
                            AND TRIM(agent_session_key) != '',
                        archived_at IS NULL,
                        COALESCE(
                            agent_runtime,
                            (SELECT runtime FROM roles WHERE roles.id = sessions.role_id),
                            ''
                        ) = 'shell'
                   FROM sessions
                  WHERE resume_on_launch = ?1
                  ORDER BY started_at, id",
            )?;
            let mut rows = stmt.query([RESUME_ON_LAUNCH_PENDING])?;
            let mut next = None;
            while let Some(row) = rows.next()? {
                let id = row.get::<_, String>(0)?;
                if excluded.contains(&id) {
                    continue;
                }
                next = Some((
                    id,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                ));
                break;
            }
            next
        };
        let Some((id, stopped, has_key, active, shell)) = row else {
            break None;
        };
        if stopped && (has_key || shell) && active {
            tx.execute(
                "UPDATE sessions
                    SET resume_on_launch = ?2
                  WHERE id = ?1
                    AND resume_on_launch = ?3",
                rusqlite::params![id, RESUME_ON_LAUNCH_CLAIMED, RESUME_ON_LAUNCH_PENDING],
            )?;
            break Some(ResumeOnLaunchClaim {
                session_id: id,
                shell,
            });
        }
        tx.execute(
            "UPDATE sessions SET resume_on_launch = 0 WHERE id = ?1",
            [&id],
        )?;
    };
    tx.commit()?;
    Ok(next)
}

/// Atomically claim one specific quit-time stamp. Invalid pending work is
/// cleared with the same rules as the startup claim consumer.
pub fn take_resume_on_launch_for_session(
    conn: &mut Connection,
    session_id: &str,
) -> rusqlite::Result<Option<ResumeOnLaunchClaim>> {
    let pending = conn
        .query_row(
            "SELECT 1
               FROM sessions
              WHERE id = ?1
                AND resume_on_launch = ?2",
            rusqlite::params![session_id, RESUME_ON_LAUNCH_PENDING],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !pending {
        return Ok(None);
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let row = tx
        .query_row(
            "SELECT id,
                    status IN ('stopped', 'crashed'),
                    agent_session_key IS NOT NULL
                        AND TRIM(agent_session_key) != '',
                    archived_at IS NULL,
                    COALESCE(
                        agent_runtime,
                        (SELECT runtime FROM roles WHERE roles.id = sessions.role_id),
                        ''
                    ) = 'shell'
               FROM sessions
              WHERE id = ?1
                AND resume_on_launch = ?2",
            rusqlite::params![session_id, RESUME_ON_LAUNCH_PENDING],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()?;
    let claim = match row {
        Some((id, stopped, has_key, active, shell)) if stopped && (has_key || shell) && active => {
            tx.execute(
                "UPDATE sessions
                    SET resume_on_launch = ?2
                  WHERE id = ?1
                    AND resume_on_launch = ?3",
                rusqlite::params![id, RESUME_ON_LAUNCH_CLAIMED, RESUME_ON_LAUNCH_PENDING],
            )?;
            Some(ResumeOnLaunchClaim {
                session_id: id,
                shell,
            })
        }
        Some((id, ..)) => {
            tx.execute(
                "UPDATE sessions SET resume_on_launch = 0 WHERE id = ?1",
                [&id],
            )?;
            None
        }
        None => None,
    };
    tx.commit()?;
    Ok(claim)
}

/// Clear a completed launch-resume claim. If quit already requeued the claim,
/// the guarded update is a no-op so the next launch still retries it.
pub fn finish_resume_on_launch(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.execute(
        "UPDATE sessions
            SET resume_on_launch = 0
          WHERE id = ?1
            AND resume_on_launch = ?2",
        rusqlite::params![id, RESUME_ON_LAUNCH_CLAIMED],
    )
    .map(|updated| updated > 0)
}

pub fn mark_resume_on_launch_claimed(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET resume_on_launch = ?2 WHERE id = ?1",
        rusqlite::params![id, RESUME_ON_LAUNCH_CLAIMED],
    )
}

/// Clear launch-resume work gated by the user's chat setting while leaving
/// shell rows queued so terminal panes always come back live.
pub fn clear_chat_resume_on_launch(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET resume_on_launch = 0
          WHERE resume_on_launch != 0
            AND COALESCE(
                    agent_runtime,
                    (SELECT runtime FROM roles WHERE roles.id = sessions.role_id),
                    ''
                ) != 'shell'",
        [],
    )
}

/// Soft-delete: refused for running rows (kill first).
pub fn archive(conn: &Connection, id: &str, archived_at: Timestamp) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET archived_at = ?2
          WHERE id = ?1
            AND status != 'running'",
        rusqlite::params![id, archived_at.to_rfc3339()],
    )
}

/// Clear a direct session's archive marker (Settings → Archived
/// restore). Scoped to direct chats like `list_archived_direct` —
/// mission/slot-bound rows must stay archived or they'd leak back
/// into `list_for_mission`. Single-column flip —
/// status, `agent_session_key`, and title all survive, so a restored
/// chat can still resume. The guards make a repeat call a 0-row no-op.
pub fn unarchive_direct(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET archived_at = NULL
          WHERE id = ?1
            AND mission_id IS NULL
            AND slot_id IS NULL
            AND archived_at IS NOT NULL",
        rusqlite::params![id],
    )
}

/// The Files view's saved root (#634); `None` for no row or no saved root.
pub fn files_root(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT files_root FROM sessions WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

pub fn set_files_root(conn: &Connection, id: &str, root: Option<&str>) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET files_root = ?2 WHERE id = ?1",
        rusqlite::params![id, root],
    )
}

pub fn set_title(conn: &Connection, id: &str, title: Option<&str>) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET title = ?2 WHERE id = ?1",
        rusqlite::params![id, title],
    )
}

pub fn set_live_title(
    conn: &Connection,
    id: &str,
    title: Option<&str>,
    expected_started_at: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET live_title = ?2
          WHERE id = ?1
            AND started_at IS ?3
            AND live_title IS NOT ?2
            AND COALESCE(agent_runtime, (SELECT runtime FROM roles WHERE id = role_id)) != 'shell'",
        rusqlite::params![id, title, expected_started_at],
    )
}

pub fn set_pinned_at(
    conn: &Connection,
    id: &str,
    pinned_at: Option<Timestamp>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET pinned_at = ?2 WHERE id = ?1",
        rusqlite::params![id, pinned_at.map(|t| t.to_rfc3339())],
    )
}

/// One mission session joined with its slot/role labels — feeds the
/// `ops::session::SessionRow` IPC DTO.
#[derive(Debug, Clone)]
pub struct MissionSessionRow {
    pub session: Session,
    pub live_title: Option<String>,
    pub agent_session_key: Option<String>,
    pub handle: String,
    pub runtime: String,
    pub lead: bool,
}

fn session_from_row_db(row: SessionRowDb) -> rusqlite::Result<Session> {
    // Mission-session surfaces INNER JOIN roles, so role_id is always
    // present; NULL would mean the query and this assembly drifted apart.
    let role_id = row.role_id.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Null,
            "mission session row missing role_id".into(),
        )
    })?;
    Ok(Session {
        id: row.id,
        mission_id: row.mission_id,
        role_id,
        slot_id: row.slot_id,
        cwd: row.cwd,
        status: row.status,
        pid: row.pid,
        started_at: row.started_at,
        stopped_at: row.stopped_at,
    })
}

/// Non-archived sessions of a mission in slot-roster order, joined with
/// the slot handle (template handle as fallback for pre-slot rows), the
/// role's runtime, and the lead flag. Assembled per decision 6: session
/// side via `from_row`, denormalized extras via plain `row.get`.
pub fn list_for_mission(
    conn: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<MissionSessionRow>> {
    let sql = format!(
        "SELECT {},
                COALESCE(sl.slot_handle, r.handle) AS handle,
                COALESCE(s.agent_runtime, '') AS role_runtime,
                COALESCE(sl.lead, 0) AS lead
           FROM sessions s
           JOIN roles r ON r.id = s.role_id
           LEFT JOIN slots sl ON sl.id = s.slot_id
          WHERE s.mission_id = ?1
            AND s.archived_at IS NULL
          ORDER BY COALESCE(sl.position, 0) ASC, s.started_at ASC",
        qualified_select_list("s", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![mission_id], |row| {
        let db_row = from_row::<SessionRowDb>(row).map_err(de_err)?;
        let handle: String = row.get("handle")?;
        let runtime: String = row.get("role_runtime")?;
        let lead: bool = row.get("lead")?;
        let agent_session_key = db_row.agent_session_key.clone();
        let live_title = db_row.live_title.clone();
        Ok(MissionSessionRow {
            session: session_from_row_db(db_row)?,
            live_title,
            agent_session_key,
            handle,
            runtime,
            lead,
        })
    })?;
    rows.collect()
}

pub fn roster_rows_for_mission(
    conn: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<SessionRowDb>> {
    // Resume replaces started_at; rowid retains the original registration order.
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM sessions WHERE mission_id = ?1 ORDER BY rowid",
        select_list(COLUMNS)
    ))?;
    let rows = stmt.query_map([mission_id], |row| {
        from_row::<SessionRowDb>(row).map_err(de_err)
    })?;
    rows.collect()
}

/// A direct-chat session row plus the role-template labels the sidebar
/// needs. `role_*` fields are None for runtime-only chats (#195).
/// Direct chats are exactly rows with no mission and no slot; legacy
/// orphaned mission-slot rows can have `mission_id NULL` after the old
/// FK behavior, but their `slot_id` must keep them off this surface.
#[derive(Debug, Clone)]
pub struct DirectSessionRow {
    pub row: SessionRowDb,
    pub role_handle: Option<String>,
    pub role_display_name: Option<String>,
}

const DIRECT_EXTRAS: &str = "r.handle    AS role_handle,
                r.display_name AS role_display_name";

fn direct_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DirectSessionRow> {
    Ok(DirectSessionRow {
        row: from_row::<SessionRowDb>(row).map_err(de_err)?,
        role_handle: row.get("role_handle")?,
        role_display_name: row.get("role_display_name")?,
    })
}

/// Flat list of every un-archived direct session. Sort key:
///   1. pinned first (pinned_at NOT NULL)
///   2. then running before stopped/crashed
///   3. then by most-recent activity (stopped_at if set, else started_at)
pub fn list_recent_direct(conn: &Connection) -> rusqlite::Result<Vec<DirectSessionRow>> {
    let sql = format!(
        "SELECT {}, {DIRECT_EXTRAS}
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.mission_id IS NULL
            AND s.slot_id IS NULL
            AND s.archived_at IS NULL
          ORDER BY CASE WHEN s.pinned_at IS NOT NULL THEN 0 ELSE 1 END,
                   CASE WHEN s.status = 'running'    THEN 0 ELSE 1 END,
                   COALESCE(s.stopped_at, s.started_at) DESC",
        qualified_select_list("s", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], direct_row)?;
    rows.collect()
}

/// Archived direct sessions, newest-archived first — the Settings →
/// Archived pane's chat list. Same row shape and direct-chat scoping as
/// `list_recent_direct`; archived mission-slot rows stay off this
/// surface.
pub fn list_archived_direct(conn: &Connection) -> rusqlite::Result<Vec<DirectSessionRow>> {
    let sql = format!(
        "SELECT {}, {DIRECT_EXTRAS}
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.mission_id IS NULL
            AND s.slot_id IS NULL
            AND s.archived_at IS NOT NULL
          ORDER BY s.archived_at DESC",
        qualified_select_list("s", COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], direct_row)?;
    rows.collect()
}

/// Unfiltered single-row lookup for a direct-chat session — archived rows
/// ARE returned (the chat page renders them read-only; see the
/// archived-row tests in `ops::session`). Mission sessions are not:
/// this surface is for direct chats only.
pub fn get_direct(conn: &Connection, id: &str) -> rusqlite::Result<Option<DirectSessionRow>> {
    let sql = format!(
        "SELECT {}, {DIRECT_EXTRAS}
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.id = ?1
            AND s.mission_id IS NULL
            AND s.slot_id IS NULL",
        qualified_select_list("s", COLUMNS)
    );
    conn.query_row(&sql, rusqlite::params![id], direct_row)
        .optional()
}

/// The title the sidebar shows for a direct chat when the app has no live
/// title of its own: the person's name for it, the agent's persisted
/// topic, else the role or runtime label.
pub fn direct_title(chat: &DirectSessionRow) -> String {
    let runtime = chat.row.agent_runtime.as_deref().unwrap_or_default();
    let handle = chat.role_handle.as_deref();
    runner_core::protocol::session_title::preferred_title(
        chat.row.title.as_deref(),
        None,
        chat.row.live_title.as_deref(),
        runtime,
        handle,
        chat.row.cwd.as_deref(),
    )
    .unwrap_or_else(|| {
        let command = chat
            .row
            .launch_definition
            .as_ref()
            .map(|definition| definition.command.as_str())
            .or(chat.row.agent_command.as_deref())
            .unwrap_or_default();
        let display_name = chat
            .role_display_name
            .as_deref()
            .filter(|_| handle.is_some())
            .or_else(|| {
                crate::runtimes::for_key(runtime)
                    .catalog()
                    .map(|catalog| catalog.display_name)
            })
            .unwrap_or(runtime);
        runner_core::protocol::session_title::default_label(runtime, command, handle, display_name)
    })
}

pub fn live_handles_for_mission(
    conn: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(sl.slot_handle, r.handle) AS handle
           FROM sessions s
           JOIN roles r ON r.id = s.role_id
           LEFT JOIN slots sl ON sl.id = s.slot_id
          WHERE s.mission_id = ?1
            AND s.status = 'running'",
    )?;
    let rows = stmt.query_map(rusqlite::params![mission_id], |row| row.get(0))?;
    rows.collect()
}

pub fn capture_cwd(
    conn: &Connection,
    session_id: &str,
    expected_started_at: &str,
    runtime: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT s.cwd
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.id = ?1
            AND s.started_at = ?2
            AND s.status = 'running'
            AND s.agent_session_key IS NULL
            AND s.agent_runtime = ?3",
        rusqlite::params![session_id, expected_started_at, runtime],
        |row| row.get(0),
    )
}

pub fn count_capture_siblings(
    conn: &Connection,
    session_id: &str,
    spawn_cwd: &str,
    runtime: &str,
) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.status = 'running'
            AND s.agent_session_key IS NULL
            AND s.agent_runtime = ?3
            AND s.id <> ?1
            AND (s.cwd = ?2 OR s.cwd IS NULL)",
        rusqlite::params![session_id, spawn_cwd, runtime],
        |row| row.get(0),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanProcessRow {
    pub session_id: String,
    pub pid: i64,
    pub runtime: Option<String>,
    pub command: Option<String>,
}

pub fn list_orphan_process_candidates(
    conn: &Connection,
) -> rusqlite::Result<Vec<OrphanProcessRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.id,
                s.pid,
                COALESCE(s.agent_runtime, r.runtime),
                COALESCE(s.agent_command, r.command)
           FROM sessions s
           LEFT JOIN roles r ON r.id = s.role_id
          WHERE s.status != 'running'
            AND s.pid IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(OrphanProcessRow {
            session_id: row.get(0)?,
            pid: row.get(1)?,
            runtime: row.get(2)?,
            command: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Clear the recorded pid of every session that is not running.
pub fn clear_stale_pids(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions SET pid = NULL WHERE status != 'running' AND pid IS NOT NULL",
        [],
    )
}

/// Clear a session's recorded pid, only while it is still `expected_pid` and
/// the session is not running.
pub fn clear_recorded_pid(
    conn: &Connection,
    session_id: &str,
    expected_pid: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE sessions
            SET pid = NULL
          WHERE id = ?1
            AND pid = ?2
            AND status != 'running'",
        rusqlite::params![session_id, expected_pid],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::Utc;

    fn seed_role(conn: &Connection, id: &str, handle: &str) {
        conn.execute(
            "INSERT INTO roles (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'alpha', 'alpha', ?4, ?4)",
            rusqlite::params![
                id,
                handle,
                format!("{handle} display"),
                "2026-04-22T00:00:00Z"
            ],
        )
        .unwrap();
    }

    fn take_resume_id(conn: &mut Connection) -> Option<String> {
        take_resume_on_launch_excluding(conn, &HashSet::new())
            .unwrap()
            .map(|claim| claim.session_id)
    }

    fn full_row() -> SessionRowDb {
        let now = Utc::now();
        SessionRowDb {
            id: "sess-full".into(),
            mission_id: None,
            project_id: None,
            role_id: Some("r1".into()),
            slot_id: None,
            cwd: Some("/tmp/work".into()),
            status: SessionStatus::Stopped,
            pid: Some(4242),
            started_at: Some(now),
            stopped_at: Some(now),
            agent_session_key: Some("2f6e0f2e-key".into()),
            archived_at: Some(now),
            title: Some("my chat".into()),
            live_title: Some("Fixing the parser".into()),
            pinned_at: Some(now),
            agent_runtime: Some("codex".into()),
            agent_command: Some("codex".into()),
            launch_definition: Some(LaunchDefinition {
                claude_account: None,
                command: "codex".into(),
                args: vec!["--search".into()],
                env: BTreeMap::from([("MARKER".into(), "before".into())]),
                model: Some("gpt-5.6-sol".into()),
                effort: Some("max".into()),
                codex_speed: Some(CodexSpeed::Fast),
                system_prompt: Some("Persona".into()),
            }),
            last_cols: Some(132),
            last_rows: Some(41),
            resume_on_launch: true,
        }
    }

    fn minimal_row() -> SessionRowDb {
        SessionRowDb::new_running("sess-min".into())
    }

    #[test]
    fn live_titles_survive_reopen_and_resume_but_not_a_fresh_conversation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runner.db");
        let started = Utc::now();
        let stamp = started.to_rfc3339();
        {
            let pool = db::open_pool(&path).unwrap();
            let conn = pool.get().unwrap();
            let mut row = minimal_row();
            row.agent_runtime = Some("codex".into());
            row.agent_command = Some("codex".into());
            row.started_at = Some(started);
            row.title = Some("Manual name".into());
            row.agent_session_key = Some("conversation".into());
            insert(&conn, &row).unwrap();
            assert_eq!(
                set_live_title(&conn, &row.id, Some("Cars"), Some(&stamp)).unwrap(),
                1
            );
            assert_eq!(
                set_live_title(&conn, &row.id, Some("Cars"), Some(&stamp)).unwrap(),
                0
            );
            set_exit_status(&conn, &row.id, SessionStatus::Stopped, Utc::now()).unwrap();
        }
        let pool = db::open_pool(&path).unwrap();
        let conn = pool.get().unwrap();
        let row = get_row(&conn, "sess-min").unwrap().unwrap();
        assert_eq!(row.live_title.as_deref(), Some("Cars"));
        assert_eq!(row.title.as_deref(), Some("Manual name"));
        let resumed = started + chrono::Duration::seconds(1);
        resume_in_place(&conn, &row.id, resumed, None, false, (80, 24), None).unwrap();
        assert_eq!(
            get_row(&conn, &row.id)
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Cars")
        );
        let fresh = resumed + chrono::Duration::seconds(1);
        resume_in_place(&conn, &row.id, fresh, None, true, (80, 24), None).unwrap();
        assert_eq!(
            set_live_title(&conn, &row.id, Some("Stale title"), Some(&stamp)).unwrap(),
            0
        );
        let row = get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(row.live_title, None);
        assert_eq!(row.title.as_deref(), Some("Manual name"));
        assert_eq!(row.agent_session_key, None);
        let stamp = fresh.to_rfc3339();
        assert_eq!(
            set_live_title(&conn, &row.id, Some("New topic"), Some(&stamp)).unwrap(),
            1
        );
        assert_eq!(
            set_live_title(&conn, &row.id, None, Some(&stamp)).unwrap(),
            1
        );
        assert_eq!(
            set_live_title(&conn, &row.id, None, Some(&stamp)).unwrap(),
            0
        );
        assert_eq!(get_row(&conn, &row.id).unwrap().unwrap().live_title, None);
    }

    #[test]
    fn shell_titles_are_never_persisted_for_direct_or_role_sessions() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_role(&conn, "shell-role", "shell");
        conn.execute(
            "UPDATE roles SET runtime = 'shell' WHERE id = 'shell-role'",
            [],
        )
        .unwrap();
        for direct in [true, false] {
            let mut row = SessionRowDb::new_running(format!("shell-{direct}"));
            if direct {
                row.agent_runtime = Some("shell".into());
            } else {
                row.role_id = Some("shell-role".into());
            }
            insert(&conn, &row).unwrap();
            assert_eq!(
                set_live_title(&conn, &row.id, Some("Shell prompt"), None).unwrap(),
                0
            );
            assert_eq!(get_row(&conn, &row.id).unwrap().unwrap().live_title, None);
        }
    }

    #[test]
    fn insert_then_get_round_trips_full_and_minimal_rows() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        for row in [full_row(), minimal_row()] {
            insert(&conn, &row).unwrap();
            assert_eq!(get_row(&conn, &row.id).unwrap().unwrap(), row);
        }
        let stored: String = conn
            .query_row(
                "SELECT launch_definition_json FROM sessions WHERE id = 'sess-full'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(
            stored["runtime_options"],
            serde_json::json!({"codex":{"speed":"fast"}})
        );
        assert!(stored.get("runtime").is_none());
    }

    #[test]
    fn legacy_rows_in_todays_stored_formats_read_cleanly() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        // Both timestamp spellings, a captured agent key, and legacy tmux
        // runtime metadata — the shapes real upgraded databases carry.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, role_id, cwd, status, pid, started_at, stopped_at,
                 agent_session_key)
             VALUES ('sess-legacy', NULL, 'r1', '/tmp', 'crashed', 123,
                     '2026-04-22T01:02:03.456789+00:00', '2026-04-22T02:00:00Z',
                     'abcd-uuid')",
            [],
        )
        .unwrap();
        let row = get_row(&conn, "sess-legacy").unwrap().unwrap();
        assert_eq!(row.status, SessionStatus::Crashed);
        assert_eq!(row.pid, Some(123));
        assert_eq!(row.agent_session_key.as_deref(), Some("abcd-uuid"));
        assert_eq!(row.launch_definition, None);
        assert_eq!(
            row.started_at.unwrap(),
            "2026-04-22T01:02:03.456789Z".parse::<Timestamp>().unwrap()
        );
        assert_eq!(
            row.stopped_at.unwrap(),
            "2026-04-22T02:00:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn writes_are_byte_identical_to_the_legacy_path() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        let row = full_row();
        insert(&conn, &row).unwrap();
        let (started_raw, status_raw): (String, String) = conn
            .query_row(
                "SELECT started_at, status FROM sessions WHERE id = 'sess-full'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(started_raw, row.started_at.unwrap().to_rfc3339());
        assert_eq!(status_raw, "stopped");
    }

    #[test]
    fn capture_agent_session_key_is_guarded_by_null_key_and_started_at() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        let mut row = minimal_row();
        let started = Utc::now();
        row.role_id = Some("r1".into());
        row.started_at = Some(started);
        insert(&conn, &row).unwrap();
        let started_str = started.to_rfc3339();

        // Stale watcher (wrong started_at) must not write.
        assert!(!capture_agent_session_key(
            &conn,
            "sess-min",
            "key-a",
            "1999-01-01T00:00:00+00:00"
        )
        .unwrap());
        // Matching guard writes.
        assert!(capture_agent_session_key(&conn, "sess-min", "key-a", &started_str).unwrap());
        // Second writer must not clobber.
        assert!(!capture_agent_session_key(&conn, "sess-min", "key-b", &started_str).unwrap());
        let stored: String = conn
            .query_row(
                "SELECT agent_session_key FROM sessions WHERE id = 'sess-min'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "key-a");
    }

    #[test]
    fn rekey_agent_session_key_requires_current_running_incarnation() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let mut row = minimal_row();
        let started = Utc::now();
        row.started_at = Some(started);
        row.agent_session_key = Some("key-a".into());
        insert(&conn, &row).unwrap();
        let started_str = started.to_rfc3339();

        assert!(
            !rekey_agent_session_key(&conn, "sess-min", "key-b", "1999-01-01T00:00:00+00:00")
                .unwrap()
        );
        assert!(rekey_agent_session_key(&conn, "sess-min", "key-b", &started_str).unwrap());

        set_exit_status(&conn, "sess-min", SessionStatus::Stopped, Utc::now()).unwrap();
        assert!(!rekey_agent_session_key(&conn, "sess-min", "key-c", &started_str).unwrap());
        assert_eq!(
            get_row(&conn, "sess-min")
                .unwrap()
                .unwrap()
                .agent_session_key
                .as_deref(),
            Some("key-b")
        );
    }

    #[test]
    fn graceful_quit_preserves_pending_and_marks_only_live_direct_and_running_mission_slots() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        let now = "2026-07-25T00:00:00Z";
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission-running', 'c1', 'Running', 'running', ?1),
                    ('mission-stopped', 'c1', 'Stopped', 'stopped', ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, role_id, slot_id, status, started_at,
                 archived_at, resume_on_launch)
             VALUES
                ('direct-running', NULL, 'r1', NULL, 'running', ?1, NULL, 0),
                ('direct-stopped', NULL, 'r1', NULL, 'stopped', ?1, NULL, 1),
                ('mission-running-slot', 'mission-running', 'r1', 'slot-a',
                 'running', ?1, NULL, 0),
                ('mission-stopped-slot', 'mission-stopped', 'r1', 'slot-b',
                 'running', ?1, NULL, 0),
                ('mission-no-slot', 'mission-running', 'r1', NULL,
                 'running', ?1, NULL, 0),
                ('mission-archived-slot', 'mission-running', 'r1', 'slot-c',
                 'running', ?1, ?1, 0)",
            [now],
        )
        .unwrap();
        let pending_before: i64 = conn
            .query_row(
                "SELECT resume_on_launch FROM sessions WHERE id = 'direct-stopped'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending_before, RESUME_ON_LAUNCH_PENDING);

        let ids = mark_running_for_resume_on_launch(&mut conn).unwrap();
        assert_eq!(ids, ["direct-running", "mission-running-slot"]);

        let marked: Vec<String> = conn
            .prepare(
                "SELECT id FROM sessions
                  WHERE resume_on_launch = 1
                  ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            marked,
            ["direct-running", "direct-stopped", "mission-running-slot"]
        );
    }

    #[test]
    fn launch_consumer_skips_unresumable_rows_and_clears_every_stamp() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        let now = "2026-07-25T00:00:00Z";
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key,
                 archived_at, resume_on_launch)
             VALUES
                ('a-missing-key', 'r1', 'stopped', ?1, NULL, NULL, 1),
                ('b-archived', 'r1', 'stopped', ?1, 'key-b', ?1, 1),
                ('c-running', 'r1', 'running', ?1, 'key-c', NULL, 1),
                ('d-resumable', 'r1', 'stopped', ?1, 'key-d', NULL, 1)",
            [now],
        )
        .unwrap();

        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("d-resumable"));
        let claim: i64 = conn
            .query_row(
                "SELECT resume_on_launch FROM sessions WHERE id = 'd-resumable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(claim, RESUME_ON_LAUNCH_CLAIMED);
        assert!(finish_resume_on_launch(&conn, "d-resumable").unwrap());
        assert_eq!(take_resume_id(&mut conn), None);
        let marked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE resume_on_launch != 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marked, 0);
    }

    #[test]
    fn launch_consumer_leaves_excluded_claims_pending_and_targets_them_later() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let now = "2026-07-25T00:00:00Z";
        conn.execute(
            "INSERT INTO sessions
                (id, status, started_at, agent_runtime, agent_command, resume_on_launch)
             VALUES
                ('drawer-shell', 'stopped', ?1, 'shell', '/bin/zsh', 1),
                ('pane-shell', 'stopped', ?1, 'shell', '/bin/zsh', 1)",
            [now],
        )
        .unwrap();
        let excluded = HashSet::from(["drawer-shell".to_owned()]);

        assert_eq!(
            take_resume_on_launch_excluding(&mut conn, &excluded).unwrap(),
            Some(ResumeOnLaunchClaim {
                session_id: "pane-shell".into(),
                shell: true,
            })
        );
        assert!(finish_resume_on_launch(&conn, "pane-shell").unwrap());
        assert_eq!(
            take_resume_on_launch_excluding(&mut conn, &excluded).unwrap(),
            None
        );
        let drawer_stamp: i64 = conn
            .query_row(
                "SELECT resume_on_launch FROM sessions WHERE id = 'drawer-shell'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(drawer_stamp, RESUME_ON_LAUNCH_PENDING);

        assert_eq!(
            take_resume_on_launch_for_session(&mut conn, "drawer-shell").unwrap(),
            Some(ResumeOnLaunchClaim {
                session_id: "drawer-shell".into(),
                shell: true,
            })
        );
        assert_eq!(
            take_resume_on_launch_for_session(&mut conn, "drawer-shell").unwrap(),
            None
        );
        assert!(finish_resume_on_launch(&conn, "drawer-shell").unwrap());
    }

    #[test]
    fn targeted_launch_claim_does_not_disturb_pop_any_order() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let now = "2026-07-25T00:00:00Z";
        conn.execute(
            "INSERT INTO sessions
                (id, status, started_at, agent_runtime, agent_command, resume_on_launch)
             VALUES
                ('first', 'stopped', ?1, 'shell', '/bin/zsh', 1),
                ('target', 'stopped', ?1, 'shell', '/bin/zsh', 1)",
            [now],
        )
        .unwrap();

        assert_eq!(
            take_resume_on_launch_for_session(&mut conn, "target")
                .unwrap()
                .map(|claim| claim.session_id)
                .as_deref(),
            Some("target")
        );
        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("first"));
    }

    #[test]
    fn disabled_chat_resume_keeps_keyless_shell_claim_available() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        let now = "2026-07-25T00:00:00Z";
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key,
                 agent_runtime, agent_command, resume_on_launch)
             VALUES
                ('chat', 'r1', 'stopped', ?1, 'key', 'alpha', NULL, 1),
                ('shell', NULL, 'stopped', ?1, NULL, 'shell', '/bin/zsh', 1)",
            [now],
        )
        .unwrap();

        assert_eq!(clear_chat_resume_on_launch(&conn).unwrap(), 1);
        assert_eq!(
            take_resume_on_launch_excluding(&mut conn, &HashSet::new()).unwrap(),
            Some(ResumeOnLaunchClaim {
                session_id: "shell".into(),
                shell: true,
            })
        );
        assert!(finish_resume_on_launch(&conn, "shell").unwrap());
        assert_eq!(take_resume_id(&mut conn), None);
    }

    #[test]
    fn quitting_while_launch_resume_is_starting_requeues_the_claim() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key, resume_on_launch)
             VALUES
                ('starting', 'r1', 'stopped', '2026-07-25T00:00:00Z', 'key', 1)",
            [],
        )
        .unwrap();

        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("starting"));
        assert!(mark_running_for_resume_on_launch(&mut conn)
            .unwrap()
            .is_empty());
        assert!(!finish_resume_on_launch(&conn, "starting").unwrap());
        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("starting"));
    }

    #[test]
    fn resumed_session_is_marked_again_on_the_next_quit_without_input() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key, resume_on_launch)
             VALUES
                ('resumed', 'r1', 'stopped', '2026-07-25T00:00:00Z', 'key', 1)",
            [],
        )
        .unwrap();

        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("resumed"));
        conn.execute(
            "UPDATE sessions SET status = 'running' WHERE id = 'resumed'",
            [],
        )
        .unwrap();
        assert!(finish_resume_on_launch(&conn, "resumed").unwrap());
        assert_eq!(
            mark_running_for_resume_on_launch(&mut conn).unwrap(),
            ["resumed"]
        );
        assert_eq!(cleanup_stale_running(&conn, Utc::now()).unwrap(), 1);
        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("resumed"));
    }

    #[test]
    fn startup_requeues_a_launch_resume_claim_interrupted_by_a_crash() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key, resume_on_launch)
             VALUES
                ('claimed', 'r1', 'stopped', '2026-07-25T00:00:00Z', 'key', 2)",
            [],
        )
        .unwrap();

        assert_eq!(cleanup_stale_running(&conn, Utc::now()).unwrap(), 0);
        assert_eq!(take_resume_id(&mut conn).as_deref(), Some("claimed"));
    }

    #[test]
    fn list_for_mission_matches_todays_dto_shape() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        seed_role(&conn, "r1", "template");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at) VALUES ('c1', 'C', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots (id, crew_id, role_id, slot_handle, position, lead, added_at)
             VALUES ('sl1', 'c1', 'r1', 'coder', 0, 1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m1', 'c1', 't', 'running', ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, role_id, slot_id, status, started_at, agent_session_key, agent_runtime)
             VALUES ('se1', 'm1', 'r1', 'sl1', 'running', ?1, 'key-1', 'alpha')",
            rusqlite::params![now],
        )
        .unwrap();
        // Archived row must be filtered out.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, role_id, slot_id, status, started_at, archived_at)
             VALUES ('se-archived', 'm1', 'r1', 'sl1', 'stopped', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();

        let rows = list_for_mission(&conn, "m1").unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.session.id, "se1");
        assert_eq!(row.session.role_id, "r1");
        assert_eq!(row.session.status, SessionStatus::Running);
        assert_eq!(row.handle, "coder", "slot handle wins over template handle");
        assert_eq!(row.runtime, "alpha");
        assert!(row.lead);
        assert_eq!(row.agent_session_key.as_deref(), Some("key-1"));
    }

    #[test]
    fn list_archived_direct_mirrors_the_active_list_newest_first() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now();
        seed_role(&conn, "r1", "alpha");
        // Active direct chat — stays off the archived list.
        conn.execute(
            "INSERT INTO sessions (id, role_id, status, started_at)
             VALUES ('d-active', 'r1', 'stopped', ?1)",
            rusqlite::params![now.to_rfc3339()],
        )
        .unwrap();
        // Two archived chats, inserted out of archive order.
        conn.execute(
            "INSERT INTO sessions (id, role_id, status, started_at, archived_at)
             VALUES ('d-arch-old', 'r1', 'stopped', ?1, ?2),
                    ('d-arch-new', 'r1', 'stopped', ?1, ?1)",
            rusqlite::params![
                now.to_rfc3339(),
                (now - chrono::Duration::hours(2)).to_rfc3339()
            ],
        )
        .unwrap();
        // Archived mission-slot row — slot-bound, so it must never
        // surface as an archived chat.
        conn.execute(
            "INSERT INTO sessions (id, role_id, slot_id, status, started_at, archived_at)
             VALUES ('slot-arch', 'r1', 'sl-old', 'stopped', ?1, ?1)",
            rusqlite::params![now.to_rfc3339()],
        )
        .unwrap();

        let ids: Vec<String> = list_archived_direct(&conn)
            .unwrap()
            .into_iter()
            .map(|d| d.row.id)
            .collect();
        assert_eq!(ids, vec!["d-arch-new", "d-arch-old"]);
    }

    #[test]
    fn unarchive_returns_the_row_to_the_active_list() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions
                (id, role_id, status, started_at, agent_session_key, archived_at)
             VALUES ('d1', 'r1', 'stopped', ?1, 'resume-key', ?1)",
            rusqlite::params![now.to_rfc3339()],
        )
        .unwrap();

        assert_eq!(unarchive_direct(&conn, "d1").unwrap(), 1);
        let row = get_row(&conn, "d1").unwrap().unwrap();
        assert_eq!(row.archived_at, None);
        assert_eq!(
            row.agent_session_key.as_deref(),
            Some("resume-key"),
            "resume key survives unarchive"
        );
        assert_eq!(row.status, SessionStatus::Stopped, "status is preserved");
        let listed = list_recent_direct(&conn).unwrap();
        assert!(listed.iter().any(|d| d.row.id == "d1"));
        assert!(list_archived_direct(&conn).unwrap().is_empty());

        // Idempotent: an already-active row is a 0-row no-op, not an error.
        assert_eq!(unarchive_direct(&conn, "d1").unwrap(), 0);
    }

    #[test]
    fn unarchive_direct_refuses_mission_and_slot_bound_rows() {
        // The Archived pane only lists direct chats; restoring a
        // mission-bound row would leak it back into `list_for_mission`.
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at) VALUES ('c1', 'C', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m1', 'c1', 't', 'completed', ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        // Archived mission session and a slot-bound legacy orphan
        // (mission_id NULL but still slot-scoped).
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at, archived_at)
             VALUES ('se-mission', 'm1', 'r1', NULL, 'stopped', ?1, ?1),
                    ('se-slot', NULL, 'r1', 'sl-old', 'stopped', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();

        for id in ["se-mission", "se-slot"] {
            assert_eq!(unarchive_direct(&conn, id).unwrap(), 0);
            let row = get_row(&conn, id).unwrap().unwrap();
            assert!(
                row.archived_at.is_some(),
                "{id}: mission/slot-bound rows must stay archived"
            );
        }
    }

    #[test]
    fn direct_session_surfaces_join_role_labels_and_keep_archived_semantics() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        seed_role(&conn, "r1", "alpha");
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, status, started_at, agent_session_key)
             VALUES ('d1', NULL, 'r1', 'stopped', ?1, 'resume-key')",
            rusqlite::params![now],
        )
        .unwrap();
        // Runtime-only chat (#195): no role row behind it.
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, role_id, status, started_at, agent_runtime, agent_command)
             VALUES ('d2', NULL, NULL, 'running', ?1, 'codex', 'codex')",
            rusqlite::params![now],
        )
        .unwrap();
        // Archived: hidden from the list, still returned by get_direct.
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, status, started_at, archived_at)
             VALUES ('d3', NULL, 'r1', 'stopped', ?1, ?1)",
            rusqlite::params![now],
        )
        .unwrap();
        // Legacy orphan from the old mission FK: no mission, but still
        // slot-bound, so it must never masquerade as a direct chat.
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
             VALUES ('orphan-slot', NULL, 'r1', 'slot-old', 'stopped', ?1)",
            rusqlite::params![now],
        )
        .unwrap();

        let listed = list_recent_direct(&conn).unwrap();
        let ids: Vec<&str> = listed.iter().map(|d| d.row.id.as_str()).collect();
        assert_eq!(ids, vec!["d2", "d1"], "running first, archived hidden");
        let d1 = listed.iter().find(|d| d.row.id == "d1").unwrap();
        assert_eq!(d1.role_handle.as_deref(), Some("alpha"));
        let d2 = listed.iter().find(|d| d.row.id == "d2").unwrap();
        assert_eq!(d2.role_handle, None);
        assert_eq!(d2.row.agent_runtime.as_deref(), Some("codex"));

        let archived = get_direct(&conn, "d3").unwrap().unwrap();
        assert!(archived.row.archived_at.is_some());
        assert!(
            get_direct(&conn, "orphan-slot").unwrap().is_none(),
            "slot-bound orphaned mission sessions must stay off direct-chat surfaces"
        );
    }
}

#[cfg(test)]
mod launch_definition_tests {
    use super::*;

    #[test]
    fn claude_rekey_preserves_existing_origins_and_updates_key_and_origin_atomically() {
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let mut row = SessionRowDb::new_running("claude-rekey".into());
        row.started_at = Some(chrono::Utc::now());
        row.cwd = Some(temp.path().to_string_lossy().into_owned());
        row.agent_runtime = Some("claude-code".into());
        let old_key = uuid::Uuid::new_v4().to_string();
        let new_key = uuid::Uuid::new_v4().to_string();
        row.agent_session_key = Some(old_key.clone());
        let native = temp.path().join("native").to_string_lossy().into_owned();
        let work = temp.path().join("work").to_string_lossy().into_owned();
        row.launch_definition = Some(LaunchDefinition {
            command: "claude".into(),
            args: vec![],
            env: BTreeMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
            claude_account: Some(runner_core::protocol::ClaudeSessionAccount {
                account_id: Some("work".into()),
                folder: work.clone(),
                native: false,
                pinned: false,
                origin_folder: native.clone(),
                permission_mode: "default".into(),
            }),
        });
        insert(&conn, &row).unwrap();
        let generation = row.started_at.unwrap().to_rfc3339();
        let read = || get_row(&conn, &row.id).unwrap().unwrap();
        let origin = || {
            read()
                .launch_definition
                .unwrap()
                .claude_account
                .unwrap()
                .origin_folder
        };

        assert!(!set_live_agent_session_key(&conn, &row.id, Some(&new_key), "stale").unwrap());
        assert_eq!(read(), row);
        assert!(set_live_agent_session_key(&conn, &row.id, Some(&old_key), &generation).unwrap());
        assert_eq!(read(), row);

        let existing_key = uuid::Uuid::new_v4().to_string();
        let existing =
            crate::claude_accounts::transcript(&native, row.cwd.as_deref(), &existing_key).unwrap();
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(existing, "existing conversation").unwrap();
        assert!(
            set_live_agent_session_key(&conn, &row.id, Some(&existing_key), &generation).unwrap()
        );
        assert_eq!(origin(), native);

        conn.execute_batch("CREATE TRIGGER reject_rekey_origin BEFORE UPDATE OF launch_definition_json ON sessions BEGIN SELECT RAISE(FAIL, 'origin write rejected'); END;").unwrap();
        assert!(set_live_agent_session_key(&conn, &row.id, Some(&new_key), &generation).is_err());
        assert_eq!(
            read().agent_session_key.as_deref(),
            Some(existing_key.as_str())
        );
        assert_eq!(origin(), native);
        conn.execute_batch("DROP TRIGGER reject_rekey_origin;")
            .unwrap();

        set_exit_status(&conn, &row.id, SessionStatus::Stopped, chrono::Utc::now()).unwrap();
        assert!(!set_live_agent_session_key(&conn, &row.id, Some(&new_key), &generation).unwrap());
        assert_eq!(
            read().agent_session_key.as_deref(),
            Some(existing_key.as_str())
        );
        assert_eq!(origin(), native);
        conn.execute(
            "UPDATE sessions SET status = 'running' WHERE id = ?1",
            [&row.id],
        )
        .unwrap();
        assert!(set_live_agent_session_key(&conn, &row.id, Some(&new_key), &generation).unwrap());
        assert_eq!(read().agent_session_key.as_deref(), Some(new_key.as_str()));
        assert_eq!(origin(), work);
    }

    #[test]
    fn set_launch_round_trips_defaults_and_rejects_unknown_version() {
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = SessionRowDb::new_running("definition".into());
        insert(&conn, &row).unwrap();
        let definition = LaunchDefinition {
            claude_account: None,
            command: "codex".into(),
            args: vec![],
            env: BTreeMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
        };
        set_launch(&conn, &row.id, "codex", &definition).unwrap();
        let stored = get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(stored.agent_runtime.as_deref(), Some("codex"));
        assert_eq!(stored.launch_definition.as_ref(), Some(&definition));
        let json: String = conn
            .query_row(
                "SELECT launch_definition_json FROM sessions WHERE id = 'definition'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["runtime_options"], serde_json::json!({}));
        assert_eq!(json["env"], serde_json::json!({}));
        assert_eq!(json["model"], serde_json::Value::Null);
        assert!(json.get("runtime").is_none());
        json["version"] = serde_json::json!(99);
        conn.execute(
            "UPDATE sessions SET launch_definition_json = ?1 WHERE id = 'definition'",
            [json.to_string()],
        )
        .unwrap();
        let error = get_row(&conn, &row.id).unwrap_err().to_string();
        assert!(
            error.contains("unsupported launch definition version: 99"),
            "{error}"
        );
    }

    #[test]
    fn resume_rolls_back_key_and_generation_when_launch_write_fails() {
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let mut row = SessionRowDb::new_running("restart-write-failure".into());
        row.status = SessionStatus::Stopped;
        row.agent_session_key = Some("codex-key".into());
        row.agent_runtime = Some("codex".into());
        row.launch_definition = Some(crate::test_support::session_definition("codex", "codex"));
        insert(&conn, &row).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_launch_write BEFORE UPDATE OF launch_definition_json ON sessions
             BEGIN SELECT RAISE(ABORT, 'injected launch write failure'); END;",
        )
        .unwrap();
        let definition = crate::test_support::session_definition("claude-code", "claude");
        let error = resume_in_place(
            &conn,
            &row.id,
            chrono::Utc::now(),
            Some("claude-key"),
            true,
            (120, 40),
            Some(("claude-code", &definition)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected launch write failure"));
        let stored = get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(stored.status, row.status);
        assert_eq!(stored.started_at, row.started_at);
        assert_eq!(stored.agent_session_key, row.agent_session_key);
        assert_eq!(stored.agent_runtime, row.agent_runtime);
        assert_eq!(stored.launch_definition, row.launch_definition);
        assert_eq!(stored.last_cols, row.last_cols);
        assert_eq!(stored.last_rows, row.last_rows);
    }

    #[test]
    fn record_process_replaces_pid_and_executable_together() {
        let pool = crate::db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert(&conn, &SessionRowDb::new_running("process".into())).unwrap();
        for (pid, executable) in [(100, "/old/codex"), (200, "/new/codex")] {
            record_process(&conn, "process", Some(pid), executable).unwrap();
            let row = get_row(&conn, "process").unwrap().unwrap();
            assert_eq!(row.pid, Some(i64::from(pid)));
            assert_eq!(row.agent_command.as_deref(), Some(executable));
        }
    }
}
