use rusqlite::{params, Connection};

use super::app_state::{ensure_app_state_table, SEED_MARKER_KEY};
use crate::error::Result;

// Historical first-launch seed IDs kept for migration 0031 so upgraded
// seed-only DBs still show onboarding, and shared templates reused by
// onboarding when creating the user's starter crew (#1171).
pub(super) const SEED_CREW_ID: &str = "01K000DEFAULT000PEERCODING01";
pub(super) const SEED_CODER_ROLE_ID: &str = "01K000DEFAULT000RUNNERCODER01";
pub(super) const SEED_REVIEWER_ROLE_ID: &str = "01K000DEFAULT000RUNNERREVW01";

// The starter crew and the copyable example share one source of truth.
// Runner prompts stay persona-only so the templates also work in direct
// chat; mission workflow and channel guidance live in the crew addendum.
pub(crate) const SEED_CODER_PROMPT: &str =
    include_str!("../../../../examples/pair-coding/coder.md");
pub(crate) const SEED_REVIEWER_PROMPT: &str =
    include_str!("../../../../examples/pair-coding/reviewer.md");
pub(crate) const SEED_CREW_ADDENDUM: &str =
    include_str!("../../../../examples/pair-coding/team-conventions.md");

pub(super) fn seed_defaults(conn: &mut Connection) -> Result<()> {
    ensure_app_state_table(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO _app_state (key, value) VALUES (?1, ?2)",
        params![SEED_MARKER_KEY, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
