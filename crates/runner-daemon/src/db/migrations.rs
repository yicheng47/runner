use rusqlite::{params, Connection};

use crate::error::Result;

// Pre-release squash: the original 0001..0008 collapsed into one
// init file. Real schema migrations resume from 0002.
//
// 0002: persona-only rewrite of the seeded Build squad system_prompts
// (#51). UPDATE-only on the seed's fixed IDs, so renamed / deleted
// roles on existing installs are unaffected. (Was 0003 pre-rename
// — the freed 0002 slot used to hold the default-crew SQL seed,
// which now lives in `seed::seed_default_crew`.)
// 0003: nullable runtime_* columns on `sessions` from the old
// runtime migration. All six process-host columns are dropped by 0027.
// 0004: adds `archived_at` to missions so the workspace can filter
// archived missions out of search/list surfaces without conflating
// them with `status = 'completed'`. Backfills existing completed
// rows so their archived_at = stopped_at.
// 0005: adds `system_prompt_addendum` (TEXT, nullable) to crews —
// Layer 2 of the system-prompt stack (#54). Spliced between
// platform preamble and role persona on mission spawns only.
// No backfill; seeded Build squad rows stay NULL.
// 0006: drops `crews.signal_types`. CLI validation is now enum-based
// in runner-core (`KnownSignalType`); the per-crew column + sidecar
// they used to feed no longer have a consumer. See feature 20.
// 0007: makes direct-chat `sessions.runner_id` nullable and adds
// `agent_runtime` / `agent_command` so runtime-only chats can resume
// without a persisted role template (#195).
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
// resolved as `slot.runtime_override ?? role.runtime` at spawn.
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
// 0016: persists the last applied PTY size per session so an unsized
// resume can fork at the prior width before any frontend pane is measurable.
// 0017: records sessions that were running at graceful quit so the next
// launch can resume them without treating crash-demoted rows the same way.
// 0018: adds nullable `slots.model_override` so a slot that selects a
// different runtime can pin that runtime's model without mutating the
// reusable role template.
// 0019: persists model/effort on runtime-only direct chats so resume
// keeps the session's selected agent configuration; replaced by 0026/0027.
// 0020: adds nullable `slots.effort_override` so a crew slot can
// override thinking effort independently of its runtime and model.
// 0023: renames the stored runner entity and its foreign-key columns to
// `roles` / `role_id`; direct sessions have referenced it since 0007.
// 0025: removes unused crew purpose and default goal. Mission goals and
// crew conventions remain in their own columns.
// 0024: runtime_options_json on roles, slots and sessions; sessions drops it in 0027.
// 0026: records launch definitions, with a Rust conversion in the same transaction.
// 0027: drops six process-host columns and three session override columns.
// 0028: optional mission crew provenance and explicit dependent-row cleanup.
// 0029: optional managed avatar image path on role templates.
// 0030: role avatar rendering style, defaulting existing roles to initials.
// 0031: optional canonical role avatar theme color.
pub(super) const MIGRATIONS: &[(i64, &str, bool)] = &[
    (1, include_str!("../../migrations/0001_init.sql"), false),
    (
        2,
        include_str!("../../migrations/0002_persona_only_seeds.sql"),
        false,
    ),
    (
        3,
        include_str!("../../migrations/0003_session_runtime.sql"),
        false,
    ),
    (
        4,
        include_str!("../../migrations/0004_mission_archived_at.sql"),
        false,
    ),
    (
        5,
        include_str!("../../migrations/0005_crew_system_prompt_addendum.sql"),
        false,
    ),
    (
        6,
        include_str!("../../migrations/0006_drop_crews_signal_types.sql"),
        false,
    ),
    (
        7,
        include_str!("../../migrations/0007_direct_runtime_sessions.sql"),
        false,
    ),
    (
        8,
        include_str!("../../migrations/0008_drop_crews_orchestrator_policy.sql"),
        false,
    ),
    (
        9,
        include_str!("../../migrations/0009_folders_tabs.sql"),
        false,
    ),
    (
        10,
        include_str!("../../migrations/0010_tab_attention.sql"),
        false,
    ),
    (
        11,
        include_str!("../../migrations/0011_projects.sql"),
        false,
    ),
    (
        12,
        include_str!("../../migrations/0012_drop_collapsed_view_state.sql"),
        false,
    ),
    (
        13,
        include_str!("../../migrations/0013_slot_runtime_override.sql"),
        false,
    ),
    (14, include_str!("../../migrations/0014_nodes.sql"), false),
    (
        15,
        include_str!("../../migrations/0015_retire_folders.sql"),
        false,
    ),
    (
        16,
        include_str!("../../migrations/0016_session_last_size.sql"),
        false,
    ),
    (
        17,
        include_str!("../../migrations/0017_session_resume_on_launch.sql"),
        false,
    ),
    (
        18,
        include_str!("../../migrations/0018_slot_model_override.sql"),
        false,
    ),
    (
        19,
        include_str!("../../migrations/0019_session_agent_options.sql"),
        false,
    ),
    (
        20,
        include_str!("../../migrations/0020_slot_effort_override.sql"),
        false,
    ),
    (
        21,
        include_str!("../../migrations/0021_session_attention.sql"),
        false,
    ),
    (
        22,
        include_str!("../../migrations/0022_session_live_title.sql"),
        false,
    ),
    (23, include_str!("../../migrations/0023_roles.sql"), false),
    (
        24,
        include_str!("../../migrations/0024_codex_speed.sql"),
        false,
    ),
    (
        25,
        include_str!("../../migrations/0025_drop_crew_purpose_goal.sql"),
        false,
    ),
    (
        26,
        include_str!("../../migrations/0026_session_launch_definition.sql"),
        false,
    ),
    (
        27,
        include_str!("../../migrations/0027_drop_unused_session_columns.sql"),
        false,
    ),
    (
        28,
        include_str!("../../migrations/0028_optional_mission_crew_restrict_deletes.sql"),
        true,
    ),
    (
        29,
        include_str!("../../migrations/0029_role_avatar_path.sql"),
        false,
    ),
    (
        30,
        include_str!("../../migrations/0030_role_avatar_style.sql"),
        false,
    ),
    (
        31,
        include_str!("../../migrations/0031_role_avatar_color.sql"),
        false,
    ),
];

