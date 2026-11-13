// Antigravity CLI conversation-key capture (spec 644 decision 1).
//
// agy assigns the conversation id itself, and only when the first message is
// sent, so Runner cannot pre-assign it the way it does for Claude Code. Every
// spawn passes `--log-file <app data>/antigravity/logs/<runner session id>.log`,
// a channel that belongs to one session, and a thread tails it for agy's
// `Streaming conversation <uuid>` line, then writes the active id into
// `agent_session_key`. agy emits this line after initial creation, /new,
// /fork and /resume. A blank chat keeps tailing until its first message or
// the end of the session.
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
const ACTIVE_CONVERSATION: &str = "] Streaming conversation ";
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

/// The active conversation id in one of agy's manager log lines.
pub(crate) fn active_conversation(line: &str) -> Option<&str> {
    let (source, rest) = line.split_once(ACTIVE_CONVERSATION)?;
    if !source
        .split_whitespace()
        .last()
        .and_then(|token| token.strip_prefix("conversation_manager.go:"))
        .is_some_and(|line| !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let id = rest.split_whitespace().next()?;
    uuid::Uuid::parse_str(id).is_ok().then_some(id)
}

pub(crate) struct CaptureRequest {
    pub(crate) manager: std::sync::Weak<crate::session::manager::SessionManager>,
    pub(crate) session_id: String,
    pub(crate) mission_id: Option<String>,
    pub(crate) log_path: PathBuf,
    pub(crate) summaries_db_path: Option<PathBuf>,
    pub(crate) cwd: Option<String>,
    pub(crate) initial_key: Option<String>,
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
    let mut active_key = request.initial_key.clone();
    let mut title_sync = SummaryTitleSync::default();
    loop {
        // Read once more after the stop flag so a line written just before
        // exit still lands.
        let stopped = request.stop.load(Ordering::Acquire);
        for key in tail.next_conversations(&request.log_path) {
            persist(&request, &key);
            active_key = Some(key);
        }
        if let (Some(summaries_db), Some(conversation_id)) =
            (request.summaries_db_path.as_deref(), active_key.as_deref())
        {
            title_sync.poll(&request, summaries_db, conversation_id);
        }
        if stopped {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

// `agy` 1.5.8 generates first-turn titles asynchronously into
// `conversation_summaries.db`, but `cli/store` skips `currentConversationID`
// on summary broadcasts and leaves `Store.Conversation.Title` empty until
// `/rename` or `--conversation` resume. Short-lived read-only queries pick up
// the native title without mutating or holding locks on `agy`'s database.
#[derive(Default)]
struct SummaryTitleSync {
    last_observed: Option<(String, String)>,
}

impl SummaryTitleSync {
    fn poll(&mut self, request: &CaptureRequest, summaries_db: &Path, conversation_id: &str) {
        let Some(raw) = read_summary_title(summaries_db, conversation_id) else {
            return;
        };
        let Some(cleaned) = crate::session::title::provider_title(&raw, request.cwd.as_deref())
        else {
            return;
        };
        if self
            .last_observed
            .as_ref()
            .is_some_and(|(id, title)| id == conversation_id && title == &cleaned)
        {
            return;
        }
        if persist_title(request, &cleaned) {
            self.last_observed = Some((conversation_id.to_owned(), cleaned));
        }
    }
}

fn read_summary_title(path: &Path, conversation_id: &str) -> Option<String> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    if !path.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(Duration::from_millis(50));
    let _ = conn.pragma_update(None, "query_only", true);
    conn.query_row(
        "SELECT title FROM conversation_summaries WHERE conversation_id = ?1",
        rusqlite::params![conversation_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn persist_title(request: &CaptureRequest, cleaned: &str) -> bool {
    let Some(manager) = request.manager.upgrade() else {
        return false;
    };
    manager
        .apply_provider_title(crate::session::manager::terminal::ProviderTitleUpdate {
            session_id: &request.session_id,
            expected_started_at: &request.expected_row_started_at,
            cleaned_title: cleaned,
            mission_id: request.mission_id.as_deref(),
            pool: &request.pool,
            events: &request.events,
        })
        .unwrap_or(false)
}

fn persist(request: &CaptureRequest, key: &str) {
    let Some(manager) = request.manager.upgrade() else {
        return;
    };
    let updated = manager
        .persist_reported_key(
            &request.session_id,
            crate::session::state::PersistKey {
                key: Some(key.into()),
                generation: request.expected_row_started_at.clone(),
                origin: crate::session::state::KeyOrigin::Rekeyed,
            },
            &request.pool,
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
    fn next_conversations(&mut self, path: &Path) -> Vec<String> {
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
            return Vec::new();
        };
        if length < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let Ok(read) = file.read_to_end(&mut bytes) else {
            return Vec::new();
        };
        self.offset += read as u64;
        self.pending.extend_from_slice(&bytes);
        let mut found = Vec::new();
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let line = String::from_utf8_lossy(&self.pending[consumed..consumed + end]);
            consumed += end + 1;
            if let Some(id) = active_conversation(&line) {
                found.push(id.to_owned());
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
    fn active_conversation_reads_agys_log_line_and_nothing_else() {
        let ids: Vec<_> = FIXTURE.lines().filter_map(active_conversation).collect();
        assert_eq!(ids, ["1b82ea2b-dba2-41e7-8318-7e3fe3680ebf"]);
        for line in [
            "I0922 16:29:57.8 1 server.go:1] Resuming conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "I0922 16:29:57.8 1 server.go:1] Created conversation not-a-uuid",
            "I0922 16:29:57.8 1 server.go:1] Created conversation ",
            "Created conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "I0922 16:29:57.8 1 subagent_manager.go:1] Streaming conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "I0922 16:29:57.8 1 server.go:1] Streaming conversation 1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
        ] {
            assert_eq!(active_conversation(line), None, "{line}");
        }
    }

    #[test]
    fn tail_keeps_every_switch_in_one_poll() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.log");
        let ids = [
            "1b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "2b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
            "3b82ea2b-dba2-41e7-8318-7e3fe3680ebf",
        ];
        let lines = ids.iter().map(|id| format!("I0928 22:00:00.000000 1 conversation_manager.go:887] Streaming conversation {id}\n")).collect::<String>();
        std::fs::write(&path, lines).unwrap();
        let mut tail = LogTail::default();
        assert_eq!(tail.next_conversations(&path), ids);
        assert!(tail.next_conversations(&path).is_empty());
    }

    #[test]
    fn tail_waits_for_complete_lines_and_restarts_after_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.log");
        let mut tail = LogTail::default();
        assert!(tail.next_conversations(&path).is_empty());

        let (head, created) = FIXTURE.split_at(FIXTURE.find("I0922 16:29:57.817770").unwrap());
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(head.as_bytes()).unwrap();
        file.write_all(&created.as_bytes()[..40]).unwrap();
        assert!(tail.next_conversations(&path).is_empty());
        file.write_all(&created.as_bytes()[40..]).unwrap();
        assert_eq!(
            tail.next_conversations(&path),
            ["1b82ea2b-dba2-41e7-8318-7e3fe3680ebf"]
        );

        std::fs::write(
            &path,
            "I0 1 conversation_manager.go:887] Streaming conversation 2c82ea2b-dba2-41e7-8318-7e3fe3680ebf\n",
        )
        .unwrap();
        assert_eq!(
            tail.next_conversations(&path),
            ["2c82ea2b-dba2-41e7-8318-7e3fe3680ebf"]
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

    #[test]
    fn summary_poll_reads_wal_filters_decorations_and_respects_generation() {
        #[derive(Default)]
        struct RecordedEvents(std::sync::Mutex<Vec<String>>);
        impl SessionEvents for RecordedEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
            fn updated(&self, event: &SessionUpdatedEvent) {
                self.0.lock().unwrap().push(event.session_id.clone());
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let missing_db = temp.path().join("missing/conversation_summaries.db");
        assert_eq!(read_summary_title(&missing_db, "conv-1"), None);
        assert!(!missing_db.exists(), "read-only poll must never create DB");

        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT ''
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let mut row =
            crate::test_support::test_session_row("s-agy", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let events = Arc::new(RecordedEvents::default());
        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-agy".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-agy"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: None,
            expected_row_started_at: started_at.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: events.clone(),
        };

        let mut sync = SummaryTitleSync::default();
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
                [c1, ""],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap()
                .live_title,
            None
        );

        for ignored in ["   ", "Antigravity CLI", "agy", "/tmp/my-project"] {
            writer
                .execute(
                    "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
                    [c1, ignored],
                )
                .unwrap();
            sync.poll(&request, &db_path, c1);
            assert_eq!(
                crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                    .unwrap()
                    .unwrap()
                    .live_title,
                None,
                "{ignored:?}"
            );
        }

        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
                [c1, "  Rainbows Form Through Light Refraction  "],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Rainbows Form Through Light Refraction")
        );
        assert_eq!(events.0.lock().unwrap().len(), 1);

        // If OSC 2 immediately sets a newer `/rename` title before `conversation_summaries.db`
        // flushes, polling the unchanged summary row must not revert `live_title`.
        crate::repo::session::set_live_title(
            &pool.get().unwrap(),
            "s-agy",
            Some("Newer OSC Rename"),
            Some(&started_at),
        )
        .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Newer OSC Rename")
        );
        assert_eq!(events.0.lock().unwrap().len(), 1);

        // Switching conversations picks up the new conversation's summary title.
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
                [c2, "Switched Topic"],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Switched Topic")
        );

        // Stale generation (`expected_row_started_at` mismatch) never overwrites `live_title`.
        let stale_request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-agy".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-agy"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: None,
            expected_row_started_at: "1999-01-01T00:00:00+00:00".into(),
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: events.clone(),
        };
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
                [c2, "Stale Generation Title"],
            )
            .unwrap();
        sync.poll(&stale_request, &db_path, c2);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Switched Topic")
        );
    }
}
