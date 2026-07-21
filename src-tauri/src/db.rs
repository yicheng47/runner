// SQLite persistence for crews, runners, missions, and sessions.
//
// Schema lives in migrations/0001_init.sql and mirrors arch §7.1 verbatim.
// The pool is opened once at app start with WAL mode + foreign keys; later
// chunks pull connections from it via Tauri state.

use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

use crate::error::Result;

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

// Pre-release squash: the original 0001..0008 collapsed into one
// init file. Real schema migrations resume from 0002.
//
// 0002: persona-only rewrite of the seeded Build squad system_prompts
// (#51). UPDATE-only on the seed's fixed IDs, so renamed / deleted
// runners on existing installs are unaffected. (Was 0003 pre-rename
// — the freed 0002 slot used to hold the default-crew SQL seed,
// which now lives in `seed_default_crew` below.)
// 0003: nullable runtime_* columns on `sessions` from the old
// runtime migration. `runtime` + `runtime_session` still identify
// the live PTY runtime session while the app is running;
// `runtime_socket`, `runtime_window`, and `runtime_pane` are legacy
// and unused by new PTY-runtime writes.
// 0004: adds `archived_at` to missions so the workspace can filter
// archived missions out of search/list surfaces without conflating
// them with `status = 'completed'`. Backfills existing completed
// rows so their archived_at = stopped_at.
// 0005: adds `system_prompt_addendum` (TEXT, nullable) to crews —
// Layer 2 of the system-prompt stack (#54). Spliced between
// platform preamble and runner persona on mission spawns only.
// No backfill; seeded Build squad rows stay NULL.
// 0006: drops `crews.signal_types`. CLI validation is now enum-based
// in runner-core (`KnownSignalType`); the per-crew column + sidecar
// they used to feed no longer have a consumer. See feature 20.
// 0007: makes direct-chat `sessions.runner_id` nullable and adds
// `agent_runtime` / `agent_command` so runtime-only chats can resume
// without a persisted runner template (#195).
// 0008: drops `crews.orchestrator_policy`. Deprecated in #247
// (superseded by `system_prompt_addendum`) and read-only since; it
// fed no prompt and was never written, so the drop is behavior-neutral.
// 0009: persists sidebar folders and stable tab identities. Tab layout
// remains a JSON blob; folder deletion is RESTRICTed so the command must
// archive and remove member tabs transactionally.
// 0010: persists tab-level completion and viewed watermarks for direct-chat
// attention indicators across navigation, windows, and app restarts.
// 0011: adds cwd-bound projects and nullable project membership on sessions
// and missions. Deleting a project unbinds its work via ON DELETE SET NULL.
// 0012: removes folder/project collapse state from SQLite. Expansion is
// per-window view state owned by the sidebar.
// 0013: adds nullable `slots.runtime_override` — per-slot engine choice
// resolved as `slot.runtime_override ?? runner.runtime` at spawn.
// Validated against the runtime registry on write (feature 41).
// 0014: feature 44 — one `nodes` table replaces folders/tabs/pointer
// grouping/pin flags as the sidebar tree (`parent_id` + `position`).
// The SQL copies rows and renames the source tables to `*_legacy`;
// `backfill_0014_nodes` (same transaction) resolves every tab's
// project parent from its layout's member sessions, seeds
// `pinned_position` from the pin flags, and re-seeds `position` per
// parent scope over the pre-migration visual sort.
// 0015: retires folder nodes. Their children are promoted to root and
// spliced into the folder's root position; the 0014 legacy tables are
// dropped in the same transaction.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_persona_only_seeds.sql")),
    (3, include_str!("../migrations/0003_session_runtime.sql")),
    (
        4,
        include_str!("../migrations/0004_mission_archived_at.sql"),
    ),
    (
        5,
        include_str!("../migrations/0005_crew_system_prompt_addendum.sql"),
    ),
    (
        6,
        include_str!("../migrations/0006_drop_crews_signal_types.sql"),
    ),
    (
        7,
        include_str!("../migrations/0007_direct_runtime_sessions.sql"),
    ),
    (
        8,
        include_str!("../migrations/0008_drop_crews_orchestrator_policy.sql"),
    ),
    (9, include_str!("../migrations/0009_folders_tabs.sql")),
    (10, include_str!("../migrations/0010_tab_attention.sql")),
    (11, include_str!("../migrations/0011_projects.sql")),
    (
        12,
        include_str!("../migrations/0012_drop_collapsed_view_state.sql"),
    ),
    (
        13,
        include_str!("../migrations/0013_slot_runtime_override.sql"),
    ),
    (14, include_str!("../migrations/0014_nodes.sql")),
    (15, include_str!("../migrations/0015_retire_folders.sql")),
];

// Default-data seed: ships the Build squad starter crew on first launch.
//
// Runs at most once per database. The marker
// `_app_state.default_crew_seeded` records that the seed step has been
// considered for this DB so we don't recreate Build squad if the user
// later deletes everything ("first launch" must mean *first* launch,
// not "any future launch where you happen to have zero crews").
//
// Even on first launch we only apply the seed when the DB has zero
// crews AND zero runners. If the user has *any* prior data — e.g.
// they ran the build-squad.seed.sh fixture against this DB before
// opening the app — we skip cleanly and still set the marker. This
// avoids the partial-crew failure mode where a colliding runner
// handle would leave Build squad missing its lead, while the start-
// mission UI still treated it as launchable.
//
// Tests skip this entire path so command tests can assume an empty
// starting state.

const SEED_MARKER_KEY: &str = "default_crew_seeded";

// Pinned IDs for the seeded rows. These are referenced by
// `0002_persona_only_seeds.sql`'s WHERE clauses, so they must match
// the values that migration's UPDATEs key on.
const SEED_CREW_ID: &str = "01K000DEFAULT000BUILDSQUAD01";
const SEED_ARCHITECT_RUNNER_ID: &str = "01K000DEFAULT000RUNNERARCH01";
const SEED_IMPL_RUNNER_ID: &str = "01K000DEFAULT000RUNNERIMPL01";
const SEED_REVIEWER_RUNNER_ID: &str = "01K000DEFAULT000RUNNERREVW01";
const SEED_TIMESTAMP: &str = "2026-05-03T00:00:00Z";

