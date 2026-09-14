use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;

use super::launch::shell_quote;
use crate::error::{Error, Result};

const STATUS_DIR: &str = "session-status";

pub(crate) const fn hooks_supported(windows: bool) -> bool {
    !windows
}

pub(crate) fn status_path(app_data_dir: &Path, session_id: &str) -> PathBuf {
    app_data_dir
        .join(STATUS_DIR)
        .join(format!("{session_id}.ndjson"))
}

pub(crate) fn clear_leftovers(app_data_dir: &Path) -> Result<()> {
    let entries = match fs::read_dir(app_data_dir.join(STATUS_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                log::warn!("read stale agent status entry: {error}");
                continue;
            }
        };
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ndjson" | "sh" | "tmp")
        ) || path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(".ndjson."))
        {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("remove stale agent status file {}: {error}", path.display());
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn hook_command(path: &Path, event: &str) -> String {
    // A bounded append names the payload so parallel, large hooks cannot interleave JSON.
    format!(
        "(sh {} {} {} || cat >/dev/null) 2>/dev/null; exit 0",
        shell_quote(&script_path(path).to_string_lossy()),
        shell_quote(&path.to_string_lossy()),
        shell_quote(event),
    )
}

pub(super) fn script_path(path: &Path) -> PathBuf {
    path.with_extension("sh")
}

struct StatusFiles(PathBuf);

impl Drop for StatusFiles {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                let prefix = format!("{}.", self.0.file_name().unwrap().to_string_lossy());
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with(&prefix) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
        for path in [
            script_path(&self.0),
            self.0.with_extension("sh.tmp"),
            self.0.clone(),
        ] {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("remove agent status file {}: {error}", path.display());
                }
            }
        }
    }
}

pub(super) struct HookFeed {
    path: PathBuf,
    reader: BufReader<File>,
    pub(super) pending: Vec<u8>,
    generation: String,
    pub(super) dirty: Arc<AtomicBool>,
    last_read: Instant,
    _watcher: RecommendedWatcher,
    _files: StatusFiles,
}

impl HookFeed {
    pub(super) fn start(path: &Path, generation: String, script_body: &str) -> Result<Self> {
        fs::create_dir_all(path.parent().expect("status file has a parent"))?;
        let files = StatusFiles(path.to_owned());
        let script = script_path(path);
        let temporary = path.with_extension("sh.tmp");
        fs::write(&temporary, script_body)?;
        fs::rename(temporary, script)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_for_watch = Arc::clone(&dirty);
        let mut watcher = notify::recommended_watcher(move |event| {
            if let Err(error) = event {
                log::warn!("agent status notify: {error}");
            }
            dirty_for_watch.store(true, Ordering::Release);
        })
        .map_err(|error| Error::msg(format!("agent status watcher: {error}")))?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|error| {
                Error::msg(format!("watch agent status {}: {error}", path.display()))
            })?;
        Ok(Self {
            path: path.to_owned(),
            reader: BufReader::new(file),
            pending: Vec::new(),
            generation,
            dirty,
            last_read: Instant::now(),
            _watcher: watcher,
            _files: files,
        })
    }

    pub(super) fn drain(&mut self, force: bool, mut observe: impl FnMut(Value)) -> Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel)
            && self.last_read.elapsed() < Duration::from_secs(1)
            && !force
        {
            return Ok(());
        }
        if !self.path.exists() || !script_path(&self.path).exists() {
            return Err(Error::msg("agent status bridge unavailable"));
        }
        self.last_read = Instant::now();
        while self.reader.read_until(b'\n', &mut self.pending)? != 0 {
            if self.pending.last() != Some(&b'\n') {
                break;
            }
            if let Some(report) = self.read_report() {
                observe(report);
            }
            self.pending.clear();
        }
        Ok(())
    }

    fn read_report(&self) -> Option<Value> {
        let mut report: Value = serde_json::from_slice(&self.pending).ok()?;
        if report.get("generation")?.as_str()? != self.generation {
            return None;
        }
        if let Some(file) = report.get("payload_file").and_then(Value::as_str) {
            let prefix = format!("{}.", self.path.file_name()?.to_string_lossy());
            if !file.starts_with(&prefix) || file.contains(['/', '\\']) {
                return None;
            }
            let event = report.get("hook_event_name")?.as_str()?.to_owned();
            let path = self.path.with_file_name(file);
            let payload = fs::read(&path);
            let _ = fs::remove_file(path);
            report = serde_json::from_slice(&payload.ok()?).ok()?;
            let object = report.as_object_mut()?;
            let reported_event = object
                .entry("hook_event_name")
                .or_insert(Value::String(event.clone()));
            if reported_event.as_str() == Some("") {
                *reported_event = Value::String(event.clone());
            }
            if reported_event.as_str() != Some(&event) {
                return None;
            }
        }
        Some(report)
    }
}

pub(super) struct TranscriptTail {
    pub(super) path: PathBuf,
    pub(super) reader: BufReader<File>,
    pub(super) pending: Vec<u8>,
}

impl TranscriptTail {
    pub(super) fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(1024 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(file);
        if start > 0 {
            reader.read_until(b'\n', &mut Vec::new())?;
        }
        Ok(Self {
            path: path.to_owned(),
            reader,
            pending: Vec::new(),
        })
    }
}
