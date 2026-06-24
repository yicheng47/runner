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
// thread that first tries to identify the rollout file opened by the
// spawned pid. When no pid-owned rollout is available, a Runner-owned
// prompt marker can identify the rollout that belongs to this exact
// session row. If neither proof exists, the guarded fallback only
// accepts a single matching cwd + start-time rollout. Ambiguous
// matches fail closed and leave `sessions.agent_session_key` NULL.
// The next `session_resume` for this row then drives
// `codex resume <uuid>` via the runtime adapter when a key was safely
// captured.
//
// The thread is bounded (30s timeout, 400ms poll interval) and best-
// effort: if the rollout never appears (codex crashed, $HOME differs,
// the user disabled rollouts), the row keeps a NULL key and codex
// continues to spawn fresh on every resume — same as today, no worse.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, Utc};
use rusqlite::params;

use crate::{
    db::DbPool,
    session::manager::{SessionEvents, SessionUpdatedEvent},
};

const CAPTURE_TIMEOUT_SECS: u64 = 30;
const POLL_INTERVAL_MS: u64 = 400;
const PROMPT_MARKER_PREFIX: &str = "runner-codex-session-key-capture:";

pub fn prompt_marker(session_id: &str) -> String {
    format!("<!-- {PROMPT_MARKER_PREFIX}{session_id} -->")
}

/// Process-shared set of rollout paths that some watcher has already
/// captured. Two concurrent watchers polling the same date dir might
/// both see the same rollout file (sibling chats started seconds
/// apart in the same cwd); without this set the first-match-wins
/// loop could write the same `payload.id` into both sessions, fusing
/// their codex conversations. Inserting here is the
/// fastest-thread-wins claim; losing watchers keep polling and fail
/// closed if no unique unclaimed rollout remains.
fn claimed_rollouts() -> &'static Mutex<HashSet<PathBuf>> {
    static SET: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Spawn a background thread that captures the codex session id for
/// `session_id` (a Runner sessions row) and writes it into
/// `agent_session_key`. Returns immediately; no-op if `$HOME/.codex/`
/// doesn't exist.
pub struct CaptureRequest {
    pub session_id: String,
    pub mission_id: Option<String>,
    pub spawn_cwd: String,
    pub started_at: DateTime<Utc>,
    pub expected_row_started_at: String,
    pub spawn_pid: Option<i32>,
    pub prompt_marker: Option<String>,
    pub pool: Arc<DbPool>,
    pub events: Arc<dyn SessionEvents>,
}

pub fn spawn_capture(request: CaptureRequest) {
    std::thread::spawn(move || run(request));
}