// Auto permission mode args for the default Codex seed:
// `codex --ask-for-approval on-request --sandbox workspace-write`.
// This matches the new-runner form's default runtime + permission
// mode without relying on claude-code's plan-gated Auto mode.
const SEED_RUNNER_ARGS_JSON: &str =
    r#"["--ask-for-approval","on-request","--sandbox","workspace-write"]"#;

// Persona-only system prompts shared with `tests/fixtures/system-prompts/*.md`.
// Keeping a single source of truth means the migration 0002 UPDATE
// pin (which targets the *pre*-rewrite text) and the seed (which
// writes the *current* text) can never disagree about what the
// "current" persona looks like.
const SEED_ARCHITECT_PROMPT: &str =
    include_str!("../../tests/fixtures/system-prompts/architect.md");
const SEED_IMPL_PROMPT: &str = include_str!("../../tests/fixtures/system-prompts/impl.md");
const SEED_REVIEWER_PROMPT: &str = include_str!("../../tests/fixtures/system-prompts/reviewer.md");

fn seed_defaults(conn: &mut Connection) -> Result<()> {
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
    let runner_count: i64 = conn.query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if crew_count == 0 && runner_count == 0 {
        seed_default_crew(&tx)?;
    }
    tx.execute(
        "INSERT INTO _app_state (key, value) VALUES (?1, ?2)",
        params![SEED_MARKER_KEY, chrono::Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Insert the Build squad crew, three runners (architect / impl /
/// reviewer), and three slots inside the caller's transaction.
/// Replaces the legacy `0002_default_crew.sql` seed file: the same
/// shape, but written in Rust so the column layout is owned by the
/// same code that handles user-driven runner creates and so
/// permission-mode args flow through as a single string constant
/// (`SEED_RUNNER_ARGS_JSON`) instead of a hand-encoded JSON literal
/// scattered across three INSERT statements.
fn seed_default_crew(tx: &rusqlite::Transaction) -> Result<()> {
    tx.execute(
        "INSERT INTO crews (id, name, purpose, goal, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![
            SEED_CREW_ID,
            "Build squad",
            "Plan, build, and review a single feature end-to-end. \
             Architect dispatches, implementer ships, reviewer gates merge.",
            "Definition of done = code merged behind a green test suite and a clean \
             review pass, with a one-paragraph human-readable summary posted as a \
             broadcast.",
            SEED_TIMESTAMP,
        ],
    )?;

    insert_seed_runner(
        tx,
        SEED_ARCHITECT_RUNNER_ID,
        "architect",
        "Architect",
        SEED_ARCHITECT_PROMPT,
    )?;
    insert_seed_runner(
        tx,
        SEED_IMPL_RUNNER_ID,
        "impl",
        "Implementation",
        SEED_IMPL_PROMPT,
    )?;
    insert_seed_runner(
        tx,
        SEED_REVIEWER_RUNNER_ID,
        "reviewer",
        "Reviewer",
        SEED_REVIEWER_PROMPT,
    )?;

    insert_seed_slot(
        tx,
        "01K000DEFAULT000SLOTARCH0001",
        SEED_ARCHITECT_RUNNER_ID,
        "architect",
        0,
        true,
    )?;
    insert_seed_slot(
        tx,
        "01K000DEFAULT000SLOTIMPL0001",
        SEED_IMPL_RUNNER_ID,
        "impl",
        1,
        false,
    )?;
    insert_seed_slot(
        tx,
        "01K000DEFAULT000SLOTREVW0001",
        SEED_REVIEWER_RUNNER_ID,
        "reviewer",
        2,
        false,
    )?;

    Ok(())
}

fn insert_seed_runner(
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
    tx.execute(
        "INSERT INTO runners (
            id, handle, display_name, runtime, command, args_json,
            system_prompt, model, effort, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'codex', 'codex', ?4, ?5,
                   NULL, NULL, ?6, ?6)",
        params![
            id,
            handle,
            display_name,
            SEED_RUNNER_ARGS_JSON,
            prompt,
            SEED_TIMESTAMP,
        ],
    )?;
    Ok(())
}

fn insert_seed_slot(
    tx: &rusqlite::Transaction,
    id: &str,
    runner_id: &str,
    slot_handle: &str,
    position: i64,
    lead: bool,
) -> Result<()> {
    tx.execute(
        "INSERT INTO slots (id, crew_id, runner_id, slot_handle, position, lead, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            SEED_CREW_ID,
            runner_id,
            slot_handle,
            position,
            lead as i64,
            SEED_TIMESTAMP,
        ],
    )?;
    Ok(())
}

fn run_migrations(conn: &mut Connection) -> Result<()> {
    run_migrations_up_to(conn, i64::MAX)
}

/// Apply pending migrations up to and including `max_version`. Only the
/// migration tests cap this — production always runs the full set.
fn run_migrations_up_to(conn: &mut Connection, max_version: i64) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
         )",
    )?;
    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _migrations",
        [],
        |row| row.get(0),
    )?;
    // Each migration + its `_migrations` bookkeeping row runs in a single
    // IMMEDIATE transaction: a crash mid-apply rolls back the DDL so the next
    // startup retries the same version instead of replaying it onto a
    // partially-migrated schema (which would fail on `CREATE TABLE crews`).
    for (version, sql) in MIGRATIONS {
        if *version > current && *version <= max_version {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute_batch(sql)?;
            // Data backfills that need Rust (JSON parsing, cross-table
            // resolution) run inside the migration's transaction.
            if *version == 14 {
                backfill_0014_nodes(&tx)?;
            }
            if *version == 15 {
                backfill_0015_retire_folders(&tx)?;
            }
            tx.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
    }
    Ok(())
}

