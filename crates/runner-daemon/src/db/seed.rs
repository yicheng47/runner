use std::sync::LazyLock;

use runner_core::template::{parse_template, CrewTemplate, Template};
use rusqlite::{params, Connection};

use super::app_state::{ensure_app_state_table, SEED_MARKER_KEY};
use crate::error::Result;

// Historical first-launch seed IDs kept for migration 0031 so upgraded
// seed-only DBs still show onboarding, and shared templates reused by
// onboarding when creating the user's starter crew (#1171).
pub(super) const SEED_CREW_ID: &str = "01K000DEFAULT000PEERCODING01";
pub(super) const SEED_CODER_ROLE_ID: &str = "01K000DEFAULT000RUNNERCODER01";
pub(super) const SEED_REVIEWER_ROLE_ID: &str = "01K000DEFAULT000RUNNERREVW01";

const PAIR_CODING_TEMPLATE: &str =
    include_str!("../../../../examples/pair-coding/pair-coding.crew.toml");

static PAIR_CODING_CREW: LazyLock<CrewTemplate> = LazyLock::new(|| {
    match parse_template(PAIR_CODING_TEMPLATE)
        .expect("bundled examples/pair-coding/pair-coding.crew.toml must be valid")
    {
        Template::Crew(crew) => crew,
        Template::Role(_) => panic!("pair-coding template must be a crew template"),
    }
});

// The starter crew and the shipped example share one source of truth.
// Runner prompts stay persona-only so the templates also work in direct
// chat; mission workflow and channel guidance live in the crew addendum.
pub(crate) static SEED_CODER_PROMPT: LazyLock<String> = LazyLock::new(|| {
    PAIR_CODING_CREW
        .roles
        .iter()
        .find(|role| role.handle == "coder")
        .and_then(|role| role.system_prompt.clone())
        .expect("pair-coding template must define coder prompt")
});
pub(crate) static SEED_REVIEWER_PROMPT: LazyLock<String> = LazyLock::new(|| {
    PAIR_CODING_CREW
        .roles
        .iter()
        .find(|role| role.handle == "reviewer")
        .and_then(|role| role.system_prompt.clone())
        .expect("pair-coding template must define reviewer prompt")
});
pub(crate) static SEED_CREW_ADDENDUM: LazyLock<String> = LazyLock::new(|| {
    PAIR_CODING_CREW
        .crew
        .conventions
        .clone()
        .expect("pair-coding template must define conventions")
});

pub(super) fn seed_defaults(conn: &mut Connection) -> Result<()> {
    ensure_app_state_table(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO _app_state (key, value) VALUES (?1, ?2)",
        params![SEED_MARKER_KEY, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
