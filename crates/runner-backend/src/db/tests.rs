use std::collections::BTreeMap;

use rusqlite::{params, Connection, ErrorCode};

use super::app_state::{
    login_shell_env_lkg, runtime_overrides, set_login_shell_env_lkg, set_runtime_override,
    LoginShellEnvLkg,
};
use super::migrations::{run_migrations, run_migrations_up_to, MIGRATIONS};
use super::seed::{
    seed_defaults, SEED_CODER_PROMPT, SEED_CREW_ADDENDUM, SEED_CREW_ID, SEED_REVIEWER_PROMPT,
    SEED_ROLE_ARGS_JSON,
};
use super::{open_in_memory, open_pool};

fn insert_crew(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
        params![id, format!("crew-{id}"), "2026-04-22T00:00:00Z"],
    )
    .unwrap();
}

fn insert_role(conn: &Connection, id: &str, handle: &str) -> rusqlite::Result<usize> {
    let timestamp = "2026-04-22T00:00:00Z".parse().unwrap();
    crate::repo::role::insert(
        conn,
        &crate::repo::role::RoleRow {
            id: id.into(),
            handle: handle.into(),
            display_name: format!("{handle} display"),
            runtime: "shell".into(),
            command: "sh".into(),
            args_json: Some(Vec::new()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(Default::default()),
            model: None,
            effort: None,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )
    .map(|()| 1)
}

fn insert_slot(
    conn: &Connection,
    id: &str,
    crew_id: &str,
    role_id: &str,
    slot_handle: &str,
    position: i64,
    lead: i64,
) -> rusqlite::Result<usize> {
    crate::repo::slot::insert(
        conn,
        &crate::repo::slot::SlotRow {
            id: id.into(),
            crew_id: crew_id.into(),
            role_id: role_id.into(),
            slot_handle: slot_handle.into(),
            position,
            lead: lead != 0,
            runtime_override: None,
            model_override: None,
            effort_override: None,
            added_at: "2026-04-22T00:00:00Z".parse().unwrap(),
        },
    )
    .map(|()| 1)
}

fn schema_has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
        [name],
        |row| row.get(0),
    )
    .unwrap()
}

fn seed_pre_0023_fixture(conn: &Connection) {
    conn.execute(
        "INSERT INTO runners (
                id, handle, display_name, runtime, command, created_at, updated_at
             ) VALUES ('r1', 'reviewer', 'Reviewer', 'shell', 'sh',
                       '2026-09-16T00:00:00Z', '2026-09-16T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-09-16T00:00:00Z', '2026-09-16T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES
                ('sl1', 'c1', 'r1', 'lead', 0, 1, '2026-09-16T00:00:00Z'),
                ('sl2', 'c1', 'r1', 'reviewer', 1, 0, '2026-09-16T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions
                (id, runner_id, status, started_at, title)
             VALUES ('direct1', 'r1', 'stopped', '2026-09-16T00:00:00Z', 'Codex')",
        [],
    )
    .unwrap();
}

struct Pre0023Role<'a> {
    id: &'a str,
    handle: &'a str,
    display_name: &'a str,
    runtime: &'a str,
    command: &'a str,
    system_prompt: Option<&'a str>,
}

fn insert_pre_0023_role(conn: &Connection, role: Pre0023Role<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO runners (
                id, handle, display_name, runtime, command, system_prompt,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            role.id,
            role.handle,
            role.display_name,
            role.runtime,
            role.command,
            role.system_prompt,
            "2026-04-22T00:00:00Z",
        ],
    )?;
    Ok(())
}

fn pre_0023_system_prompt(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT system_prompt FROM runners WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
}

fn apply_pre_0023_persona_rewrite(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(include_str!("../../migrations/0002_persona_only_seeds.sql"))
}

