//! Terminal fixture corpus: recorded PTY byte logs replayed into an
//! `alacritty_terminal` grid (impl 0031 Phase 1; the harness spec 42
//! promised).
//!
//! Format: NDJSON. First line is a `FixtureHeader`, every following
//! line a `FixtureEvent`. Output bytes are base64 so raw escape
//! sequences survive JSON and git diffs.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context as _};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

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
    /// Bytes we wrote to the PTY (kept for provenance, not replayed).
    Input { ms: u64, input: String },
    /// Child exit, if observed before the recording window closed.
    Exit { ms: u64, exit: i32 },
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
