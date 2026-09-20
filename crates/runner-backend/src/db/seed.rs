use rusqlite::{params, Connection};

use super::app_state::SEED_MARKER_KEY;
use crate::error::Result;

// Default-data seed: ships the Peer coding starter crew on first launch.
//
// Runs at most once per database. The marker
// `_app_state.default_crew_seeded` records that the seed step has been
// considered for this DB so we don't recreate Peer coding if the user
// later deletes everything ("first launch" must mean *first* launch,
// not "any future launch where you happen to have zero crews").
//
// Even on first launch we only apply the seed when the DB has zero
// crews AND zero roles. If the user has *any* prior data — e.g.
// they loaded another fixture into this DB before
// opening the app — we skip cleanly and still set the marker. This
// avoids the partial-crew failure mode where a colliding role
// handle would leave Peer coding missing its lead, while the start-
// mission UI still treated it as launchable.
//
// Tests skip this entire path so command tests can assume an empty
// starting state.

// Pinned IDs keep first-launch fixtures deterministic. Migration 0002
// still owns the legacy Build squad IDs for historical upgrades; fresh
// databases run migrations before this peer-coding seed is inserted.
pub(super) const SEED_CREW_ID: &str = "01K000DEFAULT000PEERCODING01";
const SEED_CODER_ROLE_ID: &str = "01K000DEFAULT000RUNNERCODER01";
const SEED_REVIEWER_ROLE_ID: &str = "01K000DEFAULT000RUNNERREVW01";
const SEED_TIMESTAMP: &str = "2026-08-01T00:00:00Z";

// Auto permission mode args for the default Codex seed:
// `codex --ask-for-approval on-request --sandbox workspace-write`.
// This matches the new-role form's default runtime + permission
// mode without relying on claude-code's plan-gated Auto mode.
pub(super) const SEED_ROLE_ARGS_JSON: &str =
    r#"["--ask-for-approval","on-request","--sandbox","workspace-write"]"#;

// The shipped crew and the copyable example share one source of truth.
// Runner prompts stay persona-only so the templates also work in direct
// chat; mission workflow and channel guidance live in the crew addendum.
pub(super) const SEED_CODER_PROMPT: &str =
    include_str!("../../../../examples/peer-coding/coder.md");
pub(super) const SEED_REVIEWER_PROMPT: &str =
    include_str!("../../../../examples/peer-coding/reviewer.md");
pub(super) const SEED_CREW_ADDENDUM: &str =
    include_str!("../../../../examples/peer-coding/team-conventions.md");

pub(super) fn seed_defaults(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _app_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
         )",
    )?;
    let already_seeded: bool = conn
        .query_row(
            "SELECT 1 FROM _app_state WHERE key = ?1",
            params![SEED_MARKER_KEY],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if already_seeded {
        return Ok(());
    }

    let crew_count: i64 = conn.query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))?;
    let role_count = crate::repo::role::count(conn)?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if crew_count == 0 && role_count == 0 {
        seed_default_crew(&tx)?;
    }
    tx.execute(
        "INSERT INTO _app_state (key, value) VALUES (?1, ?2)",
        params![SEED_MARKER_KEY, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Insert the two-role Peer coding example inside the caller's
/// transaction. The Rust seed owns the same fields as user-driven
/// creates and reads the copyable example prompts directly.
fn seed_default_crew(tx: &rusqlite::Transaction) -> Result<()> {
    let addendum = SEED_CREW_ADDENDUM.trim_end_matches('\n');
    tx.execute(
        "INSERT INTO crews (
            id, name, purpose, goal, system_prompt_addendum, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            SEED_CREW_ID,
            "Peer coding crew",
            "A two-role coder/reviewer loop for a single implementation task. \
             The coder ships the change; the reviewer audits it; the coder fixes \
             findings until review is clean.",
            "Definition of done: implemented, relevant checks passed, and reviewer \
             reports no remaining must-fix issues.",
            addendum,
            SEED_TIMESTAMP,
        ],
    )?;

    insert_seed_role(tx, SEED_CODER_ROLE_ID, "coder", "Coder", SEED_CODER_PROMPT)?;
    insert_seed_role(
        tx,
        SEED_REVIEWER_ROLE_ID,
        "reviewer",
        "Reviewer",
        SEED_REVIEWER_PROMPT,
    )?;

    insert_seed_slot(
        tx,
        "01K000DEFAULT000SLOTCODER001",
        SEED_CODER_ROLE_ID,
        "coder",
        0,
        true,
    )?;
    insert_seed_slot(
        tx,
        "01K000DEFAULT000SLOTREVW0001",
        SEED_REVIEWER_ROLE_ID,
        "reviewer",
        1,
        false,
    )?;

    Ok(())
}

fn insert_seed_role(
    tx: &rusqlite::Transaction,
    id: &str,
    handle: &str,
    display_name: &str,
    prompt: &str,
) -> Result<()> {
    // Strip the trailing newline the .md fixtures end with so the
    // stored prompt reads like a single paragraph stack — the same
    // shape the legacy SQL seed produced.
    let prompt = prompt.trim_end_matches('\n');
    let timestamp = SEED_TIMESTAMP.parse().expect("valid seed timestamp");
    crate::repo::role::insert(
        tx,
        &crate::repo::role::RoleRow {
            id: id.into(),
            handle: handle.into(),
            display_name: display_name.into(),
            runtime: "codex".into(),
            command: "codex".into(),
            args_json: Some(serde_json::from_str(SEED_ROLE_ARGS_JSON)?),
            working_dir: None,
            system_prompt: Some(prompt.into()),
            env_json: Some(Default::default()),
            model: None,
            effort: None,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )?;
    Ok(())
}

fn insert_seed_slot(
    tx: &rusqlite::Transaction,
    id: &str,
    role_id: &str,
    slot_handle: &str,
    position: i64,
    lead: bool,
) -> Result<()> {
    crate::repo::slot::insert(
        tx,
        &crate::repo::slot::SlotRow {
            id: id.into(),
            crew_id: SEED_CREW_ID.into(),
            role_id: role_id.into(),
            slot_handle: slot_handle.into(),
            position,
            lead,
            runtime_override: None,
            model_override: None,
            effort_override: None,
            added_at: SEED_TIMESTAMP.parse().expect("valid seed timestamp"),
        },
    )?;
    Ok(())
}
