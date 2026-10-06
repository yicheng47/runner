use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::DbPool;
use crate::error::Result;

pub(super) const SEED_MARKER_KEY: &str = "default_crew_seeded";
const LOGIN_SHELL_ENV_LKG_KEY: &str = "login_shell_env_lkg";
const RUNTIME_OVERRIDES_KEY: &str = "runtime_overrides";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginShellEnvLkg {
    pub env: crate::shell_path::LoginShellEnv,
    pub shell: String,
    pub captured_at: String,
}

pub(super) fn ensure_app_state_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _app_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         )",
    )?;
    Ok(())
}

pub(crate) fn app_state_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM _app_state WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn app_state_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO _app_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn login_shell_env_lkg(pool: &DbPool) -> Result<Option<LoginShellEnvLkg>> {
    let conn = pool.get()?;
    app_state_get(&conn, LOGIN_SHELL_ENV_LKG_KEY)?
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

pub fn set_login_shell_env_lkg(pool: &DbPool, snapshot: &LoginShellEnvLkg) -> Result<()> {
    let conn = pool.get()?;
    app_state_set(
        &conn,
        LOGIN_SHELL_ENV_LKG_KEY,
        &serde_json::to_string(snapshot)?,
    )
}

pub fn runtime_overrides(pool: &DbPool) -> Result<BTreeMap<String, String>> {
    let conn = pool.get()?;
    Ok(app_state_get(&conn, RUNTIME_OVERRIDES_KEY)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default())
}

pub fn set_runtime_override(pool: &DbPool, runtime: &str, path: Option<&str>) -> Result<()> {
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut overrides: BTreeMap<String, String> = app_state_get(&tx, RUNTIME_OVERRIDES_KEY)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    match path {
        Some(path) => {
            overrides.insert(runtime.to_string(), path.to_string());
        }
        None => {
            overrides.remove(runtime);
        }
    }
    app_state_set(
        &tx,
        RUNTIME_OVERRIDES_KEY,
        &serde_json::to_string(&overrides)?,
    )?;
    tx.commit()?;
    Ok(())
}