fn seed_pre_0023_runtime_rows(conn: &Connection, runtimes: &[&str]) -> rusqlite::Result<()> {
    for (index, runtime) in runtimes.iter().enumerate() {
        let id = index.to_string();
        conn.execute(
            "INSERT INTO runners (
                    id, handle, display_name, runtime, command, created_at, updated_at
                 ) VALUES (?1, ?1, ?1, ?2, 'sh', ?3, ?3)",
            params![id, runtime, "2026-04-22T00:00:00Z"],
        )?;
        conn.execute(
            "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead,
                     runtime_override, added_at)
                 VALUES (?1, 'c1', ?1, ?1, ?2, 0, ?3, ?4)",
            params![id, index as i64, runtime, "2026-04-22T00:00:00Z"],
        )?;
        conn.execute(
            "INSERT INTO sessions
                    (id, runner_id, status, agent_runtime, runtime)
                 VALUES (?1, ?1, 'stopped', ?2, 'native-pty')",
            params![id, runtime],
        )?;
    }
    Ok(())
}

fn insert_pre_0023_slot(
    conn: &Connection,
    id: &str,
    crew_id: &str,
    role_id: &str,
    slot_handle: &str,
    position: i64,
    lead: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '2026-04-22T00:00:00Z')",
        params![id, crew_id, role_id, slot_handle, position, lead],
    )?;
    Ok(())
}

#[test]
fn migrations_bootstrap_all_tables() {
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN
                     ('crews','slots','missions','sessions')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 4);
    assert!(crate::repo::role::table_exists(&conn).unwrap());
    assert!(!schema_has_table(&conn, "runners"));
}

#[test]
fn login_shell_lkg_round_trips_through_app_state() {
    let pool = open_in_memory().unwrap();
    let snapshot = LoginShellEnvLkg {
        env: crate::shell_path::LoginShellEnv {
            path: Some("/custom/bin:/usr/bin".into()),
            vars: BTreeMap::from([("HTTPS_PROXY".into(), "http://proxy".into())]),
        },
        shell: "/bin/zsh".into(),
        captured_at: "2026-07-25T00:00:00Z".into(),
    };
    assert_eq!(login_shell_env_lkg(&pool).unwrap(), None);
    set_login_shell_env_lkg(&pool, &snapshot).unwrap();
    assert_eq!(login_shell_env_lkg(&pool).unwrap(), Some(snapshot));
}