/// Replace each root folder with its children while preserving the
/// root order and each folder scope's order. Folder nodes were root-only
/// by invariant, so a single display-order walk produces the flattened
/// root sequence without fabricating project bindings.
fn backfill_0015_retire_folders(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    let roots: Vec<(String, String)> = tx
        .prepare(
            "SELECT id, type FROM nodes WHERE parent_id IS NULL
             ORDER BY position, created_at",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut flattened = Vec::new();
    for (id, node_type) in roots {
        if node_type != "folder" {
            flattened.push(id);
            continue;
        }
        let children: Vec<String> = tx
            .prepare(
                "SELECT id FROM nodes WHERE parent_id = ?1
                 ORDER BY pinned_position IS NULL, pinned_position,
                          position, created_at",
            )?
            .query_map([&id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute(
            "UPDATE nodes SET parent_id = NULL WHERE parent_id = ?1",
            [&id],
        )?;
        tx.execute("DELETE FROM nodes WHERE id = ?1 AND type = 'folder'", [&id])?;
        flattened.extend(children);
    }
    for (position, id) in flattened.iter().enumerate() {
        tx.execute(
            "UPDATE nodes SET position = ?2 WHERE id = ?1",
            params![id, position as i64],
        )?;
    }
    Ok(())
}

/// Rust half of migration 0014 (feature 44), run in the same
/// transaction as the SQL copy. Three steps the SQL can't do:
///
/// 1. Tabs whose member sessions (from the layout JSON) all share one
///    `project_id` move under that project's node — the grouping the
///    sidebar used to derive at render time. Every tab is examined,
///    foldered ones included: the old sidebar's project partition ran
///    on members alone, so a foldered tab with a unanimous project
///    rendered under the project, never under its folder.
/// 2. `position` re-seeds per parent scope with a row number over the
///    pre-migration visual sort, so the migrated sidebar renders in
///    exactly the same order as before: root = projects, then
///    ungrouped missions (pinned first, newest started), then folders,
///    then loose tabs; inside a project = ex-foldered tabs before
///    ex-root tabs (the old tab query's folder-first key), after its
///    missions; inside a folder = tabs by stored order.
/// 3. Pin flags seed `pinned_position`: a tab is pinned when every
///    member session is pinned (the sidebar's rule), a mission when
///    `pinned_at` is set; positions are assigned in the display-order
///    walk of the tree from step 2.
fn backfill_0014_nodes(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    use crate::repo::node::session_ids_from_layout;
    use rusqlite::OptionalExtension;

    // Step 1: project parents for tabs with a unanimous member project.
    let all_tabs: Vec<(String, Option<String>)> = tx
        .prepare("SELECT id, layout FROM nodes WHERE type = 'tab'")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (tab_id, layout) in &all_tabs {
        let members = layout
            .as_deref()
            .map(session_ids_from_layout)
            .unwrap_or_default();
        if members.is_empty() {
            continue;
        }
        let mut shared_project: Option<String> = None;
        let mut all_share = true;
        for (index, session_id) in members.iter().enumerate() {
            let project_id: Option<Option<String>> = tx
                .query_row(
                    "SELECT project_id FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get(0),
                )
                .optional()?;
            let project_id = project_id.flatten();
            if project_id.is_none() || (index > 0 && project_id != shared_project) {
                all_share = false;
                break;
            }
            shared_project = project_id;
        }
        if !all_share {
            continue;
        }
        if let Some(project_id) = shared_project {
            // The project node's id is the project's id (1:1 copy).
            tx.execute(
                "UPDATE nodes SET parent_id = ?2 WHERE id = ?1",
                params![tab_id, project_id],
            )?;
        }
    }

    // Orderings mirroring the pre-migration sidebar. Missions:
    // repo::mission::list — pinned first (newest pin first), then
    // newest started. Tabs/folders/projects: stored position order.
    let ids = |sql: &str, scope: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<Vec<String>> {
        tx.prepare(sql)?
            .query_map(scope, |row| row.get(0))?
            .collect()
    };
    let mission_order = "SELECT n.id FROM nodes n
                          JOIN missions m ON m.id = n.ref_id
                         WHERE n.type = 'mission' AND n.parent_id IS ?1
                         ORDER BY m.pinned_at IS NULL, m.pinned_at DESC, m.started_at DESC";
    // The extra folder-provenance key only bites inside project scopes,
    // where ex-foldered and ex-root tabs mix: the old tab query listed
    // foldered tabs first, so they keep leading here.
    let tab_order = "SELECT id FROM nodes
                     WHERE type = 'tab' AND parent_id IS ?1
                     ORDER BY (SELECT t.folder_id FROM tabs_legacy t
                                WHERE t.id = nodes.id) IS NULL,
                              position, created_at";

    // Step 2: display-order walk — roots first, each container's
    // children right after it — assigning positions per scope.
    let projects = ids(
        "SELECT id FROM nodes WHERE type = 'project'
         ORDER BY position, created_at",
        &[],
    )?;
    let folders = ids(
        "SELECT id FROM nodes WHERE type = 'folder'
         ORDER BY position, created_at",
        &[],
    )?;
    let none: Option<String> = None;
    let root_missions = ids(mission_order, &[&none])?;
    let loose_tabs = ids(tab_order, &[&none])?;

    let mut display_order: Vec<String> = Vec::new();
    let mut root: Vec<String> = Vec::new();
    for project_id in &projects {
        root.push(project_id.clone());
        display_order.push(project_id.clone());
        let mut children = ids(mission_order, &[project_id])?;
        children.extend(ids(tab_order, &[project_id])?);
        for (position, id) in children.iter().enumerate() {
            tx.execute(
                "UPDATE nodes SET position = ?2 WHERE id = ?1",
                params![id, position as i64],
            )?;
        }
        display_order.extend(children);
    }
    root.extend(root_missions);
    for folder_id in &folders {
        root.push(folder_id.clone());
        let children = ids(tab_order, &[folder_id])?;
        for (position, id) in children.iter().enumerate() {
            tx.execute(
                "UPDATE nodes SET position = ?2 WHERE id = ?1",
                params![id, position as i64],
            )?;
        }
    }
    root.extend(loose_tabs);
    for (position, id) in root.iter().enumerate() {
        tx.execute(
            "UPDATE nodes SET position = ?2 WHERE id = ?1",
            params![id, position as i64],
        )?;
    }
    // Non-project roots and their children join the walk after the
    // project blocks, in root order.
    for id in root.iter().filter(|id| !projects.contains(id)) {
        display_order.push(id.clone());
        let children = ids(
            "SELECT id FROM nodes WHERE parent_id = ?1
             ORDER BY position, created_at",
            &[id],
        )?;
        display_order.extend(children);
    }

    // Step 3: pinned_position over the display order.
    let mut pinned_position: i64 = 0;
    for id in &display_order {
        let (node_type, ref_id, layout): (String, Option<String>, Option<String>) = tx.query_row(
            "SELECT type, ref_id, layout FROM nodes WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let pinned = match node_type.as_str() {
            "mission" => {
                let Some(mission_id) = ref_id else {
                    continue;
                };
                tx.query_row(
                    "SELECT pinned_at IS NOT NULL FROM missions WHERE id = ?1",
                    [&mission_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false)
            }
            "tab" => {
                let members = layout
                    .as_deref()
                    .map(session_ids_from_layout)
                    .unwrap_or_default();
                !members.is_empty()
                    && members.iter().try_fold(
                        true,
                        |all, session_id| -> rusqlite::Result<bool> {
                            let pinned: Option<bool> = tx
                                .query_row(
                                    "SELECT pinned_at IS NOT NULL FROM sessions WHERE id = ?1",
                                    [session_id],
                                    |row| row.get(0),
                                )
                                .optional()?;
                            Ok(all && pinned.unwrap_or(false))
                        },
                    )?
            }
            _ => false,
        };
        if pinned {
            tx.execute(
                "UPDATE nodes SET pinned_position = ?2 WHERE id = ?1",
                params![id, pinned_position],
            )?;
            pinned_position += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::ErrorCode;

    fn insert_crew(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![id, format!("crew-{id}"), "2026-04-22T00:00:00Z"],
        )
        .unwrap();
    }

    fn insert_runner(conn: &Connection, id: &str, handle: &str) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'shell', 'sh', ?4, ?4)",
            params![
                id,
                handle,
                format!("{handle} display"),
                "2026-04-22T00:00:00Z"
            ],
        )
    }

    fn insert_slot(
        conn: &Connection,
        id: &str,
        crew_id: &str,
        runner_id: &str,
        slot_handle: &str,
        position: i64,
        lead: i64,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                crew_id,
                runner_id,
                slot_handle,
                position,
                lead,
                "2026-04-22T00:00:00Z"
            ],
        )
    }

    #[test]
    fn migrations_bootstrap_all_tables() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN
                     ('crews','runners','slots','missions','sessions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn container_collapse_state_is_not_in_the_database() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();

        for table in ["nodes", "projects"] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert!(!columns.iter().any(|column| column == "collapsed"));
        }
    }

    // The "at most one lead per crew" invariant moves to the slot
    // commands; covered by the slot_set_lead test in commands::slot.
    // The schema no longer has the partial unique index that used to
    // enforce it.

    #[test]
    fn runner_handle_is_globally_unique() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_runner(&conn, "r1", "shared").unwrap();
        let err = insert_runner(&conn, "r2", "shared").unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn same_runner_can_join_multiple_crews() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_crew(&conn, "c2");
        insert_runner(&conn, "r1", "shared").unwrap();

        insert_slot(&conn, "s1", "c1", "r1", "alpha-c1", 0, 1).unwrap();
        insert_slot(&conn, "s2", "c2", "r1", "alpha-c2", 0, 1).unwrap();
    }

    #[test]
    fn same_runner_can_fill_multiple_slots_in_one_crew() {
        // The whole point of the slot redesign: the same runner
        // template can sit in two slots of the same crew with
        // different in-crew handles.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "claude").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "architect", 0, 1).unwrap();
        insert_slot(&conn, "s2", "c1", "r1", "reviewer", 1, 0).unwrap();
    }

    #[test]
    fn slot_handle_is_unique_per_crew() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_runner(&conn, "r2", "beta").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "lead-slot", 0, 1).unwrap();
        let err = insert_slot(&conn, "s2", "c1", "r2", "lead-slot", 1, 0).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn position_is_unique_per_crew() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_runner(&conn, "r2", "beta").unwrap();

        insert_slot(&conn, "s1", "c1", "r1", "alpha", 0, 1).unwrap();
        let err = insert_slot(&conn, "s2", "c1", "r2", "beta", 0, 0).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
    }

    #[test]
    fn json_blob_columns_roundtrip() {
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();

        let env = serde_json::json!({"FOO": "bar", "BAZ": "qux"});
        let args = serde_json::json!(["--flag", "--val=1"]);
        conn.execute(
            "INSERT INTO runners (
                id, handle, display_name, runtime, command,
                args_json, env_json, created_at, updated_at
             ) VALUES ('r1','test-impl','Impl','shell','sh',?1,?2,?3,?3)",
            params![args.to_string(), env.to_string(), "2026-04-22T00:00:00Z"],
        )
        .unwrap();

        let (args_raw, env_raw): (String, String) = conn
            .query_row(
                "SELECT args_json, env_json FROM runners WHERE id = 'r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&args_raw).unwrap(),
            args
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&env_raw).unwrap(),
            env
        );
    }

    #[test]
    fn deleting_crew_cascades_slot_rows_only() {
        // Runners are global templates — deleting a crew should strip
        // its slots but leave the runner template intact so other
        // crews (or direct chats) can keep using it.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        insert_crew(&conn, "c1");
        insert_runner(&conn, "r1", "alpha").unwrap();
        insert_slot(&conn, "s1", "c1", "r1", "alpha", 0, 1).unwrap();

        conn.execute("DELETE FROM crews WHERE id = 'c1'", [])
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners WHERE id = 'r1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let slot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM slots WHERE runner_id = 'r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runner_count, 1, "runner template must survive crew delete");
        assert_eq!(slot_count, 0, "slots cascade with the crew");
    }

    #[test]
    fn seed_defaults_inserts_build_squad_on_empty_db() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        let slot_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 1);
        assert_eq!(runner_count, 3);
        assert_eq!(slot_count, 3);

        let lead_handle: String = conn
            .query_row("SELECT slot_handle FROM slots WHERE lead = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(lead_handle, "architect");

        let codex_seed_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM runners
                  WHERE runtime = 'codex'
                    AND command = 'codex'
                    AND args_json = ?1
                    AND model IS NULL
                    AND effort IS NULL",
                params![SEED_RUNNER_ARGS_JSON],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            codex_seed_count, 3,
            "all seeded runners should use codex Auto with inherited model/effort",
        );
    }

    /// Verbatim copy of the pre-#51 architect `system_prompt`
    /// (from `c8e2e6f:src-tauri/migrations/0002_default_crew.sql`,
    /// before that file was deleted in favor of the Rust
    /// `seed_default_crew`) — the original SQL string literal had
    /// every `'` doubled to `''`; here it's a Rust string so we use
    /// the literal `'`. The persona migration's WHERE clause pins on
    /// this exact text so users who edited the row in place are not
    /// wiped on upgrade. If the migration's WHERE pin ever drifts
    /// from this constant the
    /// `migration_0002_persona_rewrites_pristine_old_seed` test goes
    /// red.
    const PRE_51_ARCHITECT_SEED: &str =
        "You are the architect for this crew. When the mission starts, your job is
