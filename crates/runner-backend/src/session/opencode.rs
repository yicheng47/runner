// OpenCode support (spec 592).
//
// OpenCode assigns its own `ses_…` ids, at the first message, and nothing it
// stores says which process created a session. So every OpenCode spawn loads a
// Runner-owned plugin through `OPENCODE_CONFIG_CONTENT`. The plugin runs inside
// that one process and reports each top-level session it creates — a first
// turn, a blank chat's first typed message, a fork, `/new` — to the shared
// session-key drop watcher (`claude_rekey`), which rekeys the Runner row. A
// plain resume creates nothing, so its key stands.
//
// OpenCode's SQLite store is read only to check that a session still exists
// before resuming it.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{OpenFlags, OptionalExtension};

use crate::error::Result;

pub(crate) const REKEY_PATH_ENV: &str = "RUNNER_OPENCODE_REKEY_PATH";
pub(crate) const CONFIG_CONTENT_ENV: &str = "OPENCODE_CONFIG_CONTENT";

const PLUGIN_DIR: &str = "opencode";
const PLUGIN_FILE: &str = "runner-session-key.js";

const PLUGIN_SOURCE: &str = r#"// Runner-owned OpenCode plugin: reports each top-level session this process
// creates so Runner can resume and fork it. It reports no status.
import fs from "node:fs";
import path from "node:path";

export const RunnerSessionKey = async () => {
  const rekeyPath = process.env.RUNNER_OPENCODE_REKEY_PATH || "";
  let sequence = 0;
  return {
    event: async ({ event }) => {
      if (!rekeyPath || event?.type !== "session.created") return;
      const info = event.properties?.info;
      if (!info?.id || info.parentID) return;
      try {
        fs.mkdirSync(path.dirname(rekeyPath), { recursive: true });
        sequence += 1;
        const temporary = `${rekeyPath}.${process.pid}.${sequence}.tmp`;
        fs.writeFileSync(temporary, JSON.stringify({ session_id: info.id }));
        fs.renameSync(temporary, rekeyPath);
      } catch {}
    },
  };
};
"#;

pub(crate) fn plugin_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PLUGIN_DIR).join(PLUGIN_FILE)
}

