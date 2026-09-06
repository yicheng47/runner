use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::{
    Event as NotifyEvent, EventKind as NotifyKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde::Deserialize;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::repo;

use super::manager::{SessionEvents, SessionUpdatedEvent};

const DROP_DIR_NAME: &str = "session-keys";

pub(crate) fn drop_path(app_data_dir: &Path, runner_session_id: &str) -> PathBuf {
    app_data_dir
        .join(DROP_DIR_NAME)
        .join(format!("{runner_session_id}.json"))
}

#[derive(Deserialize)]
struct SessionStartReport {
    session_id: String,
}

pub(crate) struct ClaudeSessionKeyWatcher {
    shutdown: Arc<AtomicBool>,
    _consumer: JoinHandle<()>,
    _watcher: RecommendedWatcher,
}

impl ClaudeSessionKeyWatcher {
    pub(crate) fn start(
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Self> {
        let drop_dir = app_data_dir.join(DROP_DIR_NAME);
        fs::create_dir_all(&drop_dir)?;
        clear_leftovers(&drop_dir)?;

        let (tx, rx) = channel::<()>();
        let mut watcher = notify::recommended_watcher(
            move |result: std::result::Result<NotifyEvent, notify::Error>| {
                let Ok(event) = result else { return };
                if !matches!(
                    event.kind,
                    NotifyKind::Modify(_) | NotifyKind::Create(_) | NotifyKind::Any
                ) {
                    return;
                }
                let _ = tx.send(());
            },
        )
        .map_err(|error| Error::msg(format!("session-key notify watcher: {error}")))?;
        watcher
            .watch(&drop_dir, RecursiveMode::NonRecursive)
            .map_err(|error| {
                Error::msg(format!(
                    "session-key notify watch {}: {error}",
                    drop_dir.display()
                ))
            })?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let drop_dir_for_thread = drop_dir.clone();
        let consumer = thread::Builder::new()
            .name("claude-session-key".into())
            .spawn(move || loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                if let Err(error) = scan_drop_dir(&drop_dir_for_thread, &pool, events.as_ref()) {
                    log::warn!(
                        "scan Claude session-key reports {}: {error}",
                        drop_dir_for_thread.display()
                    );
                }
                if shutdown_for_thread.load(Ordering::SeqCst) {
                    return;
                }
            })
            .map_err(|error| Error::msg(format!("spawn session-key consumer: {error}")))?;

        Ok(Self {
            shutdown,
            _consumer: consumer,
            _watcher: watcher,
        })
    }
}

impl Drop for ClaudeSessionKeyWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn clear_leftovers(drop_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(drop_dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!(
                    "read stale Claude session-key entry in {}: {error}",
                    drop_dir.display()
                );
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                log::warn!(
                    "inspect stale Claude session-key entry {}: {error}",
                    entry.path().display()
                );
                continue;
            }
        };
        if file_type.is_file() || file_type.is_symlink() {
            if let Err(error) = fs::remove_file(entry.path()) {
                if error.kind() != ErrorKind::NotFound {
                    log::warn!(
                        "remove stale Claude session-key entry {}: {error}",
                        entry.path().display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn scan_drop_dir(drop_dir: &Path, pool: &DbPool, events: &dyn SessionEvents) -> Result<()> {
    for entry in fs::read_dir(drop_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("json")) {
            process_drop_file(&path, pool, events);
        }
    }
    Ok(())
}

fn process_drop_file(path: &Path, pool: &DbPool, events: &dyn SessionEvents) {
    if let Err(error) = try_process_drop_file(path, pool, events) {
        log::warn!(
            "process Claude session-key report {}: {error}",
            path.display()
        );
    }
    // Reports are single-use even when a transient processing error occurs;
    // leaving one behind could apply a stale key to a later spawn incarnation.
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != ErrorKind::NotFound {
            log::warn!(
                "remove Claude session-key report {}: {error}",
                path.display()
            );
        }
    }
}

fn try_process_drop_file(path: &Path, pool: &DbPool, events: &dyn SessionEvents) -> Result<()> {
    let Some(runner_session_id) = path
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let report: SessionStartReport = match serde_json::from_slice(&bytes) {
        Ok(report) => report,
        Err(_) => return Ok(()),
    };
    if uuid::Uuid::parse_str(&report.session_id).is_err() {
        return Ok(());
    }

    let conn = pool.get()?;
    let Some(row) = repo::session::get_row(&conn, runner_session_id)? else {
        return Ok(());
    };
    if row.agent_session_key.as_deref() == Some(report.session_id.as_str()) {
        return Ok(());
    }
    let Some(started_at) = row.started_at else {
        return Ok(());
    };
    if repo::session::rekey_agent_session_key(
        &conn,
        runner_session_id,
        &report.session_id,
        &started_at.to_rfc3339(),
    )? {
        events.updated(&SessionUpdatedEvent {
            session_id: runner_session_id.to_string(),
            mission_id: row.mission_id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use chrono::Utc;

    use super::*;
    use crate::model::SessionStatus;
    use crate::session::manager::{ExitEvent, OutputEvent};

    #[derive(Default)]
    struct Capture {
        updated: Mutex<Vec<SessionUpdatedEvent>>,
    }

    impl SessionEvents for Capture {
        fn output(&self, _ev: &OutputEvent) {}

        fn exit(&self, _ev: &ExitEvent) {}

        fn updated(&self, ev: &SessionUpdatedEvent) {
            self.updated.lock().unwrap().push(ev.clone());
        }
    }

    fn insert_running(pool: &DbPool, id: &str, key: &str) {
        let conn = pool.get().unwrap();
        let mut row = repo::session::SessionRowDb::new_running(id.to_string());
        row.started_at = Some(Utc::now());
        row.agent_session_key = Some(key.to_string());
        repo::session::insert(&conn, &row).unwrap();
    }

    #[test]
    fn valid_report_rekeys_running_session_and_emits_update() {
        let root = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let events = Capture::default();
        let old_key = uuid::Uuid::new_v4().to_string();
        let new_key = uuid::Uuid::new_v4().to_string();
        insert_running(&pool, "runner-session", &old_key);
        let path = drop_path(root.path(), "runner-session");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::json!({ "session_id": new_key }).to_string(),
        )
        .unwrap();

        process_drop_file(&path, &pool, &events);

        let conn = pool.get().unwrap();
        let row = repo::session::get_row(&conn, "runner-session")
            .unwrap()
            .unwrap();
        assert_eq!(row.agent_session_key.as_deref(), Some(new_key.as_str()));
        assert_eq!(events.updated.lock().unwrap().len(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn same_key_report_is_a_no_op_without_an_event() {
        let root = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let events = Capture::default();
        let key = uuid::Uuid::new_v4().to_string();
        insert_running(&pool, "runner-session", &key);
        let path = drop_path(root.path(), "runner-session");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::json!({ "session_id": key }).to_string()).unwrap();

        process_drop_file(&path, &pool, &events);

        assert!(events.updated.lock().unwrap().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn malformed_and_unknown_reports_are_ignored_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().unwrap();
        let events = Capture::default();
        let key = uuid::Uuid::new_v4().to_string();
        insert_running(&pool, "runner-session", &key);
        let malformed = drop_path(root.path(), "runner-session");
        let invalid_uuid = drop_path(root.path(), "runner-session-invalid-uuid");
        let unknown = drop_path(root.path(), "unknown-session");
        fs::create_dir_all(malformed.parent().unwrap()).unwrap();
        fs::write(&malformed, b"not json").unwrap();
        fs::write(
            &invalid_uuid,
            serde_json::json!({ "session_id": "not-a-uuid" }).to_string(),
        )
        .unwrap();
        fs::write(
            &unknown,
            serde_json::json!({ "session_id": uuid::Uuid::new_v4().to_string() }).to_string(),
        )
        .unwrap();

        process_drop_file(&malformed, &pool, &events);
        process_drop_file(&invalid_uuid, &pool, &events);
        process_drop_file(&unknown, &pool, &events);

        let conn = pool.get().unwrap();
        let row = repo::session::get_row(&conn, "runner-session")
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Running);
        assert_eq!(row.agent_session_key.as_deref(), Some(key.as_str()));
        assert!(events.updated.lock().unwrap().is_empty());
        assert!(!malformed.exists());
        assert!(!invalid_uuid.exists());
        assert!(!unknown.exists());
    }

    #[test]
    fn watcher_processes_atomic_drop_through_poll_fallback() {
        let root = tempfile::tempdir().unwrap();
        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let events = Arc::new(Capture::default());
        let old_key = uuid::Uuid::new_v4().to_string();
        let new_key = uuid::Uuid::new_v4().to_string();
        insert_running(&pool, "runner-session", &old_key);
        let watcher = ClaudeSessionKeyWatcher::start(
            root.path(),
            Arc::clone(&pool),
            Arc::clone(&events) as Arc<dyn SessionEvents>,
        )
        .unwrap();
        let path = drop_path(root.path(), "runner-session");
        let temp_path = path.with_extension("json.tmp");
        fs::write(
            &temp_path,
            serde_json::json!({ "session_id": new_key }).to_string(),
        )
        .unwrap();
        fs::rename(&temp_path, &path).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let conn = pool.get().unwrap();
            let row = repo::session::get_row(&conn, "runner-session")
                .unwrap()
                .unwrap();
            if row.agent_session_key.as_deref() == Some(new_key.as_str()) && !path.exists() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watcher never processed drop file"
            );
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(events.updated.lock().unwrap().len(), 1);
        assert!(!path.exists());
        drop(watcher);
    }
}
