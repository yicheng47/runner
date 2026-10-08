use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::Result;

const STATUS_DIR: &str = "session-status";

pub trait HookWatcher: Send {
    fn spawned(&mut self, _pid: u32) {}

    fn drain_events(
        &mut self,
        cancel: u8,
        emit: &mut dyn FnMut(
            super::state::agent::AgentEvent,
        ) -> super::state::agent::AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> Result<()>;
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
            Some("ndjson" | "sh" | "ps1" | "tmp")
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

pub(crate) struct HookFeed(super::hook_queue::HookReceiver);

impl HookFeed {
    pub(crate) fn from_receiver(receiver: super::hook_queue::HookReceiver) -> Self {
        Self(receiver)
    }

    pub(crate) fn drain(&mut self, mut observe: impl FnMut(Value)) -> Result<()> {
        self.0.drain(|mut report| {
            report.payload["hook_event_name"] = Value::String(report.event);
            observe(report.payload);
        })
    }
}

pub(crate) struct TranscriptTail {
    pub(crate) path: PathBuf,
    pub(crate) reader: BufReader<File>,
    pub(crate) pending: Vec<u8>,
}

impl TranscriptTail {
    pub(crate) fn open(path: &Path) -> std::io::Result<Self> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Runtime;

    #[test]
    fn hooks_are_gated_per_runtime() {
        for windows in [false, true] {
            for runtime in Runtime::ALL {
                assert_eq!(
                    crate::runtimes::adapter(runtime)
                        .status_hooks()
                        .is_some_and(|hooks| hooks.supported(windows)),
                    matches!(
                        runtime,
                        Runtime::ClaudeCode | Runtime::Codex | Runtime::Copilot | Runtime::Pi
                    ) || (matches!(runtime, Runtime::Antigravity | Runtime::Cursor) && !windows),
                    "{runtime:?} windows={windows}"
                );
            }
            assert!(!crate::runtimes::for_key("")
                .status_hooks()
                .is_some_and(|hooks| hooks.supported(windows)));
        }
    }

    #[test]
    fn startup_clears_powershell_reporters_left_by_a_crash() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(STATUS_DIR).join("stale.ndjson");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path.with_extension("ps1"), "exit 0").unwrap();
        fs::write(path.with_extension("ps1.tmp"), "exit 0").unwrap();
        clear_leftovers(root.path()).unwrap();
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
    }
}