pub(crate) fn install_plugin(app_data_dir: &Path) -> Result<()> {
    let directory = app_data_dir.join(PLUGIN_DIR);
    fs::create_dir_all(&directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(PLUGIN_SOURCE.as_bytes())?;
    temporary
        .persist(plugin_path(app_data_dir))
        .map_err(|error| error.error)?;
    Ok(())
}

/// OpenCode's session id shape: `ses_` and ASCII alphanumerics.
pub(crate) fn is_session_id(key: &str) -> bool {
    key.strip_prefix("ses_").is_some_and(|rest| {
        (1..=64).contains(&rest.len()) && rest.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

/// Loads the plugin into one OpenCode spawn. The `OPENCODE_CONFIG_CONTENT`
/// it extends is the one the process would otherwise get: `env`'s (the role's)
/// when set, else `process`, Runner's own environment's value, which
/// portable-pty passes to every child. Nothing is injected when the plugin is
/// missing or that value is not a JSON object, so the user's config always wins.
pub(crate) fn apply_spawn_env(
    env: &mut BTreeMap<String, String>,
    app_data_dir: &Path,
    runner_session_id: &str,
    process: Option<&str>,
) {
    let plugin = plugin_path(app_data_dir);
    if !plugin.is_file() {
        return;
    }
    let inherited = env.get(CONFIG_CONTENT_ENV).map(String::as_str).or(process);
    let Some(content) = config_content(inherited, &plugin.to_string_lossy()) else {
        log::warn!(
            "{CONFIG_CONTENT_ENV} is not a JSON object with a plugin list; OpenCode session {runner_session_id} will not report its key"
        );
        return;
    };
    env.insert(CONFIG_CONTENT_ENV.into(), content);
    env.insert(
        REKEY_PATH_ENV.into(),
        super::hook_feed::hook_path(&super::claude_rekey::drop_path(
            app_data_dir,
            runner_session_id,
        )),
    );
}

fn config_content(inherited: Option<&str>, plugin: &str) -> Option<String> {
    let mut config = match inherited.map(str::trim).filter(|value| !value.is_empty()) {
        None => serde_json::Map::new(),
        Some(value) => match crate::runtime_defaults::jsonc_document(value).ok()? {
            serde_json::Value::Object(config) => config,
            _ => return None,
        },
    };
    let plugins = config
        .entry("plugin")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()?;
    if !plugins.iter().any(|entry| entry.as_str() == Some(plugin)) {
        plugins.push(plugin.into());
    }
    Some(serde_json::Value::Object(config).to_string())
}

/// OpenCode's `Database.path()` for the release channels: `OPENCODE_DB`
/// (absolute, or relative to the data dir), else `<data>/opencode.db`, where
/// `<data>` is `$XDG_DATA_HOME/opencode` or `~/.local/share/opencode`. Each
/// variable comes from the role's env first, then Runner's own. `None` for an
/// in-memory database, which cannot be checked.
pub(crate) fn database_path(role_env: &HashMap<String, String>, home: &Path) -> Option<PathBuf> {
    let var = |name: &str| {
        role_env
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .filter(|value| !value.is_empty())
    };
    let data = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("opencode");
    match var("OPENCODE_DB") {
        Some(db) if db == ":memory:" => None,
        Some(db) => Some(data.join(db)),
        None => Some(data.join("opencode.db")),
    }
}

/// Whether OpenCode's store still holds `key`. A missing database or row is
/// `false`; any other failure is `true`, so a locked or unreadable database
/// never throws away a resumable conversation (OpenCode's own "Session not
/// found" is the backstop).
pub(crate) fn session_exists(db: &Path, key: &str) -> bool {
    if !db.is_file() {
        return false;
    }
    match query_session(db, key) {
        Ok(found) => found,
        Err(error) => {
            log::warn!("check OpenCode session {key} in {}: {error}", db.display());
            true
        }
    }
}

fn query_session(db: &Path, key: &str) -> rusqlite::Result<bool> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    // A read-only connection cannot create a WAL database's side files, and
    // SQLite deletes them only after checkpointing everything into the main
    // file, so with both absent the main file alone is complete.
    let side_files = ["-wal", "-shm"].iter().any(|suffix| {
        let mut path = db.as_os_str().to_owned();
        path.push(suffix);
        Path::new(&path).exists()
    });
    let conn = if side_files {
        rusqlite::Connection::open_with_flags(db, flags)?
    } else {
        rusqlite::Connection::open_with_flags(
            immutable_uri(db),
            flags | OpenFlags::SQLITE_OPEN_URI,
        )?
    };
    conn.busy_timeout(std::time::Duration::from_millis(250))?;
    conn.query_row("SELECT 1 FROM session WHERE id = ?1", [key], |_| Ok(()))
        .optional()
        .map(|row| row.is_some())
}

fn immutable_uri(db: &Path) -> String {
    let path = db.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file:");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?immutable=1");
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn session_ids_have_opencodes_shape() {
        assert!(is_session_id("ses_f31048251ffepsv6qvMgfycvy1"));
        for key in [
            "ses_",
            "ses_bad-id",
            "ses_with space",
            "f31048251ffepsv6qvMgfycvy1",
            "019fa1b9-a133-7841-b4dd-730d376ab1d1",
            "SES_f31048251ffepsv6qvMgfycvy1",
        ] {
            assert!(!is_session_id(key), "{key}");
        }
        assert!(!is_session_id(&format!("ses_{}", "a".repeat(65))));
    }

    #[test]
    fn config_content_appends_the_plugin_and_keeps_the_users_config_in_order() {
        let plugin = "/app data/opencode/runner-session-key.js";
        assert_eq!(
            config_content(None, plugin).unwrap(),
            r#"{"plugin":["/app data/opencode/runner-session-key.js"]}"#
        );
        assert_eq!(
            config_content(Some("  "), plugin),
            config_content(None, plugin)
        );
        // OpenCode's permission rules are last-match-wins, so key order must survive.
        let user = r#"{
  // mine
  "permission": {"edit": "ask", "*": "allow"},
  "plugin": ["my-plugin",],
}"#;
        assert_eq!(
            config_content(Some(user), plugin).unwrap(),
            r#"{"permission":{"edit":"ask","*":"allow"},"plugin":["my-plugin","/app data/opencode/runner-session-key.js"]}"#
        );
        let twice = config_content(Some(&config_content(None, plugin).unwrap()), plugin).unwrap();
        assert_eq!(twice, config_content(None, plugin).unwrap());
        for unusable in [
            "not json",
            "[1, 2]",
            r#""string""#,
            r#"{"plugin": "one-plugin"}"#,
        ] {
            assert_eq!(config_content(Some(unusable), plugin), None, "{unusable}");
        }
    }

    #[test]
    fn spawn_env_needs_the_installed_plugin_and_an_object_to_extend() {
        let root = tempfile::tempdir().unwrap();
        let mut env = BTreeMap::new();
        apply_spawn_env(&mut env, root.path(), "runner-session", None);
        assert!(env.is_empty(), "no plugin installed");

        install_plugin(root.path()).unwrap();
        assert_eq!(
            fs::read_to_string(plugin_path(root.path())).unwrap(),
            PLUGIN_SOURCE
        );
        apply_spawn_env(&mut env, root.path(), "runner-session", None);
        let plugin = plugin_path(root.path()).to_string_lossy().into_owned();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&env[CONFIG_CONTENT_ENV]).unwrap(),
            serde_json::json!({ "plugin": [plugin] })
        );
        assert_eq!(
            env[REKEY_PATH_ENV],
            super::super::hook_feed::hook_path(&super::super::claude_rekey::drop_path(
                root.path(),
                "runner-session"
            ))
        );

        let mut env = BTreeMap::new();
        apply_spawn_env(&mut env, root.path(), "runner-session", Some("[]"));
        assert!(env.is_empty(), "a non-object value is left to OpenCode");
    }

    #[test]
    fn spawn_env_extends_the_value_the_process_would_inherit() {
        let root = tempfile::tempdir().unwrap();
        install_plugin(root.path()).unwrap();
        let plugin = plugin_path(root.path()).to_string_lossy().into_owned();
        let role = r#"{"plugin": ["role-plugin"]}"#;
        let process = r#"{"model": "anthropic/claude-sonnet-4-5", "plugin": ["process-plugin"]}"#;
        let content = |role: Option<&str>, process: Option<&str>| {
            let mut env = BTreeMap::new();
            if let Some(role) = role {
                env.insert(CONFIG_CONTENT_ENV.to_string(), role.to_string());
            }
            apply_spawn_env(&mut env, root.path(), "runner-session", process);
            serde_json::from_str::<serde_json::Value>(&env[CONFIG_CONTENT_ENV]).unwrap()
        };
        assert_eq!(
            content(Some(role), None),
            serde_json::json!({ "plugin": ["role-plugin", plugin] })
        );
        assert_eq!(
            content(None, Some(process)),
            serde_json::json!({
                "model": "anthropic/claude-sonnet-4-5",
                "plugin": ["process-plugin", plugin],
            })
        );
        assert_eq!(
            content(Some(role), Some(process)),
            serde_json::json!({ "plugin": ["role-plugin", plugin] }),
            "the role's value is the one the child gets"
        );
        assert_eq!(
            content(None, None),
            serde_json::json!({ "plugin": [plugin] })
        );

        let mut env = BTreeMap::new();
        apply_spawn_env(&mut env, root.path(), "runner-session", Some("not json"));
        assert!(
            env.is_empty(),
            "an inherited value Runner cannot extend is left to OpenCode"
        );
    }

    #[test]
    fn database_path_follows_opencodes_resolution() {
        let home = Path::new("/home/me");
        let env = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect()
        };
        // Runner's own environment may set these; the role's env wins either way.
        let data = env(&[("XDG_DATA_HOME", "/data")]);
        assert_eq!(
            database_path(&data, home).unwrap(),
            Path::new("/data/opencode/opencode.db")
        );
        assert_eq!(
            database_path(
                &env(&[("XDG_DATA_HOME", "/data"), ("OPENCODE_DB", "/db/x.db")]),
                home
            )
            .unwrap(),
            Path::new("/db/x.db")
        );
        assert_eq!(
            database_path(
                &env(&[("XDG_DATA_HOME", "/data"), ("OPENCODE_DB", "alt.db")]),
                home
            )
            .unwrap(),
            Path::new("/data/opencode/alt.db")
        );
        assert_eq!(
            database_path(&env(&[("OPENCODE_DB", ":memory:")]), home),
            None
        );
    }

    fn opencode_db(path: &Path, wal: bool) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        if wal {
            conn.pragma_update(None, "journal_mode", "wal").unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE session (id text PRIMARY KEY, directory text NOT NULL, parent_id text);
             INSERT INTO session VALUES ('ses_f31048251ffepsv6qvMgfycvy1', '/work', NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn session_exists_reads_live_and_closed_wal_databases() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("dir with space #1").join("opencode.db");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        assert!(!session_exists(&db, "ses_f31048251ffepsv6qvMgfycvy1"));

        let live = opencode_db(&db, true);
        live.execute(
            "INSERT INTO session VALUES ('ses_live0000000000000000000000', '/work', NULL)",
            [],
        )
        .unwrap();
        assert!(Path::new(&format!("{}-wal", db.display())).exists());
        assert!(session_exists(&db, "ses_live0000000000000000000000"));
        assert!(session_exists(&db, "ses_f31048251ffepsv6qvMgfycvy1"));
        assert!(!session_exists(&db, "ses_missing000000000000000000"));
        drop(live);

        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", db.display()));
        }
        assert!(session_exists(&db, "ses_live0000000000000000000000"));
        assert!(!session_exists(&db, "ses_missing000000000000000000"));
        assert!(!Path::new(&format!("{}-wal", db.display())).exists());

        let other = root.path().join("other.db");
        rusqlite::Connection::open(&other)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (id text)")
            .unwrap();
        assert!(
            session_exists(&other, "ses_f31048251ffepsv6qvMgfycvy1"),
            "an unreadable store never discards a conversation"
        );
    }

    #[test]
    fn plugin_reports_only_top_level_sessions_it_creates() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping OpenCode plugin test: node is not on PATH");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let module = root.path().join("runner-session-key.mjs");
        fs::write(&module, PLUGIN_SOURCE).unwrap();
        let driver = root.path().join("driver.mjs");
        fs::write(
            &driver,
            r#"import fs from "node:fs";
import { RunnerSessionKey } from "./runner-session-key.mjs";
const path = process.env.RUNNER_OPENCODE_REKEY_PATH;
const report = () => fs.existsSync(path) ? JSON.parse(fs.readFileSync(path, "utf8")).session_id : null;
const created = (id, parentID) => ({ event: { type: "session.created", properties: { info: { id, parentID } } } });
delete process.env.RUNNER_OPENCODE_REKEY_PATH;
await (await RunnerSessionKey({})).event(created("ses_unwired"));
if (fs.existsSync(process.env.DROP_DIR)) throw new Error("reported without a path");
process.env.RUNNER_OPENCODE_REKEY_PATH = path;
const hooks = await RunnerSessionKey({});
await hooks.event({ event: { type: "session.updated", properties: { info: { id: "ses_updated" } } } });
await hooks.event(created("ses_child", "ses_parent"));
await hooks.event({ event: {} });
if (report() !== null) throw new Error(`reported ${report()}`);
await hooks.event(created("ses_first"));
if (report() !== "ses_first") throw new Error(`first ${report()}`);
await hooks.event(created("ses_second"));
if (report() !== "ses_second") throw new Error(`second ${report()}`);
const leftovers = fs.readdirSync(process.env.DROP_DIR).filter((name) => name !== "runner-session.json");
if (leftovers.length) throw new Error(`leftover files ${leftovers}`);
"#,
        )
        .unwrap();
        let drop = root.path().join("session-keys").join("runner-session.json");
        let output = Command::new("node")
            .arg(&driver)
            .env(REKEY_PATH_ENV, &drop)
            .env("DROP_DIR", drop.parent().unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
    }
}
