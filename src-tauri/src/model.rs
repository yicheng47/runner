// SQLite row types for the app binary.
//
// The on-the-wire event envelope (Event, EventKind, SignalType, EventDraft) lives
// in `runners-core` so the standalone CLI can reuse it without pulling in
// rusqlite. Those are re-exported at the bottom of this file for backward-
// compatible imports across the app code.

#![allow(dead_code, unused_imports)] // Types land in C1 but get consumed by C2+.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Re-exports from the shared core so `crate::model::Event` keeps working.
pub use runner_core::model::{Event, EventDraft, EventKind, SignalType};
pub type Timestamp = DateTime<Utc>;
pub type Ulid = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crew {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub goal: Option<String>,
    pub orchestrator_policy: Option<serde_json::Value>,
    pub signal_types: Vec<SignalType>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// Global runner definition. A runner can be referenced by zero or more
// Runner is a config template — the agent CLI's runtime, command,
// args, env, optional system_prompt, optional working_dir, plus a
// globally-unique `handle` that names the template. Per-slot identity
// lives on `Slot` (see docs/impls/crew-slots.md): the same template
// can sit in multiple slots with distinct slot_handles even within
// one crew.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runner {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub runtime: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub system_prompt: Option<String>,
    pub env: HashMap<String, String>,
    /// Optional pinned model name (e.g. `claude-opus-4-7`,
    /// `gpt-5`). When set, the runtime adapter passes it through as
    /// the agent CLI's model flag (claude-code: `--model`). NULL =
    /// inherit the agent's own default. See migration 0008.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional thinking-effort hint (e.g. `xhigh`, `high`,
    /// `medium`). claude-code accepts an effort flag; codex's
    /// equivalent is wired in the runtime adapter. NULL = inherit
    /// the agent's own default. See migration 0008.
    #[serde(default)]
    pub effort: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// One position in a crew. Each slot references a Runner template and
// carries its own in-crew identity (`slot_handle`). Two slots in the
// same crew can both reference the same Runner, with different
// slot_handles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub id: String,
    pub crew_id: String,
    pub runner_id: String,
    pub slot_handle: String,
    pub position: i64,
    pub lead: bool,
    pub added_at: Timestamp,
}

// Slot joined with its Runner template. Returned by `slot_list` so
// the UI can render a crew's roster in one shot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotWithRunner {
    #[serde(flatten)]
    pub slot: Slot,
    pub runner: Runner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionStatus {
    Running,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub crew_id: String,
    pub title: String,
    pub status: MissionStatus,
    pub goal_override: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Timestamp,
    pub stopped_at: Option<Timestamp>,
    pub pinned_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Stopped,
    Crashed,
}

// A PTY run. `mission_id` is None for "direct chat" sessions that
// the user opened from the Runners page without starting a mission.
// `slot_id` is set for mission sessions (it's the slot they
// instantiate) and None for direct chats. `runner_id` always points
// at the runner template — for mission sessions it's a denorm of
// `slots.runner_id`. `cwd` is carried on the session row so direct
// sessions have a working directory even without a parent mission to
// inherit from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub mission_id: Option<String>,
    pub runner_id: String,
    pub slot_id: Option<String>,
    pub cwd: Option<String>,
    pub status: SessionStatus,
    pub pid: Option<i64>,
    pub started_at: Option<Timestamp>,
    pub stopped_at: Option<Timestamp>,
}
