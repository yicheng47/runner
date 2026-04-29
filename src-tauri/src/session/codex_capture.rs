// Codex post-spawn session-key capture.
//
// Codex's CLI doesn't accept a caller-provided session id at spawn time
// (claude-code does, via `--session-id <uuid>`), so we can't pre-assign
// the key the way the runtime adapter does for claude-code. Instead,
// codex writes a "rollout" file at
// `$HOME/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` whose
// first JSON line is a `session_meta` envelope containing `payload.id`
// (the session UUID) and `payload.cwd` (the working directory codex
// started in).
//
// After every codex spawn that didn't already have an
// `agent_session_key` on the row, we kick off a short-lived background
// thread that polls the rollout dirs for a file matching this spawn's
// cwd + start time. When found, we write `payload.id` into
// `sessions.agent_session_key`. The next `session_resume` for this row
// then drives `codex resume <uuid>` via the runtime adapter.
//
// The thread is bounded (30s timeout, 400ms poll interval) and best-
// effort: if the rollout never appears (codex crashed, $HOME differs,
// the user disabled rollouts), the row keeps a NULL key and codex
// continues to spawn fresh on every resume — same as today, no worse.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, Utc};
use rusqlite::params;

use crate::db::DbPool;

const CAPTURE_TIMEOUT_SECS: u64 = 30;
const POLL_INTERVAL_MS: u64 = 400;
/// Slack between the row's `started_at` and the rollout's
/// `payload.timestamp`. Codex stamps `payload.timestamp` slightly
/// AFTER our spawn returns, so under normal conditions the rollout
/// is always >= our `started_at`. We allow a small negative window
/// (1s) to absorb minor clock skew between the spawn-time
/// `Utc::now()` call and codex's. A wider window risks two chats
/// started in the same cwd within seconds of each other capturing
/// each other's id — see `claimed_rollouts` below.
const TIMESTAMP_SLACK_MS: i64 = 1000;

/// Process-shared set of rollout paths that some watcher has already
/// captured. Two concurrent watchers polling the same date dir might
/// both see the same rollout file (sibling chats started seconds
/// apart in the same cwd); without this set the first-match-wins
/// loop would write the same `payload.id` into both sessions, fusing
/// their codex conversations. Inserting here is the
/// fastest-thread-wins claim — losing watchers fall through to the
/// next file.
fn claimed_rollouts() -> &'static Mutex<HashSet<PathBuf>> {
    static SET: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Spawn a background thread that captures the codex session id for
/// `session_id` (a Runner sessions row) and writes it into
/// `agent_session_key`. Returns immediately; no-op if `$HOME/.codex/`
/// doesn't exist.
pub fn spawn_capture(
    session_id: String,
    spawn_cwd: String,
    started_at: DateTime<Utc>,
    pool: Arc<DbPool>,
) {
    std::thread::spawn(move || run(session_id, spawn_cwd, started_at, pool));
}