fn run(request: CaptureRequest) {
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
    let local_started = request.started_at.with_timezone(&Local);
    let candidates = [local_started, local_started + chrono::Duration::days(1)];

    let deadline = Instant::now() + Duration::from_secs(CAPTURE_TIMEOUT_SECS);
    // `done` holds paths we've definitively decided about — either
    // they matched (we returned) or they're verifiably not ours
    // (cwd/timestamp mismatch, not a session_meta envelope). Files
    // that returned a transient verdict (empty / mid-write JSON) are
    // intentionally NOT inserted, so the next poll re-reads them.
    let mut done: HashSet<PathBuf> = HashSet::new();

    loop {
        let (scan, source) = if let Some(pid) = request.spawn_pid {
            scan_pid_result_or_fallback(
                open_rollout_paths_for_pid(pid, &sessions_root),
                &sessions_root,
                &candidates,
                &request.spawn_cwd,
                request.started_at,
                request.prompt_marker.as_deref(),
                &mut done,
            )
        } else {
            scan_fallback_with_marker(
                &sessions_root,
                &candidates,
                &request.spawn_cwd,
                request.started_at,
                request.prompt_marker.as_deref(),
                &mut done,
            )
        };

        match scan {
            CaptureScan::Unique(candidate) => {
                if source == ScanSource::Fallback
                    && !fallback_row_is_unambiguous(
                        &request.pool,
                        &request.session_id,
                        &request.expected_row_started_at,
                        &request.spawn_cwd,
                    )
                {
                    return;
                }
                // Race guard: another watcher may have already
                // captured this rollout (sibling chat in the same cwd).
                // The first to insert into the process-shared set wins.
                let claimed = claimed_rollouts()
                    .lock()
                    .unwrap()
                    .insert(candidate.path.clone());
                if !claimed {
                    done.insert(candidate.path);
                    continue;
                }
                if persist_capture(
                    &request.pool,
                    &request.session_id,
                    &request.expected_row_started_at,
                    &candidate.id,
                ) {
                    request.events.updated(&SessionUpdatedEvent {
                        session_id: request.session_id.clone(),
                        mission_id: request.mission_id.clone(),
                    });
                }
                return;
            }
            CaptureScan::Ambiguous => return,
            CaptureScan::None => {}
        }
        if Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

fn persist_capture(
    pool: &DbPool,
    session_id: &str,
    expected_row_started_at: &str,
    agent_session_key: &str,
) -> bool {
    let Ok(conn) = pool.get() else { return false };
    // Guard with `agent_session_key IS NULL` so we don't clobber a key
    // that a concurrent path (resume that already had a key) wrote
    // first. Guard with `started_at` so a stale watcher from a prior
    // incarnation of this row cannot write into a later stop/resume.
    conn.execute(
        "UPDATE sessions
            SET agent_session_key = ?2
          WHERE id = ?1
            AND agent_session_key IS NULL
            AND started_at = ?3",
        params![session_id, agent_session_key, expected_row_started_at],
    )
    .map(|updated| updated > 0)
    .unwrap_or(false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MatchCandidate {
    timestamp: DateTime<Utc>,
    path: PathBuf,
    id: String,
}

#[derive(Debug, PartialEq, Eq)]
enum CaptureScan {
    Unique(MatchCandidate),
    Ambiguous,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanSource {
    Pid,
    Marker,
    Fallback,
}

fn scan_pid_result_or_fallback(
    pid_paths: std::io::Result<Vec<PathBuf>>,
    sessions_root: &Path,
    candidates: &[DateTime<Local>],
    spawn_cwd: &str,
    started_at: DateTime<Utc>,
    prompt_marker: Option<&str>,
    done: &mut HashSet<PathBuf>,
) -> (CaptureScan, ScanSource) {
    match pid_paths {
        Ok(paths) if !paths.is_empty() => (
            scan_paths(paths, spawn_cwd, started_at, Some(done)),
            ScanSource::Pid,
        ),
        Ok(_) | Err(_) => scan_fallback_with_marker(
            sessions_root,
            candidates,
            spawn_cwd,
            started_at,
            prompt_marker,
            done,
        ),
    }
}

fn scan_fallback_with_marker(
    sessions_root: &Path,
    candidates: &[DateTime<Local>],
    spawn_cwd: &str,
    started_at: DateTime<Utc>,
    prompt_marker: Option<&str>,
    done: &mut HashSet<PathBuf>,
) -> (CaptureScan, ScanSource) {
    let paths = rollout_paths_for_dates(sessions_root, candidates);
    let Some(prompt_marker) = prompt_marker else {
        return (
            scan_paths(paths, spawn_cwd, started_at, Some(done)),
            ScanSource::Fallback,
        );
    };
    scan_paths_with_marker(paths, spawn_cwd, started_at, prompt_marker, Some(done))
}

fn fallback_row_is_unambiguous(
    pool: &DbPool,
    session_id: &str,
    expected_row_started_at: &str,
    spawn_cwd: &str,
) -> bool {
    let Ok(conn) = pool.get() else { return false };
    let current_cwd = conn.query_row(
        "SELECT s.cwd
           FROM sessions s
           LEFT JOIN runners r ON r.id = s.runner_id
          WHERE s.id = ?1
            AND s.started_at = ?2
            AND s.status = 'running'
            AND s.agent_session_key IS NULL
            AND COALESCE(s.agent_runtime, r.runtime) = 'codex'",
        params![session_id, expected_row_started_at],
        |r| r.get::<_, Option<String>>(0),
    );
    match current_cwd {
        Ok(Some(cwd)) if cwd != spawn_cwd => return false,
        Ok(_) => {}
        Err(_) => return false,
    }

    let sibling_count = conn.query_row(
        "SELECT COUNT(*)
           FROM sessions s
           LEFT JOIN runners r ON r.id = s.runner_id
          WHERE s.status = 'running'
            AND s.agent_session_key IS NULL
            AND COALESCE(s.agent_runtime, r.runtime) = 'codex'
            AND s.id <> ?1
            AND (
                s.cwd = ?2
                OR s.cwd IS NULL
            )",
        params![session_id, spawn_cwd],
        |r| r.get::<_, i64>(0),
    );
    sibling_count.map(|count| count == 0).unwrap_or(false)
}

fn rollout_paths_for_dates(sessions_root: &Path, candidates: &[DateTime<Local>]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for date in candidates {
        let dir = sessions_root
            .join(format!("{:04}", date.year()))
            .join(format!("{:02}", date.month()))
            .join(format!("{:02}", date.day()));
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        paths.extend(entries.flatten().map(|entry| entry.path()));
    }
    paths
}

fn scan_paths(
    paths: impl IntoIterator<Item = PathBuf>,
    spawn_cwd: &str,
    started_at: DateTime<Utc>,
    done: Option<&mut HashSet<PathBuf>>,
) -> CaptureScan {
    let matches = matching_candidates(paths, spawn_cwd, started_at, done);
    let claimed = claimed_rollouts().lock().unwrap();
    select_unique_unclaimed(matches, |path| claimed.contains(path))
}

fn scan_paths_with_marker(
    paths: impl IntoIterator<Item = PathBuf>,
    spawn_cwd: &str,
    started_at: DateTime<Utc>,
    prompt_marker: &str,
    done: Option<&mut HashSet<PathBuf>>,
) -> (CaptureScan, ScanSource) {
    let matches = matching_candidates(paths, spawn_cwd, started_at, done);
    let claimed = claimed_rollouts().lock().unwrap();
    let unclaimed: Vec<MatchCandidate> = matches
        .into_iter()
        .filter(|candidate| !claimed.contains(&candidate.path))
        .collect();
    drop(claimed);

    let marker_matches: Vec<MatchCandidate> = unclaimed
        .iter()
        .filter(|candidate| rollout_contains_marker(&candidate.path, prompt_marker))
        .cloned()
        .collect();
    if !marker_matches.is_empty() {
        return (
            select_unique_unclaimed(marker_matches, |_| false),
            ScanSource::Marker,
        );
    }

    (CaptureScan::None, ScanSource::Marker)
}

fn matching_candidates(
    paths: impl IntoIterator<Item = PathBuf>,
    spawn_cwd: &str,
    started_at: DateTime<Utc>,
    mut done: Option<&mut HashSet<PathBuf>>,
) -> Vec<MatchCandidate> {
    let mut matches = Vec::new();
    for path in paths {
        if done.as_ref().is_some_and(|done| done.contains(&path)) {
            continue;
        }
        if !is_rollout_file(&path) {
            if let Some(done) = done.as_deref_mut() {
                done.insert(path);
            }
            continue;
        }
        match parse_session_meta(&path, spawn_cwd, started_at) {
            ParseVerdict::Match(meta) => {
                matches.push(MatchCandidate {
                    timestamp: meta.timestamp,
                    path,
                    id: meta.id,
                });
            }
            ParseVerdict::NotOurs => {
                // Definitively not this spawn's rollout
                // (cwd/timestamp mismatch, invalid id, or a non-meta
                // envelope). Skip it on every later poll.
                if let Some(done) = done.as_deref_mut() {
                    done.insert(path);
                }
            }
            ParseVerdict::NotReady => {
                // File exists but its first line isn't yet parseable
                // (codex created the file but hasn't flushed the
                // session_meta line). Leave it out of `done` so the
                // next poll re-reads it.
            }
        }
    }
    matches
}

fn rollout_contains_marker(path: &Path, marker: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file);
    reader
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains(marker))
}

fn is_rollout_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.starts_with("rollout-"))
}

fn select_unique_unclaimed(
    mut matches: Vec<MatchCandidate>,
    mut is_claimed: impl FnMut(&PathBuf) -> bool,
) -> CaptureScan {
    matches.retain(|candidate| !is_claimed(&candidate.path));
    matches.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.path.cmp(&b.path))
    });
    match matches.len() {
        0 => CaptureScan::None,
        1 => CaptureScan::Unique(matches.remove(0)),
        _ => CaptureScan::Ambiguous,
    }
}

