// Antigravity CLI conversation-key capture (spec 644 decision 1).
//
// agy assigns the conversation id itself, and only when the first message is
// sent, so Runner cannot pre-assign it the way it does for Claude Code. Every
// spawn passes `--log-file <app data>/antigravity/logs/<runner session id>.log`,
// a channel that belongs to one session, and a thread tails it for agy's
// `Created conversation <uuid>` line, then writes the id into
// `agent_session_key`. A blank chat keeps tailing until its first message or
// the end of the session. On a resume that agy could not honour, the new
// conversation's line replaces the stale key.
//
// The line is agy's internal log, not a contract: the fixture test pins its
// shape, and a miss fails soft (no key, no resume), as Codex capture does.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::db::DbPool;
use crate::session::manager::{SessionEvents, SessionUpdatedEvent};

const LOG_DIR: &str = "antigravity/logs";
const CREATED_CONVERSATION: &str = "] Created conversation ";
const POLL_INTERVAL: Duration = Duration::from_millis(400);

pub(crate) fn log_path(app_data_dir: &Path, session_id: &str) -> PathBuf {
    app_data_dir.join(LOG_DIR).join(format!("{session_id}.log"))
}

/// Makes the log directory and drops the previous incarnation's log, so the
/// capture only ever reads lines this spawn wrote.
pub(crate) fn prepare_log(app_data_dir: &Path, session_id: &str) {
    let path = log_path(app_data_dir, session_id);
    if let Err(error) = std::fs::create_dir_all(app_data_dir.join(LOG_DIR)) {
        log::warn!("create Antigravity log directory: {error}");
    }
    remove_file(&path);
}

pub(crate) fn remove_log(app_data_dir: &Path, session_id: &str) {
    remove_file(&log_path(app_data_dir, session_id));
}

fn remove_file(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("remove Antigravity log {}: {error}", path.display());
        }
    }
}

/// Removes the logs of session rows that no longer exist, which a mission,
/// role or crew delete leaves behind through the database cascade.
pub(crate) fn clear_orphans(app_data_dir: &Path, pool: &DbPool) {
    let Ok(entries) = std::fs::read_dir(app_data_dir.join(LOG_DIR)) else {
        return;
    };
    let Ok(conn) = pool.get() else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("log") {
            continue;
        }
        let Some(session_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if matches!(crate::repo::session::get_row(&conn, session_id), Ok(None)) {
            remove_file(&path);
        }
    }
}

/// The conversation id in one of agy's `Created conversation <uuid>` lines.
pub(crate) fn created_conversation(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once(CREATED_CONVERSATION)?;
    let id = rest.split_whitespace().next()?;
    uuid::Uuid::parse_str(id).is_ok().then_some(id)
}

pub(crate) struct CaptureRequest {
    pub(crate) session_id: String,
    pub(crate) mission_id: Option<String>,
    pub(crate) log_path: PathBuf,
    pub(crate) expected_row_started_at: String,
    /// The session's output stop flag: set when the PTY exits or is killed.
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) pool: Arc<DbPool>,
    pub(crate) events: Arc<dyn SessionEvents>,
}

pub(crate) fn spawn_capture(request: CaptureRequest) {
    let name = format!("agy-capture-{}", request.session_id);
    if let Err(error) = std::thread::Builder::new()
        .name(name)
        .spawn(move || run(request))
    {
        log::warn!("spawn Antigravity key capture: {error}");
    }
}

