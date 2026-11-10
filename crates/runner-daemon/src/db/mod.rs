// SQLite persistence for crews, roles, missions, and sessions.
//
// Schema lives in migrations/0001_init.sql and mirrors arch §7.1 verbatim.
// The pool is opened once at app start with WAL mode + foreign keys; the
// backend shares it through `AppCore`.

mod app_state;
mod migrations;
mod seed;
#[cfg(test)]
mod tests;

use std::path::Path;

use r2d2::{ManageConnection, Pool};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use self::app_state::ensure_app_state_table;
use self::migrations::run_migrations;
use self::seed::seed_defaults;
use crate::error::Result;

pub(crate) use self::app_state::{app_state_get, app_state_set};
pub use self::app_state::{
    files_recent_folders, login_shell_env_lkg, push_files_recent_folder, runtime_launch_options,
    runtime_overrides, set_login_shell_env_lkg, set_runtime_launch_option, set_runtime_override,
    set_terminal_palette, set_usage_lkg, terminal_palette, usage_lkg, LoginShellEnvLkg, UsageLkg,
};

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn open_pool(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(init_connection);
    build_pool(manager, 8, true)
}

/// Disposable fixtures need file-backed WAL visibility, not power-loss durability per commit.
#[cfg(any(test, feature = "test-support"))]
pub fn open_fixture_pool(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        init_connection(conn)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
    });
    build_pool(manager, 8, true)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<DbPool> {
    // Tests get schema only — the default-crew seed would pollute the
    // empty starting state most command tests assume.
    let manager = SqliteConnectionManager::memory().with_init(init_connection);
    build_pool(manager, 1, false)
}

#[cfg(test)]
pub(crate) fn test_connection() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    init_connection(&mut conn)?;
    run_migrations(&mut conn)?;
    Ok(conn)
}

fn build_pool(manager: SqliteConnectionManager, max_size: u32, seed: bool) -> Result<DbPool> {
    // r2d2 retries failed initial connections for 30 s, hiding permanent SQLite errors.
    drop(manager.connect()?);
    let pool = Pool::builder().max_size(max_size).build(manager)?;
    let mut conn = pool.get()?;
    run_migrations(&mut conn)?;
    ensure_app_state_table(&conn)?;
    if seed {
        seed_defaults(&mut conn)?;
    }
    Ok(pool)
}

fn init_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;\n\
         PRAGMA foreign_keys = ON;\n\
         PRAGMA busy_timeout = 5000;",
    )
}
