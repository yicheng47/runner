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

pub(crate) struct IdentityWatcher(pub(crate) Box<dyn HookWatcher>);

impl HookWatcher for IdentityWatcher {
    fn spawned(&mut self, pid: u32) {
        self.0.spawned(pid);
    }
    fn drain_events(
        &mut self,
        cancel: u8,
        _emit: &mut dyn FnMut(
            super::state::agent::AgentEvent,
        ) -> super::state::agent::AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> Result<()> {
        self.0
            .drain_events(cancel, &mut |_| Default::default(), session_start)
    }
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

    pub(crate) fn is_retired(&self) -> bool {
        self.0.is_retired()
    }

    pub(crate) fn drain(&mut self, mut observe: impl FnMut(Value)) -> Result<()> {
        self.drain_admitted(|report, _| observe(report))
    }

    pub(crate) fn drain_admitted(&mut self, mut observe: impl FnMut(Value, i64)) -> Result<()> {
        self.0.drain_admitted(|mut report, admitted_at| {
            report.payload["hook_event_name"] = Value::String(report.event);
            observe(report.payload, admitted_at);
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
                        Runtime::ClaudeCode
                            | Runtime::Codex
                            | Runtime::Copilot
                            | Runtime::Pi
                            | Runtime::OpenCode
                    ) || (matches!(runtime, Runtime::Cursor | Runtime::Grok) && !windows),
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

#[cfg(test)]
#[test]
fn identity_only_watcher_rekeys_without_publishing_status() {
    let (hooks, receiver) =
        super::hook_queue::TestHookRoute::new(crate::model::Runtime::ClaudeCode, "current".into());
    let watcher =
        crate::runtimes::claude_code::claude_status::ClaudeStatusWatcher::from_receiver(receiver);
    let mut watcher = IdentityWatcher(Box::new(watcher));
    hooks
        .admit_event(
            "SessionStart",
            serde_json::json!({"session_id":"eec53725-8f5d-428a-8afb-46510ddfb000"}),
        )
        .unwrap();
    let mut status_events = 0;
    let mut keys = Vec::new();
    watcher
        .drain_events(
            0,
            &mut |_| {
                status_events += 1;
                Default::default()
            },
            &mut |key| keys.push(key),
        )
        .unwrap();
    assert_eq!(keys, ["eec53725-8f5d-428a-8afb-46510ddfb000"]);
    assert_eq!(status_events, 0);
}

#[cfg(test)]
#[test]
fn identity_only_copilot_and_grok_watchers_follow_conversation_changes() {
    for runtime in [crate::model::Runtime::Copilot, crate::model::Runtime::Grok]
        .into_iter()
        .filter(|runtime| *runtime != crate::model::Runtime::Grok || !cfg!(windows))
    {
        let root = tempfile::tempdir().unwrap();
        let (hooks, receiver) = super::hook_queue::TestHookRoute::new(runtime, "current".into());
        let spec = super::runtime::SpawnSpec {
            env: std::collections::BTreeMap::from([(
                "COPILOT_HOME".into(),
                root.path().display().to_string(),
            )]),
            ..Default::default()
        };
        let watcher = crate::runtimes::adapter(runtime)
            .status_hooks()
            .unwrap()
            .start_receiver(&spec, receiver)
            .unwrap();
        let mut watcher = IdentityWatcher(watcher);
        let mut keys = Vec::new();
        let mut statuses = 0;
        let second = "22222222-2222-4222-8222-222222222222";
        for key in ["11111111-1111-4111-8111-111111111111", second] {
            hooks
                .admit_event("SessionStart", serde_json::json!({"sessionId":key}))
                .unwrap();
            watcher
                .drain_events(
                    0,
                    &mut |_| {
                        statuses += 1;
                        Default::default()
                    },
                    &mut |key| keys.push(key),
                )
                .unwrap();
        }
        assert_eq!(keys.last().map(String::as_str), Some(second), "{runtime}");
        assert_eq!(statuses, 0);
    }
}