#[cfg(target_os = "macos")]
fn open_rollout_paths_for_pid(pid: i32, sessions_root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let output = std::process::Command::new("lsof")
        .args(["-Fn", "-p"])
        .arg(pid.to_string())
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!("lsof failed for pid {pid}")));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut paths: Vec<PathBuf> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(PathBuf::from)
        .filter(|path| path.starts_with(sessions_root) && is_rollout_file(path))
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg(not(target_os = "macos"))]
fn open_rollout_paths_for_pid(pid: i32, sessions_root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let _ = (pid, sessions_root);
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "pid-assisted codex rollout capture is only implemented on macOS",
    ))
}

struct MatchedSessionMeta {
    id: String,
    timestamp: DateTime<Utc>,
}

/// Outcome of inspecting one rollout file.
enum ParseVerdict {
    /// First line is a complete `session_meta` envelope whose cwd
    /// and timestamp match our spawn — capture this id.
    Match(MatchedSessionMeta),
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

fn parse_session_meta(path: &Path, want_cwd: &str, started_after: DateTime<Utc>) -> ParseVerdict {
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
    if ts < started_after {
        // Older rollout sitting in the same dir.
        return ParseVerdict::NotOurs;
    }
    match payload.get("id").and_then(|v| v.as_str()) {
        Some(id) if uuid::Uuid::parse_str(id).is_ok() => ParseVerdict::Match(MatchedSessionMeta {
            id: id.to_string(),
            timestamp: ts,
        }),
        _ => ParseVerdict::NotOurs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::io::Write;

    fn write_meta(
        dir: &tempfile::TempDir,
        name: &str,
        id: &str,
        cwd: &str,
        timestamp: DateTime<Utc>,
    ) -> PathBuf {
        write_meta_at_dir(dir.path(), name, id, cwd, timestamp)
    }

    fn write_meta_at_dir(
        dir: &Path,
        name: &str,
        id: &str,
        cwd: &str,
        timestamp: DateTime<Utc>,
    ) -> PathBuf {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "session_meta",
                "payload": {
                    "id": id,
                    "cwd": cwd,
                    "timestamp": timestamp.to_rfc3339(),
                }
            })
        )
        .unwrap();
        path
    }

    fn append_user_message(path: &Path, message: &str) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": message,
                }
            })
        )
        .unwrap();
    }

    #[test]
    fn parse_session_meta_rejects_rollout_before_spawn_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let older = started_at - chrono::Duration::milliseconds(1);
        let path = write_meta(
            &dir,
            "rollout-before.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4247",
            "/repo",
            older,
        );

        assert!(matches!(
            parse_session_meta(&path, "/repo", started_at),
            ParseVerdict::NotOurs
        ));
    }

    #[test]
    fn parse_session_meta_returns_timestamp_for_matching_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let id = "019ee58f-fb81-7d53-ab71-06b471bb4247";
        let path = write_meta(&dir, "rollout-match.jsonl", id, "/repo", started_at);

        match parse_session_meta(&path, "/repo", started_at) {
            ParseVerdict::Match(meta) => {
                assert_eq!(meta.id, id);
                assert_eq!(meta.timestamp, started_at);
            }
            _ => panic!("expected matching session_meta"),
        }
    }

    #[test]
    fn parse_session_meta_rejects_non_uuid_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let path = write_meta(
            &dir,
            "rollout-invalid.jsonl",
            "not-a-uuid",
            "/repo",
            started_at,
        );

        assert!(matches!(
            parse_session_meta(&path, "/repo", started_at),
            ParseVerdict::NotOurs
        ));
    }

    fn candidate(timestamp: DateTime<Utc>, path: &str, id: &str) -> MatchCandidate {
        MatchCandidate {
            timestamp,
            path: PathBuf::from(path),
            id: id.to_string(),
        }
    }

    #[test]
    fn select_unique_unclaimed_accepts_single_match() {
        let t0 = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let only = candidate(t0, "rollout-only.jsonl", "id-a");

        let got = select_unique_unclaimed(vec![only.clone()], |_| false);
        assert_eq!(got, CaptureScan::Unique(only));
    }

    #[test]
    fn select_unique_unclaimed_fails_closed_when_multiple_matches_remain() {
        let t0 = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let early = candidate(t0, "rollout-early.jsonl", "id-a");
        let late = candidate(
            t0 + chrono::Duration::seconds(2),
            "rollout-late.jsonl",
            "id-b",
        );

        let got = select_unique_unclaimed(vec![late, early], |_| false);
        assert_eq!(got, CaptureScan::Ambiguous);
    }

    #[test]
    fn select_unique_unclaimed_skips_claimed_paths() {
        let t0 = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let claimed = candidate(t0, "rollout-claimed.jsonl", "id-a");
        let unclaimed = candidate(
            t0 + chrono::Duration::seconds(2),
            "rollout-unclaimed.jsonl",
            "id-b",
        );

        let got = select_unique_unclaimed(vec![claimed.clone(), unclaimed.clone()], |path| {
            path == &claimed.path
        });
        assert_eq!(got, CaptureScan::Unique(unclaimed.clone()));

        let got = select_unique_unclaimed(vec![claimed.clone()], |path| path == &claimed.path);
        assert_eq!(got, CaptureScan::None);
    }

    #[test]
    fn scan_paths_accepts_single_matching_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let id = "019ee58f-fb81-7d53-ab71-06b471bb4247";
        let path = write_meta(&dir, "rollout-one.jsonl", id, "/repo", started_at);

        match scan_paths(vec![path.clone()], "/repo", started_at, None) {
            CaptureScan::Unique(candidate) => {
                assert_eq!(candidate.path, path);
                assert_eq!(candidate.id, id);
            }
            other => panic!("expected unique capture, got {other:?}"),
        }
    }

    #[test]
    fn scan_paths_fails_closed_for_multiple_matching_rollouts() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let a = write_meta(
            &dir,
            "rollout-a.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4247",
            "/repo",
            started_at,
        );
        let b = write_meta(
            &dir,
            "rollout-b.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4248",
            "/repo",
            started_at + chrono::Duration::milliseconds(1),
        );

        let got = scan_paths(vec![a, b], "/repo", started_at, None);
        assert_eq!(got, CaptureScan::Ambiguous);
    }

    #[test]
    fn scan_paths_with_marker_disambiguates_matching_rollouts() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let marker = prompt_marker("01KVWZN1KWNGC54E4PDRKPVJSG");
        let a = write_meta(
            &dir,
            "rollout-a.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4247",
            "/repo",
            started_at,
        );
        let b = write_meta(
            &dir,
            "rollout-b.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4248",
            "/repo",
            started_at + chrono::Duration::milliseconds(1),
        );
        append_user_message(&b, &format!("first turn\n\n{marker}"));

        let got = scan_paths_with_marker(vec![a, b.clone()], "/repo", started_at, &marker, None);

        assert_eq!(
            got,
            (
                CaptureScan::Unique(MatchCandidate {
                    timestamp: started_at + chrono::Duration::milliseconds(1),
                    path: b,
                    id: "019ee58f-fb81-7d53-ab71-06b471bb4248".to_string(),
                }),
                ScanSource::Marker,
            )
        );
    }

    #[test]
    fn scan_paths_with_marker_waits_when_siblings_lack_marker() {
        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let marker = prompt_marker("01KVWZN1KWNGC54E4PDRKPVJSG");
        let a = write_meta(
            &dir,
            "rollout-a.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4247",
            "/repo",
            started_at,
        );
        let b = write_meta(
            &dir,
            "rollout-b.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4248",
            "/repo",
            started_at + chrono::Duration::milliseconds(1),
        );

        let got = scan_paths_with_marker(vec![a, b], "/repo", started_at, &marker, None);

        assert_eq!(got, (CaptureScan::None, ScanSource::Marker));
    }

    #[test]
    fn marker_scan_waits_on_single_candidate_when_fallback_owner_is_ambiguous() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = ulid::Ulid::new().to_string();
        let current_id = ulid::Ulid::new().to_string();
        let sibling_id = ulid::Ulid::new().to_string();
        let row_started_at = "2026-06-20T12:00:00+00:00";
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES (?1, 'codex-capture', 'Codex Capture', 'codex', 'codex', ?2, ?2)",
            params![runner_id, row_started_at],
        )
        .unwrap();
        for session_id in [&current_id, &sibling_id] {
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at, agent_session_key)
                 VALUES (?1, NULL, ?2, '/repo', 'running', ?3, NULL)",
                params![session_id, runner_id, row_started_at],
            )
            .unwrap();
        }
        drop(conn);

        let dir = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let marker = prompt_marker(&current_id);
        let path = write_meta(
            &dir,
            "rollout-one.jsonl",
            "019ee58f-fb81-7d53-ab71-06b471bb4247",
            "/repo",
            started_at,
        );

        let got = scan_paths_with_marker(vec![path], "/repo", started_at, &marker, None);

        assert_eq!(got, (CaptureScan::None, ScanSource::Marker));
        assert!(
            !fallback_row_is_unambiguous(&pool, &current_id, row_started_at, "/repo"),
            "same-cwd sibling means pre-marker fallback would permanently fail closed",
        );
    }

    #[test]
    fn empty_pid_result_uses_guarded_fallback_scan() {
        let root = tempfile::tempdir().unwrap();
        let started_at = "2026-06-20T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let local_started = started_at.with_timezone(&Local);
        let date_dir = root.path().join("2026").join("06").join("20");
        std::fs::create_dir_all(&date_dir).unwrap();
        let id = "019ee58f-fb81-7d53-ab71-06b471bb4247";
        let path = write_meta_at_dir(&date_dir, "rollout-one.jsonl", id, "/repo", started_at);
        let mut done = HashSet::new();

        let got = scan_pid_result_or_fallback(
            Ok(Vec::new()),
            root.path(),
            &[local_started],
            "/repo",
            started_at,
            None,
            &mut done,
        );

        assert_eq!(
            got,
            (
                CaptureScan::Unique(MatchCandidate {
                    timestamp: started_at,
                    path,
                    id: id.to_string(),
                }),
                ScanSource::Fallback,
            )
        );
    }

    #[test]
    fn persist_capture_requires_matching_row_started_at() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let started_a = "2026-06-20T12:00:00+00:00";
        let started_b = "2026-06-20T12:01:00+00:00";
        let key_a = "019ee58f-fb81-7d53-ab71-06b471bb4247";
        let key_b = "019ee58f-fb81-7d53-ab71-06b471bb4248";
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES (?1, 'codex-capture', 'Codex Capture', 'codex', 'codex', ?2, ?2)",
            params![runner_id, started_a],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, status, started_at, agent_session_key)
             VALUES (?1, NULL, ?2, 'running', ?3, NULL)",
            params![session_id, runner_id, started_a],
        )
        .unwrap();
        drop(conn);

        assert!(persist_capture(&pool, &session_id, started_a, key_a));
        let stored: Option<String> = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT agent_session_key FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(stored.as_deref(), Some(key_a));

        {
            let conn = pool.get().unwrap();
            conn.execute(
                "UPDATE sessions
                    SET started_at = ?2,
                        agent_session_key = NULL
                  WHERE id = ?1",
                params![session_id, started_b],
            )
            .unwrap();
        }

        assert!(
            !persist_capture(&pool, &session_id, started_a, key_b),
            "stale watcher from started_at A must not write into row incarnation B",
        );
        let stored: Option<String> = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT agent_session_key FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(stored, None);
    }

    #[test]
    fn fallback_row_is_unambiguous_requires_current_row_as_only_possible_owner_for_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = ulid::Ulid::new().to_string();
        let current_id = ulid::Ulid::new().to_string();
        let sibling_id = ulid::Ulid::new().to_string();
        let started = "2026-06-20T12:00:00+00:00";
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES (?1, 'codex-fallback', 'Codex Fallback', 'codex', 'codex', ?2, ?2)",
            params![runner_id, started],
        )
        .unwrap();
        for id in [&current_id, &sibling_id] {
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at, agent_session_key)
                 VALUES (?1, NULL, ?2, '/repo', 'running', ?3, NULL)",
                params![id, runner_id, started],
            )
            .unwrap();
        }
        drop(conn);

        assert!(
            !fallback_row_is_unambiguous(&pool, &current_id, started, "/repo"),
            "fallback must not persist when a sibling live Codex row could own the rollout",
        );

        {
            let conn = pool.get().unwrap();
            conn.execute(
                "UPDATE sessions
                    SET agent_session_key = ?2
                  WHERE id = ?1",
                params![sibling_id, "019ee58f-fb81-7d53-ab71-06b471bb4248"],
            )
            .unwrap();
        }

        assert!(
            fallback_row_is_unambiguous(&pool, &current_id, started, "/repo"),
            "fallback may persist only when the current row is the sole live unkeyed Codex row",
        );

        {
            let conn = pool.get().unwrap();
            conn.execute(
                "UPDATE sessions
                    SET status = 'stopped'
                  WHERE id = ?1",
                params![current_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions
                    SET agent_session_key = NULL
                  WHERE id = ?1",
                params![sibling_id],
            )
            .unwrap();
        }

        assert!(
            !fallback_row_is_unambiguous(&pool, &current_id, started, "/repo"),
            "fallback must not persist for a stopped/stale current row even when a sibling is the only live row",
        );
    }

    #[test]
    fn fallback_row_is_unambiguous_treats_null_inherited_cwd_as_possible_same_cwd() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let runner_id = ulid::Ulid::new().to_string();
        let current_id = ulid::Ulid::new().to_string();
        let sibling_id = ulid::Ulid::new().to_string();
        let started = "2026-06-20T12:00:00+00:00";
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command, created_at, updated_at)
             VALUES (?1, 'codex-inherited', 'Codex Inherited', 'codex', 'codex', ?2, ?2)",
            params![runner_id, started],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, cwd, status, started_at, agent_session_key)
             VALUES (?1, NULL, ?2, NULL, 'running', ?3, NULL)",
            params![current_id, runner_id, started],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, cwd, status, started_at, agent_session_key)
             VALUES (?1, NULL, ?2, '/repo', 'running', ?3, NULL)",
            params![sibling_id, runner_id, started],
        )
        .unwrap();
        drop(conn);

        assert!(
            !fallback_row_is_unambiguous(&pool, &current_id, started, "/repo"),
            "fallback must fail closed when an inherited-cwd row and explicit-cwd row could share the rollout cwd",
        );
    }
}