fn run(request: CaptureRequest) {
    let mut tail = LogTail::default();
    loop {
        // Read once more after the stop flag so a line written just before
        // exit still lands.
        let stopped = request.stop.load(Ordering::Acquire);
        if let Some(key) = tail.next_conversation(&request.log_path) {
            persist(&request, &key);
            return;
        }
        if stopped {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn persist(request: &CaptureRequest, key: &str) {
    let Ok(conn) = request.pool.get() else { return };
    // Guarded by the row's start time and running status, so a thread from a
    // previous incarnation of this row cannot write into a later one.
    let updated = crate::repo::session::rekey_agent_session_key(
        &conn,
        &request.session_id,
        key,
        &request.expected_row_started_at,
    )
    .unwrap_or(false);
    if updated {
        log::info!(
            "Antigravity conversation captured: session={} key={key}",
            request.session_id
        );
        request.events.updated(&SessionUpdatedEvent {
            session_id: request.session_id.clone(),
            mission_id: request.mission_id.clone(),
        });
    }
}

#[derive(Default)]
struct LogTail {
    offset: u64,
    pending: Vec<u8>,
}

impl LogTail {
    fn next_conversation(&mut self, path: &Path) -> Option<String> {
        let mut file = File::open(path).ok()?;
        let length = file.metadata().ok()?.len();
        if length < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut bytes = Vec::new();
        self.offset += file.read_to_end(&mut bytes).ok()? as u64;
        self.pending.extend_from_slice(&bytes);
        let mut found = None;
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let line = String::from_utf8_lossy(&self.pending[consumed..consumed + end]);
            consumed += end + 1;
            if let Some(id) = created_conversation(&line) {
                found = Some(id.to_owned());
                break;
            }
        }
        self.pending.drain(..consumed);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Shapes copied from agy 1.2.8 logs on macOS (glog format), ids replaced.
    const FIXTURE: &str = "\
I0922 16:29:49.923655     231 server.go:3056] Failed to read conversations directory /Users/me/.gemini/antigravity/conversations: open /Users/me/.gemini/antigravity/conversations: no such file or directory
I0922 16:29:49.928144       1 hooks_manager.go:53] loaded 1 named hooks from 1 hooks.json file(s)
I0922 16:29:57.810387     580 conversation_manager.go:512] Starting new conversation (agent=false)
I0922 16:29:57.817770     580 server.go:1224] Created conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf
I0922 16:29:57.817817     580 server.go:3192] GetConversationDetail: found conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf (active=true)
I0922 16:29:57.818131     580 conversation_manager.go:887] Streaming conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf
";

    #[test]
    fn created_conversation_reads_agys_log_line_and_nothing_else() {
        let ids: Vec<_> = FIXTURE.lines().filter_map(created_conversation).collect();
        assert_eq!(ids, ["1b82ea2b-dba2-41e7-8318-7e3fe3680ebf"]);
        for line in [
            "I0922 16:29:57.8 1 server.go:1] Resuming conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "I0922 16:29:57.8 1 server.go:1] Created conversation not-a-uuid",
            "I0922 16:29:57.8 1 server.go:1] Created conversation ",
            "Created conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
        ] {
            assert_eq!(created_conversation(line), None, "{line}");
        }
    }

    #[test]
    fn tail_waits_for_complete_lines_and_restarts_after_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.log");
        let mut tail = LogTail::default();
        assert_eq!(tail.next_conversation(&path), None);

        let (head, created) = FIXTURE.split_at(FIXTURE.find("I0922 16:29:57.817770").unwrap());
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(head.as_bytes()).unwrap();
        file.write_all(&created.as_bytes()[..40]).unwrap();
        assert_eq!(tail.next_conversation(&path), None);
        file.write_all(&created.as_bytes()[40..]).unwrap();
        assert_eq!(
            tail.next_conversation(&path).as_deref(),
            Some("1b82ea2b-dba2-41e7-8318-7e3fe3680ebf")
        );

        std::fs::write(
            &path,
            "I0 1 server.go:1] Created conversation 2c82ea2b-dba2-41e7-8318-7e3fe3680ebf\n",
        )
        .unwrap();
        assert_eq!(
            tail.next_conversation(&path).as_deref(),
            Some("2c82ea2b-dba2-41e7-8318-7e3fe3680ebf")
        );
    }

    #[test]
    fn orphan_sweep_keeps_logs_whose_row_still_exists() {
        let temp = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let kept = "kept-session";
        crate::repo::session::insert(
            &pool.get().unwrap(),
            &crate::test_support::test_session_row(kept, crate::model::SessionStatus::Stopped),
        )
        .unwrap();
        prepare_log(temp.path(), kept);
        for id in [kept, "deleted-session"] {
            std::fs::write(log_path(temp.path(), id), FIXTURE).unwrap();
        }
        let other = temp.path().join(LOG_DIR).join("notes.txt");
        std::fs::write(&other, "not a log").unwrap();

        clear_orphans(temp.path(), &pool);
        assert!(log_path(temp.path(), kept).exists());
        assert!(!log_path(temp.path(), "deleted-session").exists());
        assert!(other.exists());
    }

    #[test]
    fn prepare_makes_the_directory_and_drops_a_stale_log() {
        let temp = tempfile::tempdir().unwrap();
        let path = log_path(temp.path(), "s1");
        prepare_log(temp.path(), "s1");
        assert!(path.parent().unwrap().is_dir());
        std::fs::write(&path, FIXTURE).unwrap();
        prepare_log(temp.path(), "s1");
        assert!(!path.exists());
        std::fs::write(&path, FIXTURE).unwrap();
        remove_log(temp.path(), "s1");
        assert!(!path.exists());
    }
}
