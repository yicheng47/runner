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

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use self::app_state::ensure_app_state_table;
use self::migrations::run_migrations;
use self::seed::seed_defaults;
use crate::error::Result;

pub(crate) use self::app_state::{app_state_get, app_state_set};
pub use self::app_state::{
    login_shell_env_lkg, runtime_overrides, set_login_shell_env_lkg, set_runtime_override,
    LoginShellEnvLkg,
};

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn open_pool(db_path: &Path) -> Result<DbPool> {
    let manager = SqliteConnectionManager::file(db_path).with_init(init_connection);
    build_pool(manager, 8, true)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<DbPool> {
    // Tests get schema only — the default-crew seed would pollute the
    // empty starting state most command tests assume.
    let manager = SqliteConnectionManager::memory().with_init(init_connection);
    build_pool(manager, 1, false)
}

fn build_pool(manager: SqliteConnectionManager, max_size: u32, seed: bool) -> Result<DbPool> {
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