pub(super) fn run_migrations(conn: &mut Connection) -> Result<()> {
    run_migrations_up_to(conn, i64::MAX)
}

/// Apply pending migrations up to and including `max_version`. Only the
/// migration tests cap this — production always runs the full set.
pub(super) fn run_migrations_up_to(conn: &mut Connection, max_version: i64) -> Result<()> {
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
    for (version, sql, foreign_keys_off) in MIGRATIONS {
        if *version > current && *version <= max_version {
            apply_migration(conn, *version, sql, *foreign_keys_off)?;
        }
    }
    Ok(())
}

pub(super) fn apply_migration(
    conn: &mut Connection,
    version: i64,
    sql: &str,
    foreign_keys_off: bool,
) -> Result<()> {
    if foreign_keys_off {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let result = (|| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        if version == 14 {
            backfill_0014_nodes(&tx)?;
        }
        if version == 15 {
            backfill_0015_retire_folders(&tx)?;
        }
        if version == 26 {
            backfill_0026_launch_definitions(&tx)?;
        }
        if foreign_keys_off {
            let mut check = tx.prepare("PRAGMA foreign_key_check")?;
            let mut rows = check.query([])?;
            if let Some(row) = rows.next()? {
                let table: String = row.get(0)?;
                let parent: String = row.get(2)?;
                return Err(crate::error::Error::msg(format!(
                    "migration {version}: foreign key violation in {table} referencing {parent}"
                )));
            }
        }
        tx.execute(
            "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
            params![version, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    })();
    // The transaction has committed or rolled back before changing the pragma.
    if foreign_keys_off {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    result
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

fn backfill_0026_launch_definitions(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    #[derive(serde::Deserialize)]
    struct LegacySession {
        id: String,
        role_id: Option<String>,
        agent_runtime: Option<String>,
        agent_command: Option<String>,
        agent_model: Option<String>,
        agent_effort: Option<String>,
        #[serde(
            rename = "runtime_options_json",
            with = "crate::repo::serde::runtime_speed_json"
        )]
        speed: Option<crate::model::CodexSpeed>,
    }
    let rows: Vec<LegacySession> = tx.prepare(
        "SELECT id, role_id, agent_runtime, agent_command, agent_model, agent_effort, runtime_options_json FROM sessions",
    )?.query_map([], |row| serde_rusqlite::from_row(row).map_err(crate::repo::de_err))?
        .collect::<rusqlite::Result<_>>()?;
    for row in rows {
        let role = row
            .role_id
            .as_deref()
            .map(|id| crate::repo::role::get(tx, id))
            .transpose()?
            .flatten();
        if row.role_id.is_some() && role.is_none() {
            log::warn!(
                "session {} launch definition conversion: missing role",
                row.id
            );
            continue;
        }
        match crate::session::launch_definition::convert_legacy(
            role.as_ref(),
            row.agent_runtime.as_deref(),
            row.agent_command.as_deref(),
            row.agent_model.as_deref(),
            row.agent_effort.as_deref(),
            row.speed,
        ) {
            Ok((runtime, definition)) => {
                crate::repo::session::set_launch(tx, &row.id, &runtime, &definition)?;
            }
            Err(error) => log::warn!("session {} launch definition conversion: {error}", row.id),
        }
    }
    Ok(())
}
