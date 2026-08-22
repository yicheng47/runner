//! Terminal fixture corpus: recorded PTY byte logs replayed into an
//! `alacritty_terminal` grid (impl 0031 Phase 1; the harness spec 42
//! promised).
//!
//! Format: NDJSON. First line is a `FixtureHeader`, every following
//! line a `FixtureEvent`. Output bytes are base64 so raw escape
//! sequences survive JSON and git diffs.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{bail, Context as _};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::input_state::InputEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureHeader {
    pub v: u32,
    pub cols: u16,
    pub rows: u16,
    pub command: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FixtureEvent {
    /// PTY output chunk, base64-encoded, `ms` since spawn.
    Data { ms: u64, data: String },
    /// Human input observed by the native terminal tracker.
    Input { ms: u64, input: FixtureInput },
    /// Child exit, if observed before the recording window closed.
    Exit { ms: u64, exit: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FixtureInput {
    Event(InputEvent),
    LegacyBytes(String),
}

#[derive(Debug, Clone)]
pub struct Fixture {
    pub header: FixtureHeader,
    pub events: Vec<FixtureEvent>,
}

impl Fixture {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw =
            fs::read_to_string(path).with_context(|| format!("read fixture {}", path.display()))?;
        let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
        let header_line = match lines.next() {
            Some(l) => l,
            None => bail!("fixture {} is empty", path.display()),
        };
        let header: FixtureHeader =
            serde_json::from_str(header_line).context("parse fixture header")?;
        if header.v != 1 {
            bail!("unsupported fixture version {}", header.v);
        }
        let events = lines
            .map(|l| serde_json::from_str::<FixtureEvent>(l).context("parse fixture event"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self { header, events })
    }

    /// All PTY output bytes, in order, timing stripped.
    pub fn output_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        for ev in &self.events {
            if let FixtureEvent::Data { data, .. } = ev {
                out.extend(B64.decode(data).context("decode fixture data chunk")?);
            }
        }
        Ok(out)
    }
}

pub fn encode_chunk(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn decode_chunk(data: &str) -> anyhow::Result<Vec<u8>> {
    B64.decode(data).context("decode fixture data chunk")
}

pub struct FixtureRecorder {
    started: Instant,
    file: Mutex<File>,
}

impl FixtureRecorder {
    pub fn from_env(session_id: &str, cols: u16, rows: u16) -> anyhow::Result<Option<Self>> {
        let Some(path) = std::env::var_os("RUNNER_RECORD_INPUT_FIXTURE") else {
            return Ok(None);
        };
        let mut output_path = path;
        output_path.push(format!(".{session_id}.ndjson"));
        let path = PathBuf::from(output_path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("create input fixture {}", path.display()))?;
        let header = FixtureHeader {
            v: 1,
            cols,
            rows,
            command: session_id.to_owned(),
            args: Vec::new(),
            note: Some("recorded by TerminalSession".into()),
        };
        serde_json::to_writer(&mut file, &header).context("serialize input fixture header")?;
        file.write_all(b"\n")
            .context("write input fixture header")?;
        Ok(Some(Self {
            started: Instant::now(),
            file: Mutex::new(file),
        }))
    }

    pub fn record_output(&self, bytes: &[u8]) {
        self.record(&FixtureEvent::Data {
            ms: self.elapsed_ms(),
            data: encode_chunk(bytes),
        });
    }

    pub fn record_input(&self, input: &InputEvent) {
        self.record(&FixtureEvent::Input {
            ms: self.elapsed_ms(),
            input: FixtureInput::Event(input.clone()),
        });
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn record(&self, event: &FixtureEvent) {
        let mut file = self.file.lock().unwrap();
        if serde_json::to_writer(&mut *file, event).is_ok() {
            let _ = file.write_all(b"\n");
        }
    }
}