to decompose the goal and dispatch tasks to the right slots — not to
implement the work yourself.

On `mission_goal`:

1. Read the goal carefully. If it is ambiguous or missing context you need
   to plan, escalate with:
       runner signal ask_human --payload '{\"prompt\":\"…\",\"choices\":[\"…\",\"…\"]}'
   Do not start dispatching until the goal is workable.
2. Break the goal into 2–5 well-scoped tasks. Each task names exactly one
   target slot, the deliverable, the file paths or interfaces in scope,
   and the acceptance criteria (tests to add, behavior to verify).
3. Send each task as a directed message:
       runner msg post --to <slot_handle> \"<task>\"
   Do not broadcast tasks. Broadcasts (omit --to) are reserved for
   crew-wide updates (\"I will pause dispatch for 5 minutes\",
   \"@reviewer is now the gate before merge\").
4. Keep an inline task ledger so you can track which slot is working what
   and what they have reported back.

While the mission runs:

- Read your inbox with `runner msg read` — pull-based, only shows unread.
- When a worker reports completion, audit the diff against the goal and
  your acceptance criteria. If something is missing, send a follow-up to
  the same slot — do not silently move on.
- If two slots disagree on an interface, decide. Workers escalate via
  `ask_lead`; the buck stops with you. State the decision and reasoning
  in one message and direct it back.
- Status discipline: report `runner status idle` whenever you are waiting
  on workers and have nothing else to dispatch.

When the mission goal is satisfied:

- If there is any ambiguity, confirm with `ask_human` before declaring
  done. Otherwise post a final summary as a broadcast naming what shipped
  and what was deferred.

Constraints:

- You write plans, not code. If you find yourself opening a file to edit,
  stop and dispatch instead.
- Stay within the goal. Out-of-scope cleanup is a follow-up mission, not
  a silent expansion of the current one.

Talking to the human:

- The human watches the workspace feed, not your TUI scrollback. Always
  reply via `runner msg post --to human \"<your reply>\"`. Typing into the
  TUI leaves your reply in scrollback only.
- Their input lands in your TUI without a `runner msg post` envelope
  (sometimes prefixed `[human_said]`). `human` is a reserved virtual
  handle for this two-way path.";

    /// New post-#51 architect persona (mirrors
    /// tests/fixtures/system-prompts/architect.md, sans the trailing
    /// newline that the .md file ends with). Dropping the trailing
    /// newline matches the SQL literal's body.
    fn new_architect_persona() -> String {
        let md = include_str!("../../tests/fixtures/system-prompts/architect.md");
        md.trim_end_matches('\n').to_string()
    }

    /// Run only migration 0002's UPDATE statements directly,
    /// bypassing `run_migrations`' `_migrations`-version gate. Used
    /// by the preserve / rewrite tests so they can pre-insert a
    /// runner row in whatever shape they want and then exercise the
    /// migration on it.
    fn apply_0002_persona_rewrite(conn: &Connection) {
        conn.execute_batch(include_str!("../migrations/0002_persona_only_seeds.sql"))
            .unwrap();
    }

    #[test]
    fn migration_0002_persona_preserves_customized_system_prompts() {
        // Reviewer-codex flagged this on #51: the persona migration
        // must NOT clobber a user who edited their seeded
        // architect/impl/reviewer row in place (same id, customized
        // prompt). The WHERE pin on the pre-#51 seed text is what
        // makes the migration idempotent for customized rows.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let custom = "My customized architect prompt — please do not overwrite.";
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, system_prompt,
                 created_at, updated_at)
             VALUES ('01K000DEFAULT000RUNNERARCH01', 'architect', 'Custom A',
                     'claude-code', 'claude', ?1,
                     '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z')",
            params![custom],
        )
        .unwrap();
        apply_0002_persona_rewrite(&conn);
        let preserved: String = conn
            .query_row(
                "SELECT system_prompt FROM runners
                  WHERE id = '01K000DEFAULT000RUNNERARCH01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            preserved, custom,
            "persona migration must preserve a customized architect system_prompt",
        );
    }

    #[test]
    fn migration_0002_persona_rewrites_pristine_old_seed() {
        // Sanity check the WHERE pin isn't so strict it never matches
        // anything: a row carrying the EXACT pre-#51 architect seed
        // (an unedited install) must get rewritten to the new
        // persona text. Mirrors what shipping users on v0.1.x will
        // actually see when the persona migration runs.
        let pool = open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, system_prompt,
                 created_at, updated_at)
             VALUES ('01K000DEFAULT000RUNNERARCH01', 'architect', 'Architect',
                     'claude-code', 'claude', ?1,
                     '2026-05-03T00:00:00Z', '2026-05-03T00:00:00Z')",
            params![PRE_51_ARCHITECT_SEED],
        )
        .unwrap();
        apply_0002_persona_rewrite(&conn);
        let rewritten: String = conn
            .query_row(
                "SELECT system_prompt FROM runners
                  WHERE id = '01K000DEFAULT000RUNNERARCH01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            rewritten,
            new_architect_persona(),
            "persona migration must rewrite the pristine pre-#51 architect seed to the new persona",
        );
    }

    #[test]
    fn seeded_personas_contain_no_bus_verbs() {
        // Regression guard for #51: the seed system_prompts must be
        // persona-only — the bus contract (runner msg post / runner
        // msg read / ask_lead, plus @<handle> framing) is now the
        // job of WORKER_COORDINATION_PREAMBLE in the runtime prompt
        // composer for mission first-turn argv delivery. If a
        // future drift adds bus verbs back into the seed prompts,
        // direct chats would surface verbs that don't work
        // (RUNNER_CREW_ID / RUNNER_MISSION_ID / RUNNER_EVENT_LOG are
        // unset off-bus, the bundled `runner` CLI is not on PATH).
        //
        // Post-renumber the seed lives in `seed_default_crew` (Rust)
        // and reads the .md files via `include_str!` — so checking
        // the .md fixtures is checking the seed, no separate SQL
        // pin needed.
        let banned_substrings = [
            "runner msg post",
            "runner msg read",
            "runner status idle",
            "ask_lead",
            "ask_human",
        ];
        for (name, md) in [
            ("architect.md", SEED_ARCHITECT_PROMPT),
            ("impl.md", SEED_IMPL_PROMPT),
            ("reviewer.md", SEED_REVIEWER_PROMPT),
        ] {
            for needle in banned_substrings {
                assert!(
                    !md.contains(needle),
                    "{name} must not contain bus verb {needle:?}",
                );
            }
            // @-handle pattern: @<ASCII-alpha-start>. Persona content
            // currently uses no @-symbol; a single bare scan
            // (no regex dep) catches any future drift loudly.
            let bytes = md.as_bytes();
            for i in 0..bytes.len().saturating_sub(1) {
                if bytes[i] == b'@' && bytes[i + 1].is_ascii_alphabetic() {
                    let snippet_end = (i + 24).min(bytes.len());
                    let snippet = String::from_utf8_lossy(&bytes[i..snippet_end]);
                    panic!("{name} must not contain @-handle framing (found near {snippet:?})");
                }
            }
        }
    }

    #[test]
    fn seed_defaults_skips_when_user_has_a_crew() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        insert_crew(&conn, "user-c1");
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 1, "should not seed when crews already exist");
    }

    #[test]
    fn seed_defaults_skips_when_user_has_a_runner_but_no_crew() {
        // The partial-crew failure mode: a user manually created an
        // `architect` runner template (or ran the seed.sh fixture
        // directly into this DB), then opened the app for the first
        // time. Pre-fix, the migration inserted the Build squad crew
        // and inserted only impl + reviewer runners (architect skipped
        // by the per-handle NOT EXISTS guard), then the slot insert
        // for the architect slot couldn't find our runner ID and
        // skipped — leaving Build squad with two slots and no lead.
        // The start-mission UI treated that as launchable, then the
        // backend rejected it. Now the whole seed bails, marker is
        // still set, and we never produce a partial crew.
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        insert_runner(&conn, "user-r1", "architect").unwrap();
        seed_defaults(&mut conn).unwrap();

        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 0, "should not create Build squad");
        assert_eq!(runner_count, 1, "user's runner stays untouched");
    }

    #[test]
    fn seed_defaults_marker_prevents_reseed_after_user_deletes_everything() {
        // First launch: empty DB → seed runs and marker is recorded.
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();

        // User wipes the seeded data — slots cascade with the crew,
        // runners are global templates so we delete them explicitly.
        conn.execute("DELETE FROM crews", []).unwrap();
        conn.execute("DELETE FROM runners", []).unwrap();
        let crew_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crew_count, 0);

        // Next launch: seed sees the marker and skips, even though
        // the DB looks "empty" again.
        seed_defaults(&mut conn).unwrap();
        let crew_count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            crew_count_after, 0,
            "marker must prevent reseeding after deletion"
        );
    }

    #[test]
    fn seed_defaults_is_idempotent_across_reseeds() {
        let pool = open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        seed_defaults(&mut conn).unwrap();
        seed_defaults(&mut conn).unwrap();

        let runner_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runners", [], |r| r.get(0))
            .unwrap();
        let slot_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runner_count, 3);
        assert_eq!(slot_count, 3);
    }

    /// Migration 0014: seed a pre-migration-shaped DB (schema 13),
    /// run the cutover, and assert the resulting tree — parentage,
    /// per-scope positions over the old visual sort, pin seeding,
    /// watermark/layout carry-over, and the `*_legacy` renames.
    #[test]
    fn migration_0014_builds_the_node_tree_in_the_old_visual_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations_up_to(&mut conn, 13).unwrap();

        // Two projects (positions 1, 0 — stored order differs from
        // insert order on purpose).
        conn.execute_batch(
            "INSERT INTO projects (id, name, cwd, position, created_at) VALUES
                 ('proj-a', 'A', '/tmp/a', 1, '2026-07-01T00:00:00Z'),
                 ('proj-b', 'B', '/tmp/b', 0, '2026-07-01T00:00:00Z');",
        )
        .unwrap();
        // Two folders.
        conn.execute_batch(
            "INSERT INTO folders (id, name, position, created_at) VALUES
                 ('fold-1', 'Work', 0, '2026-07-01T00:00:00Z'),
                 ('fold-2', 'Play', 1, '2026-07-01T00:00:00Z');",
        )
        .unwrap();
        // Sessions: s1/s2 in folder tabs; s3+s4 share proj-a (s4
        // pinned); s5 pinned loose chat; s6/s7 split across projects;
        // s8/s9 share proj-b from inside a folder tab.
        conn.execute_batch(
            "INSERT INTO sessions (id, status, project_id, pinned_at, started_at) VALUES
                 ('s1', 'stopped', NULL, NULL, '2026-07-01T00:00:00Z'),
                 ('s2', 'stopped', NULL, NULL, '2026-07-01T00:00:00Z'),
                 ('s3', 'stopped', 'proj-a', NULL, '2026-07-01T00:00:00Z'),
                 ('s4', 'stopped', 'proj-a', '2026-07-02T00:00:00Z', '2026-07-01T00:00:00Z'),
                 ('s5', 'stopped', NULL, '2026-07-03T00:00:00Z', '2026-07-01T00:00:00Z'),
                 ('s6', 'stopped', 'proj-a', NULL, '2026-07-01T00:00:00Z'),
                 ('s7', 'stopped', 'proj-b', NULL, '2026-07-01T00:00:00Z'),
                 ('s8', 'stopped', 'proj-b', NULL, '2026-07-01T00:00:00Z'),
                 ('s9', 'stopped', 'proj-b', NULL, '2026-07-01T00:00:00Z');",
        )
        .unwrap();
        // Tabs: two foldered (positions 1, 0), one two-member proj-a
        // tab, one pinned loose tab (all members pinned), one
        // mixed-project tab that must stay at root, and one FOLDERED
        // tab whose members unanimously share proj-b — the old sidebar
        // rendered that one under the project, not its folder, so the
        // migration must too. tab-w carries attention watermarks.
        conn.execute_batch(
            r#"INSERT INTO tabs (id, folder_id, name, position, layout, created_at,
                                 last_completed_at, last_viewed_at) VALUES
                 ('tab-w', 'fold-1', 'w', 1,
                  '{"preset":"single","slots":["s1"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', '2026-07-05T00:00:00Z', '2026-07-04T00:00:00Z'),
                 ('tab-x', 'fold-1', 'x', 0,
                  '{"preset":"single","slots":["s2"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', NULL, NULL),
                 ('tab-proj', NULL, 'proj tab', 0,
                  '{"preset":"cols-2","slots":["s3","s4"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', NULL, NULL),
                 ('tab-pin', NULL, 'pinned', 1,
                  '{"preset":"single","slots":["s5"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', NULL, NULL),
                 ('tab-mixed', NULL, 'mixed', 2,
                  '{"preset":"cols-2","slots":["s6","s7"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', NULL, NULL),
                 ('tab-fold-proj', 'fold-1', 'foldered proj tab', 2,
                  '{"preset":"cols-2","slots":["s8","s9"],"sizes":{}}',
                  '2026-07-01T00:00:00Z', NULL, NULL);"#,
        )
        .unwrap();
        // Missions: one bound to proj-a, one pinned at root, one
        // unpinned at root started later, one archived (no node).
        conn.execute_batch(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at,
                                   project_id, pinned_at, archived_at) VALUES
                 ('m-proj', 'c1', 'In project', 'running',
                  '2026-07-01T01:00:00Z', 'proj-a', NULL, NULL),
                 ('m-pin', 'c1', 'Pinned', 'running',
                  '2026-07-01T01:00:00Z', NULL, '2026-07-01T02:00:00Z', NULL),
                 ('m-new', 'c1', 'Newest', 'running',
                  '2026-07-02T01:00:00Z', NULL, NULL, NULL),
                 ('m-arch', 'c1', 'Archived', 'completed',
                  '2026-07-01T01:00:00Z', NULL, NULL, '2026-07-03T00:00:00Z');",
        )
        .unwrap();

        run_migrations_up_to(&mut conn, 14).unwrap();

        let node = |id: &str| -> (Option<String>, i64, String, Option<i64>) {
            conn.query_row(
                "SELECT parent_id, position, type, pinned_position
                   FROM nodes WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
        };

        // Parentage: foldered tabs with no unanimous project keep
        // their folder; unanimous-project tabs move under the project
        // node whether they were loose OR foldered (the old sidebar's
        // project partition ignored folder membership); the mixed tab
        // stayed at root; the bound mission nests under proj-a.
        assert_eq!(node("tab-w").0.as_deref(), Some("fold-1"));
        assert_eq!(node("tab-x").0.as_deref(), Some("fold-1"));
        assert_eq!(node("tab-proj").0.as_deref(), Some("proj-a"));
        assert_eq!(node("tab-fold-proj").0.as_deref(), Some("proj-b"));
        assert_eq!(node("tab-pin").0, None);
        assert_eq!(node("tab-mixed").0, None);
        assert_eq!(node("m-proj").0.as_deref(), Some("proj-a"));
        assert_eq!(node("m-pin").0, None);
        assert_eq!(node("m-new").0, None);

        // Archived mission gets no node.
        let arch_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE ref_id = 'm-arch'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(arch_count, 0);

        // Root order = the old sidebar top-to-bottom: PROJECT section
        // (stored project order: B before A), MISSION section (pinned
        // first, then newest started), CHAT section (folders, then
        // loose tabs by stored position).
        let roots: Vec<String> = conn
            .prepare("SELECT id FROM nodes WHERE parent_id IS NULL ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            roots,
            [
                "proj-b",
                "proj-a",
                "m-pin",
                "m-new",
                "fold-1",
                "fold-2",
                "tab-pin",
                "tab-mixed"
            ]
        );

        // Folder scope keeps stored tab order (position, created_at).
        let folder_children: Vec<String> = conn
            .prepare("SELECT id FROM nodes WHERE parent_id = 'fold-1' ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(folder_children, ["tab-x", "tab-w"]);

        // Project scope: missions first, then tabs — the old nested
        // rendering order.
        let project_children: Vec<String> = conn
            .prepare("SELECT id FROM nodes WHERE parent_id = 'proj-a' ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(project_children, ["m-proj", "tab-proj"]);
        let project_b_children: Vec<String> = conn
            .prepare("SELECT id FROM nodes WHERE parent_id = 'proj-b' ORDER BY position")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(project_b_children, ["tab-fold-proj"]);

        // Pin seeding: the pinned mission and the all-members-pinned
        // loose tab carry pinned_position in display order; the
        // proj-a tab (one unpinned member) does not.
        assert_eq!(node("tab-proj").3, None);
        let m_pin_slot = node("m-pin").3.expect("pinned mission seeded");
        let tab_pin_slot = node("tab-pin").3.expect("pinned tab seeded");
        assert!(m_pin_slot < tab_pin_slot, "display order: mission first");
        assert_eq!(node("m-new").3, None);
        assert_eq!(node("tab-mixed").3, None);

        // Layout and watermarks carry over byte-for-byte.
        let (layout, completed, viewed): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT layout, last_completed_at, last_viewed_at
                   FROM nodes WHERE id = 'tab-w'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(layout, r#"{"preset":"single","slots":["s1"],"sizes":{}}"#);
        assert_eq!(completed.as_deref(), Some("2026-07-05T00:00:00Z"));
        assert_eq!(viewed.as_deref(), Some("2026-07-04T00:00:00Z"));

        // Folder nodes own their names; source tables are renamed,
        // not dropped.
        let folder_name: Option<String> = conn
            .query_row("SELECT name FROM nodes WHERE id = 'fold-1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(folder_name.as_deref(), Some("Work"));
        for (gone, kept) in [("folders", "folders_legacy"), ("tabs", "tabs_legacy")] {
            let count = |table: &str| -> i64 {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap()
            };
            assert_eq!(count(gone), 0, "{gone} should be renamed away");
            assert_eq!(count(kept), 1, "{kept} should survive the cutover");
        }
        let legacy_tabs: i64 = conn
            .query_row("SELECT COUNT(*) FROM tabs_legacy", [], |r| r.get(0))
            .unwrap();
        assert_eq!(legacy_tabs, 6);
    }

    #[test]
    fn migration_0015_promotes_folder_children_in_place_and_drops_legacy_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        run_migrations_up_to(&mut conn, 14).unwrap();

        conn.execute_batch(
            "INSERT INTO nodes
                 (id, parent_id, position, type, name, created_at, pinned_position)
             VALUES
                 ('root-a', NULL, 0, 'tab', 'A', '2026-07-01T00:00:00Z', NULL),
                 ('folder-a', NULL, 1, 'folder', 'Folder A', '2026-07-01T00:00:01Z', NULL),
                 ('root-b', NULL, 2, 'mission', NULL, '2026-07-01T00:00:02Z', NULL),
                 ('folder-empty', NULL, 3, 'folder', 'Empty', '2026-07-01T00:00:03Z', NULL),
                 ('folder-b', NULL, 4, 'folder', 'Folder B', '2026-07-01T00:00:04Z', NULL),
                 ('root-c', NULL, 5, 'project', NULL, '2026-07-01T00:00:05Z', NULL),
                 ('child-a-later', 'folder-a', 7, 'tab', 'Later', '2026-07-01T00:00:07Z', NULL),
                 ('child-a-first', 'folder-a', 2, 'tab', 'First', '2026-07-01T00:00:06Z', 7),
                 ('child-b', 'folder-b', 9, 'mission', NULL, '2026-07-01T00:00:08Z', NULL);",
        )
        .unwrap();

        run_migrations(&mut conn).unwrap();

        let roots: Vec<(String, Option<String>, i64, Option<i64>)> = conn
            .prepare(
                "SELECT id, parent_id, position, pinned_position FROM nodes
                 ORDER BY position, created_at",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            roots,
            [
                ("root-a".to_owned(), None, 0, None),
                ("child-a-first".to_owned(), None, 1, Some(7)),
                ("child-a-later".to_owned(), None, 2, None),
                ("root-b".to_owned(), None, 3, None),
                ("child-b".to_owned(), None, 4, None),
                ("root-c".to_owned(), None, 5, None),
            ]
        );
        let folder_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE type = 'folder'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(folder_count, 0);
        for table in ["folders_legacy", "tabs_legacy"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} should be dropped");
        }
    }

    #[test]
    fn migrations_are_idempotent_on_reopen() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("runner.db");

        {
            let _pool = open_pool(&path).unwrap();
        }
        let pool = open_pool(&path).unwrap();
        let conn = pool.get().unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            applied,
            MIGRATIONS.len() as i64,
            "each migration should apply exactly once"
        );
    }

    #[test]
    fn sessions_has_runtime_columns_after_migration() {
        // Defensive: keep the legacy runtime columns present for
        // existing databases. New PTY-runtime writes use only
        // `runtime` + `runtime_session`; socket/window/pane are
        // legacy and unused since the PTY migration.
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("runner.db");
        let pool = open_pool(&path).unwrap();
        let conn = pool.get().unwrap();
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>("name"))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for required in [
            "runtime",
            "runtime_socket",
            "runtime_session",
            "runtime_window",
            "runtime_pane",
            "runtime_cursor",
        ] {
            assert!(
                columns.iter().any(|c| c == required),
                "sessions.{required} missing; columns = {columns:?}"
            );
        }
    }
}