fn run(session_id: String, spawn_cwd: String, started_at: DateTime<Utc>, pool: Arc<DbPool>) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let sessions_root = PathBuf::from(home).join(".codex").join("sessions");
    if !sessions_root.is_dir() {
        return;
    }

    // Rollout dirs are partitioned by *local* date. The user might
    // start codex right before midnight and the rollout could land in
    // the next day's dir, so probe both.
    let local_started = started_at.with_timezone(&Local);
    let candidates = [local_started, local_started + chrono::Duration::days(1)];

    let deadline = Instant::now() + Duration::from_secs(CAPTURE_TIMEOUT_SECS);
    // `done` holds paths we've definitively decided about — either
    // they matched (we returned) or they're verifiably not ours
    // (cwd/timestamp mismatch, not a session_meta envelope). Files
    // that returned a transient verdict (empty / mid-write JSON) are
    // intentionally NOT inserted, so the next poll re-reads them.
    let mut done: HashSet<PathBuf> = HashSet::new();

    loop {
        for date in &candidates {
            let dir = sessions_root
                .join(format!("{:04}", date.year()))
                .join(format!("{:02}", date.month()))
                .join(format!("{:02}", date.day()));
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if done.contains(&path) {
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    done.insert(path);
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    done.insert(path);
                    continue;
                };
                if !name.starts_with("rollout-") {
                    done.insert(path);
                    continue;
                }
                match parse_session_meta(&path, &spawn_cwd, started_at) {
                    ParseVerdict::Match(id) => {
                        // Race guard: another watcher may have
                        // already captured this rollout (sibling chat
                        // in the same cwd). The first to insert into
                        // the process-shared set wins — losers fall
                        // through to the next file.
                        let claimed = claimed_rollouts().lock().unwrap().insert(path.clone());
                        if !claimed {
                            done.insert(path);
                            continue;
                        }
                        if let Ok(conn) = pool.get() {
                            // Guard with `agent_session_key IS NULL`
                            // so we don't clobber a key that a
                            // concurrent path (resume that already
                            // had a key) wrote first.
                            let _ = conn.execute(
                                "UPDATE sessions
                                    SET agent_session_key = ?2
                                  WHERE id = ?1
                                    AND agent_session_key IS NULL",
                                params![session_id, id],
                            );
                        }
                        return;
                    }
                    ParseVerdict::NotOurs => {
                        // Definitively not this spawn's rollout
                        // (cwd/timestamp mismatch, or a non-meta
                        // envelope). Skip it on every later poll.
                        done.insert(path);
                    }
                    ParseVerdict::NotReady => {
                        // File exists but its first line isn't yet
                        // parseable (codex created the file but
                        // hasn't flushed the session_meta line).
                        // Leave it out of `done` so the next poll
                        // re-reads it.
                    }
                }
            }
        }
        if Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

/// Outcome of inspecting one rollout file.
enum ParseVerdict {
    /// First line is a complete `session_meta` envelope whose cwd
    /// and timestamp match our spawn — capture this id.
    Match(String),
    /// First line parsed cleanly as JSON but is from a different
    /// codex run (cwd/timestamp mismatch, missing fields, or the
    /// envelope isn't `session_meta`). Won't ever match — skip on
    /// later polls.
    NotOurs,
    /// File can't be opened, is empty, or its first line isn't yet
    /// valid JSON (codex created the file but hasn't flushed the
    /// meta line). Try again on the next poll.
    NotReady,
}

fn parse_session_meta(
    path: &PathBuf,
    want_cwd: &str,
    started_after: DateTime<Utc>,
) -> ParseVerdict {
    // File opens often race the writer. Treat open errors as
    // transient so a future poll can retry.
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return ParseVerdict::NotReady,
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let read = match reader.read_line(&mut line) {
        Ok(n) => n,
        Err(_) => return ParseVerdict::NotReady,
    };
    if read == 0 || line.trim().is_empty() {
        return ParseVerdict::NotReady;
    }
    // A valid first line in codex's rollout always ends with `\n`.
    // If we read bytes but no newline, the writer is mid-flush —
    // retry.
    if !line.ends_with('\n') {
        return ParseVerdict::NotReady;
    }
    let parsed: serde_json::Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(_) => return ParseVerdict::NotReady,
    };
    if parsed.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return ParseVerdict::NotOurs;
    }
    let Some(payload) = parsed.get("payload") else {
        return ParseVerdict::NotOurs;
    };
    let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) else {
        return ParseVerdict::NotOurs;
    };
    if cwd != want_cwd {
        return ParseVerdict::NotOurs;
    }
    let Some(ts_str) = payload.get("timestamp").and_then(|v| v.as_str()) else {
        return ParseVerdict::NotOurs;
    };
    let Ok(ts) = ts_str.parse::<DateTime<Utc>>() else {
        return ParseVerdict::NotOurs;
    };
    if ts + chrono::Duration::milliseconds(TIMESTAMP_SLACK_MS) < started_after {
        // Older rollout sitting in the same dir.
        return ParseVerdict::NotOurs;
    }
    match payload.get("id").and_then(|v| v.as_str()) {
        Some(id) => ParseVerdict::Match(id.to_string()),
        None => ParseVerdict::NotOurs,
    }
}
