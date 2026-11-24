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

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

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
    let title_sync = SummaryTitleSync::for_launch(&request);
    if let Err(error) = std::thread::Builder::new()
        .name(name)
        .spawn(move || run(request, title_sync))
    {
        log::warn!("spawn Antigravity key capture: {error}");
    }
}

fn run(request: CaptureRequest, mut title_sync: SummaryTitleSync) {
    let mut tail = LogTail::default();
    let mut active_key = request.initial_key.clone();
    loop {
        // Read once more after the stop flag so a line written just before
        // exit still lands.
        let stopped = request.stop.load(Ordering::Acquire);
        for key in tail.next_conversations(&request.log_path) {
            persist(&request, &key);
            if active_key.as_deref() != Some(key.as_str()) {
                title_sync.switch_conversation(active_key.as_deref(), &key);
                active_key = Some(key);
            }
        }
        if let (Some(summaries_db), Some(conversation_id)) =
            (request.summaries_db_path.as_deref(), active_key.as_deref())
        {
            title_sync.poll(&request, summaries_db, conversation_id);
            #[cfg(test)]
            if let Some(manager) = request.manager.upgrade() {
                manager.record_agy_summary_poll();
            }
        }
        if stopped {
            #[cfg(test)]
            if let Some(manager) = request.manager.upgrade() {
                manager.record_agy_capture_exit();
            }
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRow {
    title: String,
    last_modified_time: Option<DateTime<Utc>>,
}

type SharedOscTracker = Arc<Mutex<crate::session::manager::terminal::OscTitleTracker>>;

#[derive(Default)]
struct ConversationSummaryState {
    highest_modified_at: Option<DateTime<Utc>>,
    observed_db_title: Option<String>,
    trusted_summary: Option<TrustedSummary>,
    pending_baseline: bool,
}

impl ConversationSummaryState {
    fn advance_modified_at(&mut self, ts: DateTime<Utc>) {
        self.highest_modified_at = Some(match self.highest_modified_at {
            Some(current) => current.max(ts),
            None => ts,
        });
    }
}

struct TrustedSummary {
    title: String,
    modified_at: Option<DateTime<Utc>>,
    osc_seq_at_apply: u64,
}

// `agy` 1.5.8 generates first-turn titles asynchronously into
// `conversation_summaries.db`, but `cli/store` skips `currentConversationID`
// on summary broadcasts and leaves `Store.Conversation.Title` empty until
// `/rename` or `--conversation` resume. Short-lived read-only queries pick up
// the native title without mutating or holding locks on `agy`'s database.
#[derive(Default)]
struct SummaryTitleSync {
    /// Per-conversation summary watermarks, observed DB titles, and last trusted titles retained
    /// across conversation switches so returning to an earlier conversation neither accepts a
    /// rollback nor replays an unchanged launch baseline.
    conversations: HashMap<String, ConversationSummaryState>,
    /// Currently tracked conversation id (used to detect switches across direct `poll` calls).
    active_conversation_id: Option<String>,
    /// Set on a real switch to another conversation so a stale, missing, or unchanged-baseline
    /// summary row restores that conversation's last trusted provider title once.
    pending_restore: Option<String>,
    /// Launch conversation whose Runner session row could not be read at launch; hydrated into
    /// `trusted_summary` once the session DB becomes readable.
    pending_launch_trusted_from_row: Option<String>,
    /// Shared per-generation OSC 0/2 freshness tracker, held through the final post-stop poll.
    osc_tracker: Option<SharedOscTracker>,
}

impl SummaryTitleSync {
    /// A resumed conversation's summary can predate a `/rename` Runner already holds, so the
    /// summary at launch is treated as seen and only a later change counts as a new title.
    fn for_launch(request: &CaptureRequest) -> Self {
        let osc_tracker = request.manager.upgrade().map(|manager| {
            manager.osc_title_tracker_for_generation(
                &request.session_id,
                &request.expected_row_started_at,
                request.initial_key.clone(),
                Some(request.log_path.clone()),
            )
        });
        // An unreadable row counts as titled: skipping the baseline could erase the person's name.
        let (titled, existing_live_title, row_unreadable) = match request.pool.get() {
            Ok(conn) => match crate::repo::session::get_row(&conn, &request.session_id) {
                Ok(Some(row)) => {
                    let live = row.live_title.as_deref().and_then(|t| {
                        crate::session::title::provider_title(t, request.cwd.as_deref())
                    });
                    (row.live_title.is_some(), live, false)
                }
                Ok(None) => (false, None, false),
                Err(_) => (true, None, true),
            },
            Err(_) => (true, None, true),
        };
        let mut conversations = HashMap::new();
        let pending_launch_trusted_from_row = row_unreadable
            .then(|| request.initial_key.clone())
            .flatten();
        if let (Some(key), Some(title)) = (request.initial_key.as_deref(), existing_live_title) {
            conversations.insert(
                key.to_owned(),
                ConversationSummaryState {
                    trusted_summary: Some(TrustedSummary {
                        title,
                        modified_at: None,
                        osc_seq_at_apply: 0,
                    }),
                    ..ConversationSummaryState::default()
                },
            );
        }
        let (Some(key), Some(summaries_db)) = (
            request.initial_key.as_deref().filter(|_| titled),
            request.summaries_db_path.as_deref(),
        ) else {
            return Self {
                conversations,
                active_conversation_id: request.initial_key.clone(),
                pending_launch_trusted_from_row,
                osc_tracker,
                ..Self::default()
            };
        };
        match read_summary_title(summaries_db, key) {
            Ok(Some(row)) => {
                let state = conversations.entry(key.to_owned()).or_default();
                if let Some(ts) = row.last_modified_time {
                    state.advance_modified_at(ts);
                    if let Some(trusted) = state.trusted_summary.as_mut() {
                        trusted.modified_at = Some(ts);
                    }
                }
                let cleaned =
                    crate::session::title::provider_title(&row.title, request.cwd.as_deref());
                state.observed_db_title = cleaned.clone();
                if !row_unreadable {
                    if let Some(title) = cleaned {
                        state.trusted_summary.get_or_insert(TrustedSummary {
                            title,
                            modified_at: row.last_modified_time,
                            osc_seq_at_apply: 0,
                        });
                    }
                }
                Self {
                    conversations,
                    active_conversation_id: Some(key.to_owned()),
                    pending_restore: None,
                    pending_launch_trusted_from_row,
                    osc_tracker,
                }
            }
            Ok(None) => Self {
                conversations,
                active_conversation_id: Some(key.to_owned()),
                pending_restore: None,
                pending_launch_trusted_from_row,
                osc_tracker,
            },
            Err(_) => {
                conversations
                    .entry(key.to_owned())
                    .or_default()
                    .pending_baseline = true;
                Self {
                    conversations,
                    active_conversation_id: Some(key.to_owned()),
                    pending_restore: None,
                    pending_launch_trusted_from_row,
                    osc_tracker,
                }
            }
        }
    }

    fn switch_conversation(&mut self, from: Option<&str>, to: &str) {
        self.active_conversation_id = Some(to.to_owned());
        if self.pending_launch_trusted_from_row.as_deref() != Some(to) {
            self.pending_launch_trusted_from_row = None;
        }
        self.pending_restore = from.filter(|prev| *prev != to).map(|_| to.to_owned());
    }

    fn hydrate_launch_trusted_from_row(&mut self, request: &CaptureRequest, conversation_id: &str) {
        if self.pending_launch_trusted_from_row.as_deref() != Some(conversation_id) {
            return;
        }
        let Ok(conn) = request.pool.get() else {
            return;
        };
        let Ok(row) = crate::repo::session::get_row(&conn, &request.session_id) else {
            return;
        };
        self.pending_launch_trusted_from_row = None;
        let state = self
            .conversations
            .entry(conversation_id.to_owned())
            .or_default();
        if state.trusted_summary.is_some() {
            return;
        }
        let existing_live_title = row.and_then(|row| {
            row.live_title
                .as_deref()
                .and_then(|t| crate::session::title::provider_title(t, request.cwd.as_deref()))
        });
        if let Some(title) = existing_live_title.or_else(|| state.observed_db_title.clone()) {
            state.trusted_summary = Some(TrustedSummary {
                title,
                modified_at: state.highest_modified_at,
                osc_seq_at_apply: 0,
            });
        }
    }

    fn osc_tracker(&mut self, request: &CaptureRequest) -> Option<SharedOscTracker> {
        if let Some(tracker) = &self.osc_tracker {
            return Some(Arc::clone(tracker));
        }
        let tracker = request.manager.upgrade().map(|manager| {
            manager.osc_title_tracker_for_generation(
                &request.session_id,
                &request.expected_row_started_at,
                request.initial_key.clone(),
                Some(request.log_path.clone()),
            )
        });
        self.osc_tracker = tracker.clone();
        tracker
    }

    fn try_restore_trusted_title(
        &mut self,
        request: &CaptureRequest,
        conversation_id: &str,
        osc_tracker: Option<&SharedOscTracker>,
        osc_obs: Option<&crate::session::manager::terminal::OscObservation>,
    ) {
        if self.pending_restore.as_deref() != Some(conversation_id) {
            return;
        }
        let trusted_summary = self
            .conversations
            .get(conversation_id)
            .and_then(|state| state.trusted_summary.as_ref());
        let trusted = match (trusted_summary, osc_obs) {
            (Some(summary), Some(osc)) if osc.seq > summary.osc_seq_at_apply => {
                Some((osc.title.clone(), Some(osc.observed_at)))
            }
            (Some(summary), _) => Some((summary.title.clone(), summary.modified_at)),
            (None, Some(osc)) => Some((osc.title.clone(), Some(osc.observed_at))),
            (None, None) => None,
        };
        let Some((title, source_modified_at)) = trusted else {
            self.pending_restore = None;
            return;
        };
        if persist_title(
            request,
            conversation_id,
            &title,
            source_modified_at,
            osc_tracker,
        ) || osc_tracker.is_some_and(|tracker| {
            tracker.lock().unwrap().rejects_provider_title(
                &request.expected_row_started_at,
                conversation_id,
                source_modified_at,
            )
        }) {
            self.pending_restore = None;
        }
    }

    fn poll(&mut self, request: &CaptureRequest, summaries_db: &Path, conversation_id: &str) {
        if let Some(prev) = self.active_conversation_id.clone() {
            if prev != conversation_id {
                self.switch_conversation(Some(&prev), conversation_id);
            }
        } else {
            self.active_conversation_id = Some(conversation_id.to_owned());
        }
        let osc_tracker = self.osc_tracker(request);
        let Ok(row) = read_summary_title(summaries_db, conversation_id) else {
            return;
        };
        let snapshot = osc_tracker
            .as_ref()
            .map(|tracker| {
                tracker
                    .lock()
                    .unwrap()
                    .snapshot_for_generation(&request.expected_row_started_at, conversation_id)
            })
            .unwrap_or_default();
        if snapshot.superseded {
            return;
        }
        let osc_obs = snapshot.observation;
        self.hydrate_launch_trusted_from_row(request, conversation_id);
        let baseline = self
            .conversations
            .get_mut(conversation_id)
            .map(|state| std::mem::take(&mut state.pending_baseline))
            .unwrap_or(false);
        let Some(row) = row else {
            self.try_restore_trusted_title(
                request,
                conversation_id,
                osc_tracker.as_ref(),
                osc_obs.as_ref(),
            );
            return;
        };
        let cleaned = crate::session::title::provider_title(&row.title, request.cwd.as_deref());
        if baseline {
            let state = self
                .conversations
                .entry(conversation_id.to_owned())
                .or_default();
            if let Some(ts) = row.last_modified_time {
                state.advance_modified_at(ts);
                if let Some(trusted) = state.trusted_summary.as_mut() {
                    if trusted.modified_at.is_none() {
                        trusted.modified_at = Some(ts);
                    }
                }
            }
            if let Some(title) = cleaned {
                let osc_seq = osc_obs.as_ref().map_or(0, |osc| osc.seq);
                state.observed_db_title = Some(title.clone());
                if self.pending_launch_trusted_from_row.as_deref() != Some(conversation_id) {
                    state.trusted_summary.get_or_insert(TrustedSummary {
                        title,
                        modified_at: row.last_modified_time,
                        osc_seq_at_apply: osc_seq,
                    });
                }
            }
            self.try_restore_trusted_title(
                request,
                conversation_id,
                osc_tracker.as_ref(),
                osc_obs.as_ref(),
            );
            return;
        }
        if let Some(ts) = row.last_modified_time {
            let watermark = self
                .conversations
                .get(conversation_id)
                .and_then(|state| state.highest_modified_at);
            let osc_floor = osc_obs.as_ref().map(|osc| osc.observed_at);
            let floor = match (watermark, osc_floor) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if floor.is_some_and(|floor| ts < floor) {
                // Record a non-rollback summary flush matching the active live OSC title so a later
                // DB rename back to a prior title is still recognized as a real DB change without
                // writing to Runner's display state.
                if watermark.is_none_or(|w| ts >= w) {
                    if let (Some(title), Some(osc)) = (cleaned.as_ref(), osc_obs.as_ref()) {
                        if title == &osc.title {
                            let state = self
                                .conversations
                                .entry(conversation_id.to_owned())
                                .or_default();
                            state.advance_modified_at(ts);
                            state.observed_db_title = Some(title.clone());
                        }
                    }
                }
                self.try_restore_trusted_title(
                    request,
                    conversation_id,
                    osc_tracker.as_ref(),
                    osc_obs.as_ref(),
                );
                return;
            }
            // Advance the watermark once the provider row passes freshness checks so a transient
            // Runner write failure cannot let an older rollback slip through later; keep
            // `observed_db_title` updated only on successful persistence so same-timestamp retries
            // still apply.
            self.conversations
                .entry(conversation_id.to_owned())
                .or_default()
                .advance_modified_at(ts);
        }
        let Some(cleaned) = cleaned else {
            self.try_restore_trusted_title(
                request,
                conversation_id,
                osc_tracker.as_ref(),
                osc_obs.as_ref(),
            );
            return;
        };
        if self
            .conversations
            .get(conversation_id)
            .and_then(|state| state.observed_db_title.as_deref())
            == Some(cleaned.as_str())
        {
            self.try_restore_trusted_title(
                request,
                conversation_id,
                osc_tracker.as_ref(),
                osc_obs.as_ref(),
            );
            return;
        }
        if persist_title(
            request,
            conversation_id,
            &cleaned,
            row.last_modified_time,
            osc_tracker.as_ref(),
        ) {
            let osc_seq = osc_obs.as_ref().map_or(0, |osc| osc.seq);
            self.pending_restore = None;
            let state = self
                .conversations
                .entry(conversation_id.to_owned())
                .or_default();
            state.observed_db_title = Some(cleaned.clone());
            state.trusted_summary = Some(TrustedSummary {
                title: cleaned,
                modified_at: row.last_modified_time,
                osc_seq_at_apply: osc_seq,
            });
        }
    }
}

fn parse_summary_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f%#z") {
        return Some(dt.with_timezone(&Utc));
    }
    let without_utc = trimmed.strip_suffix(" UTC").unwrap_or(trimmed);
    DateTime::parse_from_str(without_utc, "%Y-%m-%d %H:%M:%S%.f %#z")
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn is_missing_last_modified_time_column(error: &rusqlite::Error) -> bool {
    match error {
        rusqlite::Error::SqlInputError { error, msg, .. } => {
            error.extended_code == rusqlite::ffi::SQLITE_ERROR
                && msg.contains("no such column: last_modified_time")
        }
        rusqlite::Error::SqliteFailure(error, Some(msg)) => {
            error.extended_code == rusqlite::ffi::SQLITE_ERROR
                && msg.contains("no such column: last_modified_time")
        }
        _ => false,
    }
}

fn read_summary_title(path: &Path, conversation_id: &str) -> rusqlite::Result<Option<SummaryRow>> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    if matches!(path.try_exists(), Ok(false)) {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let _ = conn.busy_timeout(Duration::from_millis(50));
    let _ = conn.pragma_update(None, "query_only", true);
    match conn
        .query_row(
            "SELECT title, last_modified_time FROM conversation_summaries WHERE conversation_id = ?1",
            rusqlite::params![conversation_id],
            |row| {
                let title = row.get::<_, Option<String>>(0)?.unwrap_or_default();
                let last_modified_time = match row.get_ref(1)? {
                    rusqlite::types::ValueRef::Text(bytes) => {
                        std::str::from_utf8(bytes).ok().and_then(parse_summary_timestamp)
                    }
                    _ => None,
                };
                Ok(SummaryRow {
                    title,
                    last_modified_time,
                })
            },
        )
        .optional()
    {
        Ok(row) => Ok(row),
        Err(error) if is_missing_last_modified_time_column(&error) => conn
            .query_row(
                "SELECT title FROM conversation_summaries WHERE conversation_id = ?1",
                rusqlite::params![conversation_id],
                |row| {
                    Ok(SummaryRow {
                        title: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        last_modified_time: None,
                    })
                },
            )
            .optional(),
        Err(error) => Err(error),
    }
}

fn persist_title(
    request: &CaptureRequest,
    conversation_id: &str,
    cleaned: &str,
    source_modified_at: Option<DateTime<Utc>>,
    osc_tracker: Option<&SharedOscTracker>,
) -> bool {
    let Some(manager) = request.manager.upgrade() else {
        return false;
    };
    manager
        .apply_provider_title(crate::session::manager::terminal::ProviderTitleUpdate {
            session_id: &request.session_id,
            expected_started_at: &request.expected_row_started_at,
            conversation_id,
            cleaned_title: cleaned,
            source_modified_at,
            osc_tracker,
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
pub(crate) struct LogTail {
    offset: u64,
    pending: Vec<u8>,
}

impl LogTail {
    pub(crate) fn next_conversations(&mut self, path: &Path) -> Vec<String> {
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
        assert_eq!(read_summary_title(&missing_db, "conv-1").unwrap(), None);
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

    #[test]
    fn resumed_summary_replay_keeps_a_newer_runner_name_until_the_summary_changes() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let mut launches = vec!["readable", "sql-locked", "row-unreadable"];
        if cfg!(unix) {
            launches.push("dir-unsearchable");
        }
        for launch in launches {
            let temp = tempfile::tempdir().unwrap();
            let provider = temp.path().join("provider");
            std::fs::create_dir(&provider).unwrap();
            let db_path = provider.join("conversation_summaries.db");
            let writer = rusqlite::Connection::open(&db_path).unwrap();
            writer
                .execute_batch(
                    "CREATE TABLE conversation_summaries (
                        conversation_id TEXT PRIMARY KEY,
                        title TEXT NOT NULL DEFAULT ''
                    );",
                )
                .unwrap();
            let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
            writer
                .execute(
                    "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, 'Old A')",
                    [c1],
                )
                .unwrap();

            let pool = Arc::new(crate::db::open_in_memory().unwrap());
            let manager = crate::session::manager::SessionManager::key_test_manager();
            let mut row = crate::test_support::test_session_row(
                "s-agy",
                crate::model::SessionStatus::Running,
            );
            row.agent_runtime = Some("antigravity".into());
            row.cwd = Some("/tmp/my-project".into());
            row.live_title = Some("Native B".into());
            row.title = Some("Runner C".into());
            let started_at = row.started_at.unwrap().to_rfc3339();
            crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
            let request_with = |pool: &Arc<DbPool>| CaptureRequest {
                manager: Arc::downgrade(&manager),
                session_id: "s-agy".into(),
                mission_id: None,
                log_path: log_path(temp.path(), "s-agy"),
                summaries_db_path: Some(db_path.clone()),
                cwd: Some("/tmp/my-project".into()),
                initial_key: Some(c1.into()),
                expected_row_started_at: started_at.clone(),
                stop: Arc::new(AtomicBool::new(false)),
                pool: Arc::clone(pool),
                events: Arc::new(NoEvents),
            };
            let request = request_with(&pool);
            let stored = || {
                let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                    .unwrap()
                    .unwrap();
                (row.title, row.live_title)
            };
            let names =
                |title: Option<&str>, live: &str| (title.map(str::to_owned), Some(live.to_owned()));

            let mut sync = match launch {
                "sql-locked" => {
                    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
                    let sync = SummaryTitleSync::for_launch(&request);
                    writer.execute_batch("COMMIT").unwrap();
                    sync
                }
                "row-unreadable" => {
                    let unreadable = Arc::new(crate::db::open_in_memory().unwrap());
                    unreadable
                        .get()
                        .unwrap()
                        .execute_batch("ALTER TABLE sessions RENAME TO sessions_unreadable")
                        .unwrap();
                    SummaryTitleSync::for_launch(&request_with(&unreadable))
                }
                #[cfg(unix)]
                "dir-unsearchable" => {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = |mode| {
                        std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(mode))
                            .unwrap()
                    };
                    mode(0o000);
                    let sync = SummaryTitleSync::for_launch(&request);
                    mode(0o755);
                    sync
                }
                _ => SummaryTitleSync::for_launch(&request),
            };
            assert!(
                sync.conversations.get(c1).is_some_and(|state| {
                    state.observed_db_title.is_some() || state.pending_baseline
                }),
                "{launch} launch left no baseline"
            );
            sync.poll(&request, &db_path, c1);
            sync.poll(&request, &db_path, c1);
            assert_eq!(stored(), names(Some("Runner C"), "Native B"), "{launch}");

            // Round-tripping away to c2 and back to c1 while c1's DB row still holds launch
            // baseline "Old A" must restore "Native B" once without ever writing "Old A".
            let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
            writer
                .execute(
                    "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, 'Other C2')",
                    [c2],
                )
                .unwrap();
            pool.get()
                .unwrap()
                .execute_batch(
                    "CREATE TABLE title_writes (title TEXT);
                     CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
                     BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
                )
                .unwrap();
            let written_titles = || -> Vec<Option<String>> {
                let conn = pool.get().unwrap();
                let mut stmt = conn.prepare("SELECT title FROM title_writes").unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect()
            };
            persist(&request, c2);
            sync.switch_conversation(Some(c1), c2);
            sync.poll(&request, &db_path, c2);
            assert_eq!(stored(), names(None, "Other C2"), "{launch}");
            persist(&request, c1);
            sync.switch_conversation(Some(c2), c1);
            sync.poll(&request, &db_path, c1);
            assert_eq!(stored(), names(None, "Native B"), "{launch}");
            assert!(
                !written_titles()
                    .iter()
                    .any(|title| title.as_deref() == Some("Old A")),
                "{launch} replayed launch baseline after round trip"
            );
            crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner C"))
                .unwrap();
            let writes_after_return = written_titles().len();
            sync.poll(&request, &db_path, c1);
            assert_eq!(stored(), names(Some("Runner C"), "Native B"), "{launch}");
            assert_eq!(written_titles().len(), writes_after_return, "{launch}");

            for (summary, expected) in [
                ("Native B", names(Some("Runner C"), "Native B")),
                ("Fresh D", names(None, "Fresh D")),
            ] {
                writer
                    .execute(
                        "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
                        [c1, summary],
                    )
                    .unwrap();
                sync.poll(&request, &db_path, c1);
                assert_eq!(stored(), expected, "{summary}, {launch}");
            }
        }
    }

    #[test]
    fn parse_summary_timestamp_handles_variable_precision_zero_time_and_rfc3339() {
        let five_digits = parse_summary_timestamp("2026-11-24 11:01:36.82741+00:00").unwrap();
        let six_digits_same = parse_summary_timestamp("2026-11-24 11:01:36.827410+00:00").unwrap();
        let one_digit_newer = parse_summary_timestamp("2026-11-24 11:01:36.9+00:00").unwrap();
        let nine_digits = parse_summary_timestamp("2026-11-24 11:01:36.827410001+00:00").unwrap();
        assert_eq!(five_digits, six_digits_same);
        assert!(one_digit_newer > five_digits);
        assert!(nine_digits > five_digits);
        assert!(one_digit_newer > nine_digits);

        let rolled_back = parse_summary_timestamp("2026-11-24 06:54:13.872081+00:00").unwrap();
        let current = parse_summary_timestamp("2026-11-24 11:28:24.867079+00:00").unwrap();
        assert!(rolled_back < current);

        let zero_time = parse_summary_timestamp("0001-01-01 00:00:00+00:00").unwrap();
        assert!(zero_time < rolled_back);
        assert_eq!(
            zero_time,
            parse_summary_timestamp("0001-01-01T00:00:00Z").unwrap()
        );

        assert_eq!(
            current,
            parse_summary_timestamp("2026-11-24T11:28:24.867079Z").unwrap()
        );
        assert_eq!(
            current,
            parse_summary_timestamp("2026-11-24T19:28:24.867079+08:00").unwrap()
        );
        assert_eq!(
            current,
            parse_summary_timestamp("2026-11-24 06:28:24.867079-05:00").unwrap()
        );
        assert_eq!(
            current,
            parse_summary_timestamp("2026-11-24 11:28:24.867079+0000").unwrap()
        );
        assert_eq!(
            current,
            parse_summary_timestamp("2026-11-24 11:28:24.867079 +0000 UTC").unwrap()
        );
        assert!(parse_summary_timestamp("2026-11-24T11:28:24Z").is_some());

        for invalid in ["", "   ", "not-a-timestamp", "2026-13-40 99:99:99+00:00"] {
            assert_eq!(parse_summary_timestamp(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn read_summary_title_falls_back_when_last_modified_time_column_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";

        let legacy_path = temp.path().join("legacy.db");
        let legacy = rusqlite::Connection::open(&legacy_path).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT ''
                );",
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, 'Legacy Title')",
                [c1],
            )
            .unwrap();
        assert_eq!(
            read_summary_title(&legacy_path, c1).unwrap(),
            Some(SummaryRow {
                title: "Legacy Title".into(),
                last_modified_time: None,
            })
        );

        let modern_path = temp.path().join("modern.db");
        let modern = rusqlite::Connection::open(&modern_path).unwrap();
        modern
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT
                );",
            )
            .unwrap();
        modern
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, 'Current Title', '2026-11-24 11:28:24.867079+00:00')",
                [c1],
            )
            .unwrap();
        assert_eq!(
            read_summary_title(&modern_path, c1).unwrap(),
            Some(SummaryRow {
                title: "Current Title".into(),
                last_modified_time: parse_summary_timestamp("2026-11-24 11:28:24.867079+00:00"),
            })
        );
        modern
            .execute(
                "UPDATE conversation_summaries SET last_modified_time = NULL WHERE conversation_id = ?1",
                [c1],
            )
            .unwrap();
        assert_eq!(
            read_summary_title(&modern_path, c1).unwrap(),
            Some(SummaryRow {
                title: "Current Title".into(),
                last_modified_time: None,
            })
        );

        let missing_table_path = temp.path().join("empty.db");
        let _empty = rusqlite::Connection::open(&missing_table_path).unwrap();
        assert!(read_summary_title(&missing_table_path, c1).is_err());
    }

    #[test]
    fn summary_poll_rejects_backwards_last_modified_time_rollbacks_for_unnamed_and_manual_chats() {
        #[derive(Default)]
        struct RecordedEvents(std::sync::Mutex<Vec<String>>);
        impl SessionEvents for RecordedEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
            fn updated(&self, event: &SessionUpdatedEvent) {
                self.0.lock().unwrap().push(event.session_id.clone());
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
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
        let stored = || {
            let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap();
            (row.title, row.live_title)
        };

        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, '', '0001-01-01 00:00:00+00:00')",
                [c1],
            )
            .unwrap();
        let mut sync = SummaryTitleSync::default();
        sync.poll(&request, &db_path, c1);
        assert_eq!(stored(), (None, None));

        // 1. Live title arrives at a newer timestamp; backwards rollbacks are rejected.
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "refactor", "2026-11-24 11:28:24.867079+00:00"],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(stored(), (None, Some("refactor".into())));
        assert_eq!(events.0.lock().unwrap().len(), 1);

        for (stale_title, stale_ts) in [
            ("refacotr", "2026-11-24 06:54:13.872081+00:00"),
            ("refacotr-zero", "0001-01-01 00:00:00+00:00"),
        ] {
            writer
                .execute(
                    "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                    [c1, stale_title, stale_ts],
                )
                .unwrap();
            sync.poll(&request, &db_path, c1);
            assert_eq!(stored(), (None, Some("refactor".into())), "{stale_title}");
            assert_eq!(events.0.lock().unwrap().len(), 1);
        }

        // 2. Manual Runner title survives both newer same-title turns and later rollbacks,
        // while a genuinely newer different summary title still replaces it (#1442).
        crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner C")).unwrap();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "refactor", "2026-11-24 11:30:00.1+00:00"],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(stored(), (Some("Runner C".into()), Some("refactor".into())));

        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "refacotr", "2026-11-24 11:29:00.000001+00:00"],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(stored(), (Some("Runner C".into()), Some("refactor".into())));

        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "Fresh D", "2026-11-24 11:30:00.2+00:00"],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(stored(), (None, Some("Fresh D".into())));
        assert_eq!(events.0.lock().unwrap().len(), 2);

        // 3. Resumed `for_launch` and `pending_baseline` both establish the watermark and baseline
        // DB title, reject backwards rollbacks, and keep launch baseline from replaying after a
        // `c1 -> c2 -> c1` round trip. Use a rollback-journal DB for the locked launch case
        // because WAL readers do not block on `BEGIN EXCLUSIVE`.
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        for locked in [false, true] {
            crate::repo::session::set_live_title(
                &pool.get().unwrap(),
                "s-agy",
                Some("Fresh D"),
                Some(&started_at),
            )
            .unwrap();
            crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner E"))
                .unwrap();
            let resume_db = temp.path().join(if locked {
                "resumed-locked.db"
            } else {
                "resumed-readable.db"
            });
            let resume_writer = rusqlite::Connection::open(&resume_db).unwrap();
            resume_writer
                .execute_batch(
                    "CREATE TABLE conversation_summaries (
                        conversation_id TEXT PRIMARY KEY,
                        title TEXT NOT NULL DEFAULT '',
                        last_modified_time TEXT NOT NULL
                    );",
                )
                .unwrap();
            resume_writer
                .execute(
                    "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                     VALUES (?1, ?2, ?3), (?4, 'Other C2', '2026-11-24 09:00:00+00:00')",
                    [c1, "Old Baseline A", "2026-11-24 11:30:00.2+00:00", c2],
                )
                .unwrap();
            let resumed_request = CaptureRequest {
                manager: Arc::downgrade(&manager),
                session_id: "s-agy".into(),
                mission_id: None,
                log_path: log_path(temp.path(), "s-agy"),
                summaries_db_path: Some(resume_db.clone()),
                cwd: Some("/tmp/my-project".into()),
                initial_key: Some(c1.into()),
                expected_row_started_at: started_at.clone(),
                stop: Arc::new(AtomicBool::new(false)),
                pool: Arc::clone(&pool),
                events: events.clone(),
            };
            let mut resumed_sync = if locked {
                resume_writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
                let sync = SummaryTitleSync::for_launch(&resumed_request);
                resume_writer.execute_batch("COMMIT").unwrap();
                assert!(sync
                    .conversations
                    .get(c1)
                    .is_some_and(|state| state.pending_baseline));
                assert_eq!(
                    sync.conversations
                        .get(c1)
                        .and_then(|state| state.highest_modified_at),
                    None
                );
                sync
            } else {
                let sync = SummaryTitleSync::for_launch(&resumed_request);
                assert!(!sync
                    .conversations
                    .get(c1)
                    .is_some_and(|state| state.pending_baseline));
                assert!(sync
                    .conversations
                    .get(c1)
                    .and_then(|state| state.highest_modified_at)
                    .is_some());
                sync
            };
            resumed_sync.poll(&resumed_request, &resume_db, c1);
            assert!(!resumed_sync
                .conversations
                .get(c1)
                .is_some_and(|state| state.pending_baseline));
            assert_eq!(
                resumed_sync
                    .conversations
                    .get(c1)
                    .and_then(|state| state.highest_modified_at),
                parse_summary_timestamp("2026-11-24 11:30:00.2+00:00")
            );
            assert_eq!(
                stored(),
                (Some("Runner E".into()), Some("Fresh D".into())),
                "locked={locked}"
            );
            resume_writer
                .execute(
                    "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                    [c1, "Rolled Back", "2026-11-24 11:00:00+00:00"],
                )
                .unwrap();
            resumed_sync.poll(&resumed_request, &resume_db, c1);
            assert_eq!(
                stored(),
                (Some("Runner E".into()), Some("Fresh D".into())),
                "locked={locked}"
            );

            // Round-trip c1 -> c2 -> c1 while c1 still holds launch baseline "Old Baseline A"
            // (even with a newer turn timestamp): restores "Fresh D" without applying baseline.
            resume_writer
                .execute(
                    "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                    [c1, "Old Baseline A", "2026-11-24 11:31:00+00:00"],
                )
                .unwrap();
            persist(&resumed_request, c2);
            resumed_sync.switch_conversation(Some(c1), c2);
            resumed_sync.poll(&resumed_request, &resume_db, c2);
            assert_eq!(stored(), (None, Some("Other C2".into())), "locked={locked}");
            persist(&resumed_request, c1);
            resumed_sync.switch_conversation(Some(c2), c1);
            resumed_sync.poll(&resumed_request, &resume_db, c1);
            assert_eq!(stored(), (None, Some("Fresh D".into())), "locked={locked}");
            crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner E"))
                .unwrap();
            resumed_sync.poll(&resumed_request, &resume_db, c1);
            assert_eq!(
                stored(),
                (Some("Runner E".into()), Some("Fresh D".into())),
                "locked={locked}"
            );
        }
    }

    #[test]
    fn summary_poll_coordinates_osc_floor_and_resets_tracking_on_conversation_switch() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        let mut row =
            crate::test_support::test_session_row("s-agy", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        row.agent_session_key = Some(c1.into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        manager
            .prepare_unlisted_terminal("s-agy", (80, 24), &pool, &events)
            .unwrap();
        let terminal = manager.terminal_model("s-agy").unwrap();

        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-agy".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-agy"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: Some(c1.into()),
            expected_row_started_at: started_at.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events),
        };
        let stored = || {
            let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-agy")
                .unwrap()
                .unwrap();
            (row.title, row.live_title, terminal.title())
        };

        // Live OSC 2 rename on c1 sets a freshness floor for c1.
        terminal.feed_output(1, b"\x1b]2;OSC Rename B\x07");
        terminal.flush_events();
        assert_eq!(
            stored(),
            (None, Some("OSC Rename B".into()), "OSC Rename B".into())
        );
        crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner C")).unwrap();

        let older_c1 = (Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c1, "Stale Summary A", &older_c1],
            )
            .unwrap();
        let mut sync = SummaryTitleSync::default();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (
                Some("Runner C".into()),
                Some("OSC Rename B".into()),
                "OSC Rename B".into()
            )
        );

        // Even a direct `apply_provider_title` call for c1 with an older timestamp is rejected
        // on the terminal event thread without touching SQLite or TerminalModel.
        assert!(!persist_title(
            &request,
            c1,
            "Stale Summary A",
            parse_summary_timestamp(&older_c1),
            sync.osc_tracker.as_ref(),
        ));
        assert_eq!(
            stored(),
            (
                Some("Runner C".into()),
                Some("OSC Rename B".into()),
                "OSC Rename B".into()
            )
        );

        // A genuinely newer summary for c1 still updates live_title, TerminalModel, and clears
        // the manual Runner title (#1442).
        let newer_c1 = (Utc::now() + chrono::Duration::minutes(1))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "Newer Summary D", &newer_c1],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (
                None,
                Some("Newer Summary D".into()),
                "Newer Summary D".into()
            )
        );

        // Switching to an older conversation c2 clears c1's active dedup and adopts c2.
        persist(&request, c2);
        sync.switch_conversation(Some(c1), c2);
        let older_c2 = (Utc::now() - chrono::Duration::hours(24))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c2, "Older C2 Topic", &older_c2],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            stored(),
            (None, Some("Older C2 Topic".into()), "Older C2 Topic".into())
        );

        // Once c2 is adopted, an even older rollback on c2 is rejected.
        let stale_c2 = (Utc::now() - chrono::Duration::hours(48))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c2, "Stale C2 Topic", &stale_c2],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            stored(),
            (None, Some("Older C2 Topic".into()), "Older C2 Topic".into())
        );

        // 1. Unchanged DB title whose timestamp advances on a normal turn after an OSC + manual
        // rename (A@10 -> OSC B@30 -> manual C -> A@40, with no intervening DB "B") must keep the
        // OSC/manual names intact with zero DB writes.
        let pool_conn = pool.get().unwrap();
        pool_conn
            .execute_batch(
                "CREATE TABLE title_writes (title TEXT);
                 CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
                 BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
            )
            .unwrap();
        drop(pool_conn);
        let write_count = || -> usize {
            pool.get()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
                .unwrap()
        };
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c2, "Older C2 Topic", &older_c2],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(write_count(), 0);

        terminal.feed_output(2, b"\x1b]2;OSC Intervening B\x07");
        terminal.flush_events();
        assert_eq!(
            stored(),
            (
                None,
                Some("OSC Intervening B".into()),
                "OSC Intervening B".into()
            )
        );
        let writes_after_osc = write_count();
        assert_eq!(writes_after_osc, 1);
        crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner C2")).unwrap();

        let t_unchanged_bump = (Utc::now() + chrono::Duration::minutes(1))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c2, "Older C2 Topic", &t_unchanged_bump],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            stored(),
            (
                Some("Runner C2".into()),
                Some("OSC Intervening B".into()),
                "OSC Intervening B".into()
            )
        );
        assert_eq!(write_count(), writes_after_osc);

        // 2. When the DB genuinely flushes the OSC title (even slightly before OSC's observed_at)
        // and later changes back to a prior title (A@10 -> OSC B@30 -> DB B@29 -> manual C ->
        // DB A@40), the later A@40 is recognized as a real DB title change and clears manual C.
        let c3 = "4c3b1d84-5c5c-44aa-8f4d-3e84b1bc0e90";
        persist(&request, c3);
        sync.switch_conversation(Some(c2), c3);
        let t_c3_a10 = (Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c3, "Prior C3 A", &t_c3_a10],
            )
            .unwrap();
        sync.poll(&request, &db_path, c3);
        assert_eq!(
            stored(),
            (None, Some("Prior C3 A".into()), "Prior C3 A".into())
        );

        terminal.feed_output(3, b"\x1b]2;OSC C3 B\x07");
        terminal.flush_events();
        assert_eq!(stored(), (None, Some("OSC C3 B".into()), "OSC C3 B".into()));
        let writes_after_c3_osc = write_count();

        let t_c3_b29 = (Utc::now() - chrono::Duration::minutes(1))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c3, "OSC C3 B", &t_c3_b29],
            )
            .unwrap();
        sync.poll(&request, &db_path, c3);
        assert_eq!(write_count(), writes_after_c3_osc);
        crate::repo::session::set_title(&pool.get().unwrap(), "s-agy", Some("Runner C3")).unwrap();

        let t_c3_a40 = (Utc::now() + chrono::Duration::minutes(1))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c3, "Prior C3 A", &t_c3_a40],
            )
            .unwrap();
        sync.poll(&request, &db_path, c3);
        assert_eq!(
            stored(),
            (None, Some("Prior C3 A".into()), "Prior C3 A".into())
        );
        let writes_after_c3_a40 = write_count();
        assert_eq!(writes_after_c3_a40, writes_after_c3_osc + 1);
        sync.poll(&request, &db_path, c3);
        assert_eq!(write_count(), writes_after_c3_a40);

        manager.drain_terminal_workers();
    }

    #[test]
    fn early_osc_title_before_first_key_binds_to_first_conversation_and_clears_on_switch() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        let mut row =
            crate::test_support::test_session_row("s-early", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        manager
            .prepare_unlisted_terminal("s-early", (80, 24), &pool, &events)
            .unwrap();
        let terminal = manager.terminal_model("s-early").unwrap();

        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-early".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-early"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: None,
            expected_row_started_at: started_at.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events),
        };

        // OSC 2 arrives before c1 is captured (`agent_session_key` is still NULL).
        terminal.feed_output(1, b"\x1b]2;Early OSC Title\x07");
        terminal.flush_events();
        assert_eq!(terminal.title(), "Early OSC Title");

        // Capturing c1 binds that OSC floor to c1 so an older c1 summary cannot revert it.
        persist(&request, c1);
        let older_c1 = (Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c1, "Stale Early C1", &older_c1],
            )
            .unwrap();
        let mut sync = SummaryTitleSync::default();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-early")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Early OSC Title")
        );
        assert_eq!(terminal.title(), "Early OSC Title");

        // Switching to c2 clears c1's bound OSC floor so an older c2 summary is adopted.
        persist(&request, c2);
        let older_c2 = (Utc::now() - chrono::Duration::hours(24))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c2, "Older C2 After Early OSC", &older_c2],
            )
            .unwrap();
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-early")
                .unwrap()
                .unwrap()
                .live_title
                .as_deref(),
            Some("Older C2 After Early OSC")
        );
        assert_eq!(terminal.title(), "Older C2 After Early OSC");

        manager.drain_terminal_workers();
    }

    #[test]
    fn repeated_osc_title_and_failed_osc_db_write_still_record_freshness_floor() {
        #[derive(Default)]
        struct RecordedEvents(std::sync::Mutex<Vec<String>>);
        impl SessionEvents for RecordedEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
            fn updated(&self, event: &SessionUpdatedEvent) {
                self.0.lock().unwrap().push(event.session_id.clone());
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TABLE title_writes (title TEXT);
                 CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
                 BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
            )
            .unwrap();
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events = Arc::new(RecordedEvents::default());
        let events_dyn: Arc<dyn SessionEvents> = events.clone();
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let mut row =
            crate::test_support::test_session_row("s-repeat", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        row.agent_session_key = Some(c1.into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-repeat".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-repeat"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: Some(c1.into()),
            expected_row_started_at: started_at,
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events_dyn),
        };
        let mut sync = SummaryTitleSync::for_launch(&request);
        manager
            .prepare_unlisted_terminal("s-repeat", (80, 24), &pool, &events_dyn)
            .unwrap();
        let terminal = manager.terminal_model("s-repeat").unwrap();

        let format_ts = |dt: DateTime<Utc>| dt.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string();
        let t10 = format_ts(Utc::now() - chrono::Duration::minutes(20));
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c1, "Title B", &t10],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(terminal.title(), "Title B");
        assert_eq!(events.0.lock().unwrap().len(), 1);
        crate::repo::session::set_title(&pool.get().unwrap(), "s-repeat", Some("Runner C"))
            .unwrap();

        let write_count = || -> usize {
            pool.get()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(write_count(), 1);

        // Identical OSC 2 bytes for "Title B" arrive at t=30: DB writes and notifications
        // remain deduplicated, manual Runner C stays intact, and the OSC floor advances to ~now.
        terminal.feed_output(1, b"\x1b]2;Title B\x07");
        terminal.flush_events();
        assert_eq!(write_count(), 1);
        assert_eq!(events.0.lock().unwrap().len(), 1);

        // Reconciliation exposes A@20 (newer than summary B@10, older than OSC B@30): rejected.
        let t20 = format_ts(Utc::now() - chrono::Duration::minutes(10));
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "Stale A", &t20],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-repeat")
            .unwrap()
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("Runner C"));
        assert_eq!(row.live_title.as_deref(), Some("Title B"));
        assert_eq!(terminal.title(), "Title B");
        assert_eq!(write_count(), 1);

        // Even when the OSC DB write itself fails transiently, observe_live_title still records
        // the freshness floor so an older summary row cannot overwrite it.
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_live_title BEFORE UPDATE OF live_title ON sessions
                 BEGIN SELECT RAISE(ABORT, 'simulated db error'); END;",
            )
            .unwrap();
        terminal.feed_output(2, b"\x1b]2;Transient OSC\x07");
        terminal.flush_events();
        pool.get()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_live_title;")
            .unwrap();
        let t25 = format_ts(Utc::now() - chrono::Duration::minutes(5));
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "Stale After Failed OSC", &t25],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-repeat")
            .unwrap()
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("Runner C"));
        assert_eq!(row.live_title.as_deref(), Some("Title B"));
        assert_eq!(terminal.title(), "Transient OSC");

        manager.drain_terminal_workers();
    }

    #[test]
    fn osc_title_after_logged_switch_before_capture_pass_protects_new_conversation() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        let mut row =
            crate::test_support::test_session_row("s-switch", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        row.agent_session_key = Some(c1.into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        prepare_log(temp.path(), "s-switch");
        let session_log = log_path(temp.path(), "s-switch");
        std::fs::write(
            &session_log,
            format!(
                "I0928 22:00:00.000000 1 conversation_manager.go:887] Streaming conversation {c1}\n"
            ),
        )
        .unwrap();

        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-switch".into(),
            mission_id: None,
            log_path: session_log.clone(),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: Some(c1.into()),
            expected_row_started_at: started_at,
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events),
        };
        let mut sync = SummaryTitleSync::for_launch(&request);
        manager
            .prepare_unlisted_terminal("s-switch", (80, 24), &pool, &events)
            .unwrap();
        let terminal = manager.terminal_model("s-switch").unwrap();

        // agy logs a switch to c2 and emits c2's OSC rename before the 400ms capture pass runs
        // (so sessions.agent_session_key in SQLite still reads c1).
        std::fs::OpenOptions::new()
            .append(true)
            .open(&session_log)
            .unwrap()
            .write_all(
                format!(
                    "I0928 22:01:00.000000 1 conversation_manager.go:887] Streaming conversation {c2}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        terminal.feed_output(1, b"\x1b]2;Rename C2\x07");
        terminal.flush_events();
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), "s-switch")
                .unwrap()
                .unwrap()
                .agent_session_key
                .as_deref(),
            Some(c1)
        );

        // An outstanding c1 summary update (with a valid newer c1 timestamp or even a legacy
        // row lacking last_modified_time) running after c2's OSC and before capture catches up
        // must be rejected without replacing c2's title or clearing a manual Runner title.
        crate::repo::session::set_title(&pool.get().unwrap(), "s-switch", Some("Runner C2"))
            .unwrap();
        let newer_c1 = (Utc::now() + chrono::Duration::minutes(5))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c1, "Late C1 Summary", &newer_c1],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert!(!persist_title(
            &request,
            c1,
            "Late C1 Timestamped",
            parse_summary_timestamp(&newer_c1),
            sync.osc_tracker.as_ref(),
        ));
        assert!(!persist_title(
            &request,
            c1,
            "Late C1 Legacy",
            None,
            sync.osc_tracker.as_ref(),
        ));
        let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-switch")
            .unwrap()
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("Runner C2"));
        assert_eq!(row.live_title.as_deref(), Some("Rename C2"));
        assert_eq!(terminal.title(), "Rename C2");

        // Now the capture pass catches up with c2 and polls c2's older summary row.
        let older_c2 = (Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%d %H:%M:%S%.6f+00:00")
            .to_string();
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c2, "Stale Summary C2", &older_c2],
            )
            .unwrap();
        persist(&request, c2);
        sync.switch_conversation(Some(c1), c2);
        sync.poll(&request, &db_path, c2);
        let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-switch")
            .unwrap()
            .unwrap();
        assert_eq!(row.agent_session_key.as_deref(), Some(c2));
        assert_eq!(row.title.as_deref(), Some("Runner C2"));
        assert_eq!(row.live_title.as_deref(), Some("Rename C2"));
        assert_eq!(terminal.title(), "Rename C2");

        manager.drain_terminal_workers();
    }

    #[test]
    fn final_poll_after_terminal_release_and_state_prune_retains_osc_protection() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let mut row =
            crate::test_support::test_session_row("s-exit", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        row.agent_session_key = Some(c1.into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-exit".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-exit"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: Some(c1.into()),
            expected_row_started_at: started_at,
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events),
        };
        let mut sync = SummaryTitleSync::for_launch(&request);
        manager
            .prepare_unlisted_terminal("s-exit", (80, 24), &pool, &events)
            .unwrap();
        let terminal = manager.terminal_model("s-exit").unwrap();

        let format_ts = |dt: DateTime<Utc>| dt.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string();
        let t10 = format_ts(Utc::now() - chrono::Duration::minutes(20));
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, ?2, ?3)",
                [c1, "Turn One", &t10],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);

        terminal.feed_output(1, b"\x1b]2;Live OSC B\x07");
        terminal.flush_events();
        crate::repo::session::set_title(&pool.get().unwrap(), "s-exit", Some("Runner C")).unwrap();

        // Simulate session exit: terminal workers stop/release, state is removed/pruned,
        // and the session row transitions to stopped with the same started_at generation.
        drop(terminal);
        manager.drain_terminal_workers();
        manager.discard_unlisted_terminal("s-exit");
        crate::repo::session::set_exit_status(
            &pool.get().unwrap(),
            "s-exit",
            crate::model::SessionStatus::Stopped,
            Utc::now(),
        )
        .unwrap();

        // Stale row is newer than Turn One (t10) but older than Live OSC B (~now).
        // The final post-stop poll must still reject it.
        let t20 = format_ts(Utc::now() - chrono::Duration::minutes(10));
        writer
            .execute(
                "UPDATE conversation_summaries SET title = ?2, last_modified_time = ?3 WHERE conversation_id = ?1",
                [c1, "Post-Exit Stale A", &t20],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-exit")
            .unwrap()
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("Runner C"));
        assert_eq!(row.live_title.as_deref(), Some("Live OSC B"));
    }

    #[test]
    fn returning_to_earlier_conversation_keeps_watermark_and_rejects_rollback() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TABLE title_writes (title TEXT);
                 CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
                 BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
            )
            .unwrap();
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let c2 = "9d79746d-9af0-41a2-83b4-45edfb6ed263";
        let mut row =
            crate::test_support::test_session_row("s-return", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-return".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-return"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: None,
            expected_row_started_at: started_at,
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events: Arc::clone(&events),
        };
        let mut sync = SummaryTitleSync::for_launch(&request);
        manager
            .prepare_unlisted_terminal("s-return", (80, 24), &pool, &events)
            .unwrap();
        let terminal = manager.terminal_model("s-return").unwrap();
        let stored = || {
            let row = crate::repo::session::get_row(&pool.get().unwrap(), "s-return")
                .unwrap()
                .unwrap();
            (row.title, row.live_title, terminal.title())
        };
        let written_titles = || -> Vec<Option<String>> {
            let conn = pool.get().unwrap();
            let mut stmt = conn.prepare("SELECT title FROM title_writes").unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };

        let format_ts = |dt: DateTime<Utc>| dt.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string();
        let t_c1 = format_ts(Utc::now() - chrono::Duration::minutes(20));
        let t_c2 = format_ts(Utc::now() - chrono::Duration::hours(24));
        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, 'C1 Current B', ?3),
                        (?2, 'C2 Older Topic', ?4)",
                [c1, c2, &t_c1, &t_c2],
            )
            .unwrap();

        persist(&request, c1);
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (None, Some("C1 Current B".into()), "C1 Current B".into())
        );

        // Switch c1 -> c2: older c2 summary is adopted.
        persist(&request, c2);
        sync.switch_conversation(Some(c1), c2);
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            stored(),
            (None, Some("C2 Older Topic".into()), "C2 Older Topic".into())
        );

        // c1 rolls back while away; returning to c1 restores c1's known good name once
        // without writing the stale row.
        let t_c1_stale = format_ts(Utc::now() - chrono::Duration::hours(5));
        writer
            .execute(
                "UPDATE conversation_summaries SET title = 'C1 Stale A', last_modified_time = ?2
                 WHERE conversation_id = ?1",
                [c1, &t_c1_stale],
            )
            .unwrap();
        persist(&request, c1);
        sync.switch_conversation(Some(c2), c1);
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (None, Some("C1 Current B".into()), "C1 Current B".into())
        );
        assert!(!written_titles()
            .iter()
            .any(|title| title.as_deref() == Some("C1 Stale A")));

        // Staying on c1 after restore keeps manual Runner names safe from repeated stale polls.
        crate::repo::session::set_title(&pool.get().unwrap(), "s-return", Some("Runner C"))
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (
                Some("Runner C".into()),
                Some("C1 Current B".into()),
                "C1 Current B".into()
            )
        );

        // Without any DB rollback, if c1 received an OSC rename newer than its stored summary
        // row, switching c1 -> c2 -> c1 restores c1's last trusted OSC title once.
        terminal.feed_output(1, b"\x1b]2;C1 Live OSC\x07");
        terminal.flush_events();
        assert_eq!(
            stored(),
            (None, Some("C1 Live OSC".into()), "C1 Live OSC".into())
        );
        persist(&request, c2);
        sync.switch_conversation(Some(c1), c2);
        sync.poll(&request, &db_path, c2);
        assert_eq!(
            stored(),
            (None, Some("C2 Older Topic".into()), "C2 Older Topic".into())
        );
        persist(&request, c1);
        sync.switch_conversation(Some(c2), c1);
        sync.poll(&request, &db_path, c1);
        assert_eq!(
            stored(),
            (None, Some("C1 Live OSC".into()), "C1 Live OSC".into())
        );
        assert!(!written_titles()
            .iter()
            .any(|title| title.as_deref() == Some("C1 Stale A")));

        manager.drain_terminal_workers();
    }

    #[test]
    fn transient_write_failure_still_advances_watermark_and_allows_same_timestamp_retry() {
        struct NoEvents;
        impl SessionEvents for NoEvents {
            fn exit(&self, _: &crate::session::manager::ExitEvent) {}
        }

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("conversation_summaries.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute_batch(
                "CREATE TABLE conversation_summaries (
                    conversation_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    last_modified_time TEXT NOT NULL
                );",
            )
            .unwrap();

        let pool = Arc::new(crate::db::open_in_memory().unwrap());
        let manager = crate::session::manager::SessionManager::key_test_manager();
        let events: Arc<dyn SessionEvents> = Arc::new(NoEvents);
        let c1 = "27499127-777f-4aad-9e04-7c9208bfd9b6";
        let mut row =
            crate::test_support::test_session_row("s-retry", crate::model::SessionStatus::Running);
        row.agent_runtime = Some("antigravity".into());
        row.cwd = Some("/tmp/my-project".into());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let request = CaptureRequest {
            manager: Arc::downgrade(&manager),
            session_id: "s-retry".into(),
            mission_id: None,
            log_path: log_path(temp.path(), "s-retry"),
            summaries_db_path: Some(db_path.clone()),
            cwd: Some("/tmp/my-project".into()),
            initial_key: None,
            expected_row_started_at: started_at,
            stop: Arc::new(AtomicBool::new(false)),
            pool: Arc::clone(&pool),
            events,
        };
        let live_title = || {
            crate::repo::session::get_row(&pool.get().unwrap(), "s-retry")
                .unwrap()
                .unwrap()
                .live_title
        };

        writer
            .execute(
                "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time)
                 VALUES (?1, 'Initial A', '2026-11-24 10:00:00+00:00')",
                [c1],
            )
            .unwrap();
        let mut sync = SummaryTitleSync::for_launch(&request);
        sync.poll(&request, &db_path, c1);
        assert_eq!(live_title().as_deref(), Some("Initial A"));

        // Fail the next live_title update transiently when New B@30 is read.
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_once BEFORE UPDATE OF live_title ON sessions
                 WHEN NEW.live_title = 'New B'
                 BEGIN SELECT RAISE(ABORT, 'transient write failure'); END;",
            )
            .unwrap();
        writer
            .execute(
                "UPDATE conversation_summaries SET title = 'New B', last_modified_time = '2026-11-24 12:00:00+00:00'
                 WHERE conversation_id = ?1",
                [c1],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(live_title().as_deref(), Some("Initial A"));

        pool.get()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_once;")
            .unwrap();

        // A rollback to 11:00 (newer than 10:00, older than 12:00) must be rejected because
        // the watermark already advanced to 12:00 on read.
        writer
            .execute(
                "UPDATE conversation_summaries SET title = 'Rollback Mid', last_modified_time = '2026-11-24 11:00:00+00:00'
                 WHERE conversation_id = ?1",
                [c1],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(live_title().as_deref(), Some("Initial A"));

        // Retrying New B at the same timestamp (12:00) succeeds because `observed_db_title`
        // was not marked applied when the first write failed.
        writer
            .execute(
                "UPDATE conversation_summaries SET title = 'New B', last_modified_time = '2026-11-24 12:00:00+00:00'
                 WHERE conversation_id = ?1",
                [c1],
            )
            .unwrap();
        sync.poll(&request, &db_path, c1);
        assert_eq!(live_title().as_deref(), Some("New B"));
    }
}