#[test]
fn runtime_overrides_set_clear_as_one_json_value() {
    let pool = open_in_memory().unwrap();
    set_runtime_override(&pool, "codex", Some("/opt/codex")).unwrap();
    set_runtime_override(&pool, "claude-code", Some("/opt/claude")).unwrap();
    assert_eq!(
        runtime_overrides(&pool).unwrap(),
        BTreeMap::from([
            ("claude-code".into(), "/opt/claude".into()),
            ("codex".into(), "/opt/codex".into()),
        ])
    );
    set_runtime_override(&pool, "codex", None).unwrap();
    assert_eq!(
        runtime_overrides(&pool).unwrap(),
        BTreeMap::from([("claude-code".into(), "/opt/claude".into())])
    );
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
// commands; covered by the slot_set_lead test in ops::slot.
// The schema no longer has the partial unique index that used to
// enforce it.

#[test]
fn role_handle_is_globally_unique() {
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    insert_role(&conn, "r1", "shared").unwrap();
    let err = insert_role(&conn, "r2", "shared").unwrap_err();
    assert_eq!(
        err.sqlite_error_code(),
        Some(ErrorCode::ConstraintViolation)
    );
}

#[test]
fn same_role_can_join_multiple_crews() {
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    insert_crew(&conn, "c1");
    insert_crew(&conn, "c2");
    insert_role(&conn, "r1", "shared").unwrap();

    insert_slot(&conn, "s1", "c1", "r1", "alpha-c1", 0, 1).unwrap();
    insert_slot(&conn, "s2", "c2", "r1", "alpha-c2", 0, 1).unwrap();
}

#[test]
fn same_role_can_fill_multiple_slots_in_one_crew() {
    // The whole point of the slot redesign: the same role
    // template can sit in two slots of the same crew with
    // different in-crew handles.
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    insert_crew(&conn, "c1");
    insert_role(&conn, "r1", "claude").unwrap();
    insert_slot(&conn, "s1", "c1", "r1", "architect", 0, 1).unwrap();
    insert_slot(&conn, "s2", "c1", "r1", "reviewer", 1, 0).unwrap();
}

#[test]
fn slot_handle_is_unique_per_crew() {
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    insert_crew(&conn, "c1");
    insert_role(&conn, "r1", "alpha").unwrap();
    insert_role(&conn, "r2", "beta").unwrap();
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
    insert_role(&conn, "r1", "alpha").unwrap();
    insert_role(&conn, "r2", "beta").unwrap();

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
    let timestamp = "2026-04-22T00:00:00Z".parse().unwrap();
    crate::repo::role::insert(
        &conn,
        &crate::repo::role::RoleRow {
            id: "r1".into(),
            handle: "test-impl".into(),
            display_name: "Impl".into(),
            runtime: "shell".into(),
            command: "sh".into(),
            args_json: Some(serde_json::from_value(args.clone()).unwrap()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(serde_json::from_value(env.clone()).unwrap()),
            model: None,
            effort: None,
            created_at: timestamp,
            updated_at: timestamp,
        },
    )
    .unwrap();

    let role = crate::repo::role::get(&conn, "r1").unwrap().unwrap();
    assert_eq!(serde_json::to_value(role.args).unwrap(), args);
    assert_eq!(serde_json::to_value(role.env).unwrap(), env);
}

#[test]
fn deleting_crew_cascades_slot_rows_only() {
    // Runners are global templates — deleting a crew should strip
    // its slots but leave the role template intact so other
    // crews (or direct chats) can keep using it.
    let pool = open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    insert_crew(&conn, "c1");
    insert_role(&conn, "r1", "alpha").unwrap();
    insert_slot(&conn, "s1", "c1", "r1", "alpha", 0, 1).unwrap();

    conn.execute("DELETE FROM crews WHERE id = 'c1'", [])
        .unwrap();
    let role_count = i64::from(crate::repo::role::get(&conn, "r1").unwrap().is_some());
    let slot_count = crate::repo::slot::list_for_role_with_crew_name(&conn, "r1")
        .unwrap()
        .len();
    assert_eq!(role_count, 1, "runner template must survive crew delete");
    assert_eq!(slot_count, 0, "slots cascade with the crew");
}

#[test]
fn seed_defaults_inserts_pair_coding_crew_on_empty_db() {
    let pool = open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    seed_defaults(&mut conn).unwrap();

    let crew_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
        .unwrap();
    let role_count = crate::repo::role::count(&conn).unwrap();
    let slot_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(crew_count, 1);
    assert_eq!(role_count, 2);
    assert_eq!(slot_count, 2);

    let lead_handle: String = conn
        .query_row("SELECT slot_handle FROM slots WHERE lead = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(lead_handle, "coder");

    let (name, addendum): (String, Option<String>) = conn
        .query_row(
            "SELECT name, system_prompt_addendum FROM crews WHERE id = ?1",
            params![SEED_CREW_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(name, "Pair coding crew");
    assert_eq!(
        addendum.as_deref(),
        Some(SEED_CREW_ADDENDUM.trim_end_matches('\n'))
    );

    let expected_args: Vec<String> = serde_json::from_str(SEED_ROLE_ARGS_JSON).unwrap();
    let codex_seed_count = crate::repo::role::list(&conn)
        .unwrap()
        .into_iter()
        .filter(|role| {
            role.runtime == "codex"
                && role.command == "codex"
                && role.args == expected_args
                && role.model.is_none()
                && role.effort.is_none()
        })
        .count();
    assert_eq!(
        codex_seed_count, 2,
        "all seeded roles should use codex Auto with inherited model/effort",
    );

    for (handle, prompt) in [
        ("coder", SEED_CODER_PROMPT),
        ("reviewer", SEED_REVIEWER_PROMPT),
    ] {
        let stored = crate::repo::role::get_by_handle(&conn, handle)
            .unwrap()
            .unwrap()
            .system_prompt
            .unwrap();
        assert_eq!(stored, prompt.trim_end_matches('\n'));
    }
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
    let md = include_str!("../../../../tests/fixtures/system-prompts/architect.md");
    md.trim_end_matches('\n').to_string()
}

#[test]
fn migration_0002_persona_preserves_customized_system_prompts() {
    // Reviewer-codex flagged this on #51: the persona migration
    // must NOT clobber a user who edited their seeded
    // architect/impl/reviewer row in place (same id, customized
    // prompt). The WHERE pin on the pre-#51 seed text is what
    // makes the migration idempotent for customized rows.
    let mut conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&mut conn, 1).unwrap();
    let custom = "My customized architect prompt — please do not overwrite.";
    insert_pre_0023_role(
        &conn,
        Pre0023Role {
            id: "01K000DEFAULT000RUNNERARCH01",
            handle: "architect",
            display_name: "Custom A",
            runtime: "claude-code",
            command: "claude",
            system_prompt: Some(custom),
        },
    )
    .unwrap();
    apply_pre_0023_persona_rewrite(&conn).unwrap();
    let preserved = pre_0023_system_prompt(&conn, "01K000DEFAULT000RUNNERARCH01")
        .unwrap()
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
    let mut conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&mut conn, 1).unwrap();
    insert_pre_0023_role(
        &conn,
        Pre0023Role {
            id: "01K000DEFAULT000RUNNERARCH01",
            handle: "architect",
            display_name: "Architect",
            runtime: "claude-code",
            command: "claude",
            system_prompt: Some(PRE_51_ARCHITECT_SEED),
        },
    )
    .unwrap();
    apply_pre_0023_persona_rewrite(&conn).unwrap();
    let rewritten = pre_0023_system_prompt(&conn, "01K000DEFAULT000RUNNERARCH01")
        .unwrap()
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
    // The seed reads the copyable pair-coding example via
    // `include_str!`, so checking these sources checks the stored
    // role prompts too. Mission verbs belong in the crew addendum.
    let banned_substrings = [
        "runner msg post",
        "runner msg read",
        "runner status idle",
        "ask_lead",
        "ask_human",
    ];
    for (name, md) in [
        ("coder.md", SEED_CODER_PROMPT),
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
fn seed_defaults_skips_when_user_has_a_role_but_no_crew() {
    // Any existing role means this is not a first-launch-empty DB.
    // The seed must bail as one unit instead of colliding on a handle
    // and leaving a partial crew without its lead.
    let pool = open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    insert_role(&conn, "user-r1", "coder").unwrap();
    seed_defaults(&mut conn).unwrap();

    let crew_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM crews", [], |r| r.get(0))
        .unwrap();
    let role_count = crate::repo::role::count(&conn).unwrap();
    assert_eq!(crew_count, 0, "should not create Pair coding crew");
    assert_eq!(role_count, 1, "user's runner stays untouched");
}

#[test]
fn seed_defaults_marker_prevents_reseed_after_user_deletes_everything() {
    // First launch: empty DB → seed runs and marker is recorded.
    let pool = open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    seed_defaults(&mut conn).unwrap();

    // User wipes the seeded data — slots cascade with the crew,
    // Roles are global templates so we delete them explicitly.
    conn.execute("DELETE FROM crews", []).unwrap();
    crate::repo::role::delete_all(&conn).unwrap();
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

    let role_count = crate::repo::role::count(&conn).unwrap();
    let slot_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM slots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(role_count, 2);
    assert_eq!(slot_count, 2);
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
fn migrations_0022_and_0023_preserve_data_and_role_cascades() {
    {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations_up_to(&mut conn, 21).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, title) VALUES ('chat', 'stopped', 'Codex')",
            [],
        )
        .unwrap();
        run_migrations_up_to(&mut conn, 22).unwrap();
        let names: (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT title, live_title FROM sessions WHERE id = 'chat'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(names, (Some("Codex".into()), None));
    }

    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    run_migrations_up_to(&mut conn, 22).unwrap();
    seed_pre_0023_fixture(&conn);
    run_migrations(&mut conn).unwrap();

    assert!(crate::repo::role::table_exists(&conn).unwrap());
    assert!(!schema_has_table(&conn, "runners"));
    let role = crate::repo::role::get(&conn, "r1").unwrap().unwrap();
    assert_eq!(role.id, "r1");
    assert_eq!(role.handle, "reviewer");
    assert_eq!(role.display_name, "Reviewer");
    assert_eq!(role.runtime, "shell");
    assert_eq!(role.command, "sh");

    let crew = crate::repo::crew::get(&conn, "c1").unwrap().unwrap();
    assert_eq!(crew.id, "c1");
    assert_eq!(crew.name, "Crew");

    let slots = crate::repo::slot::list_for_crew(&conn, "c1").unwrap();
    assert_eq!(
        slots
            .iter()
            .map(|slot| (
                slot.id.as_str(),
                slot.role_id.as_str(),
                slot.slot_handle.as_str(),
                slot.position,
                slot.lead,
            ))
            .collect::<Vec<_>>(),
        [
            ("sl1", "r1", "lead", 0, true),
            ("sl2", "r1", "reviewer", 1, false),
        ]
    );

    let session = crate::repo::session::get_row(&conn, "direct1")
        .unwrap()
        .unwrap();
    assert_eq!(session.id, "direct1");
    assert_eq!(session.role_id.as_deref(), Some("r1"));
    assert_eq!(session.status, crate::model::SessionStatus::Stopped);
    assert_eq!(session.title.as_deref(), Some("Codex"));
    assert_eq!(session.live_title, None);

    let violation_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violation_count, 0);

    assert_eq!(crate::repo::role::delete(&conn, "r1").unwrap(), 1);
    assert!(crate::repo::slot::list_for_crew(&conn, "c1")
        .unwrap()
        .is_empty());
    assert!(crate::repo::session::get_row(&conn, "direct1")
        .unwrap()
        .is_none());
    assert!(crate::repo::crew::get(&conn, "c1").unwrap().is_some());
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
fn migration_0020_adds_nullable_slot_effort_override() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    run_migrations_up_to(&mut conn, 19).unwrap();
    insert_crew(&conn, "c1");
    insert_pre_0023_role(
        &conn,
        Pre0023Role {
            id: "r1",
            handle: "alpha",
            display_name: "alpha display",
            runtime: "shell",
            command: "sh",
            system_prompt: None,
        },
    )
    .unwrap();
    insert_pre_0023_slot(&conn, "s1", "c1", "r1", "alpha", 0, true).unwrap();

    let columns_before: Vec<String> = conn
        .prepare("PRAGMA table_info(slots)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>("name"))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(!columns_before.iter().any(|c| c == "effort_override"));

    run_migrations(&mut conn).unwrap();
    let inherited: Option<String> = conn
        .query_row(
            "SELECT effort_override FROM slots WHERE id = 's1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(inherited, None);

    conn.execute(
        "UPDATE slots SET effort_override = 'xhigh' WHERE id = 's1'",
        [],
    )
    .unwrap();
    let overridden: Option<String> = conn
        .query_row(
            "SELECT effort_override FROM slots WHERE id = 's1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(overridden.as_deref(), Some("xhigh"));
}

#[test]
fn session_attention_migrates_existing_unread_tabs_without_consuming_siblings() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    run_migrations_up_to(&mut conn, 20).unwrap();
    conn.execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('a', 'stopped', 'codex'), ('b', 'stopped', 'codex')", []).unwrap();
    let node =
        crate::repo::node::create_tab(&conn, None, "chat", 0, r#"{"slots":["a","b"]}"#).unwrap();
    conn.execute(
        "UPDATE nodes SET last_completed_at = '2026-09-14T00:00:00Z' WHERE id = ?1",
        [&node.id],
    )
    .unwrap();
    run_migrations(&mut conn).unwrap();
    crate::repo::session_attention::mark_viewed(&conn, "b", "2026-09-14T00:01:00Z").unwrap();
    let unread: Vec<(String, Option<i64>)> = conn
        .prepare("SELECT session_id, unread_since FROM session_attention ORDER BY session_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(unread[0].1.is_some());
    assert_eq!(unread[1].1, None);
}

#[test]
fn stored_runtime_names_remain_readable_and_unchanged_without_a_migration() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    run_migrations_up_to(&mut conn, 20).unwrap();
    insert_crew(&conn, "c1");
    seed_pre_0023_runtime_rows(&conn, &["qoder", "Runtime-Needle", "shell"]).unwrap();

    run_migrations(&mut conn).unwrap();
    for (index, runtime) in ["qoder", "Runtime-Needle", "shell"].into_iter().enumerate() {
        let id = index.to_string();
        let role = crate::repo::role::get(&conn, &id).unwrap().unwrap();
        assert_eq!(role.runtime, runtime);
        crate::repo::role::update(&conn, &crate::repo::role::RoleRow::from(&role)).unwrap();
        let updated = crate::ops::role::update(
            &conn,
            &id,
            crate::ops::role::UpdateRoleInput {
                display_name: Some("Renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.runtime, runtime);
        let slot = crate::repo::slot::get(&conn, &id).unwrap().unwrap();
        assert_eq!(slot.runtime_override.as_deref(), Some(runtime));
        let session = crate::repo::session::get_row(&conn, &id).unwrap().unwrap();
        assert_eq!(session.agent_runtime.as_deref(), Some(runtime));
        assert_eq!(session.runtime.as_deref(), Some("native-pty"));
        assert_eq!(
            crate::repo::session::effective_runtime(&conn, &id)
                .unwrap()
                .as_deref(),
            Some(runtime)
        );
        assert_eq!(
            crate::repo::role::get(&conn, &id).unwrap().unwrap().runtime,
            runtime
        );
    }
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM _migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, MIGRATIONS.last().unwrap().0);
}

#[test]
fn sessions_has_runtime_size_and_resume_columns_after_migration() {
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
        "agent_model",
        "agent_effort",
        "last_cols",
        "last_rows",
        "resume_on_launch",
    ] {
        assert!(
            columns.iter().any(|c| c == required),
            "sessions.{required} missing; columns = {columns:?}"
        );
    }
}

#[test]
fn migration_0024_renames_only_the_untouched_default_crew() {
    fn crew_name_after_0024(seeded_name: &str) -> String {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations_up_to(&mut conn, 23).unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, ?2, '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z')",
            params![SEED_CREW_ID, seeded_name],
        )
        .unwrap();
        run_migrations_up_to(&mut conn, 24).unwrap();
        conn.query_row(
            "SELECT name FROM crews WHERE id = ?1",
            params![SEED_CREW_ID],
            |row| row.get(0),
        )
        .unwrap()
    }

    assert_eq!(
        crew_name_after_0024("Peer coding crew"),
        "Pair coding crew",
        "the untouched default must pick up the new name",
    );
    assert_eq!(
        crew_name_after_0024("My loop"),
        "My loop",
        "a crew the user renamed keeps their name",
    );
}
