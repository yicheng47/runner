use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    #[serde(flatten)]
    pub session: Session,
    pub live_title: Option<String>,
    /// Handle of the role this session instantiates — denormalized so the
    /// frontend can render `@coder`-style labels without a second lookup.
    pub handle: String,
    /// Effective runtime kind for this session (`"claude-code"`,
    /// `"codex"`, `"shell"`, …): `sessions.agent_runtime` when the row
    /// recorded one (runtime-override spawns), else the role row's
    /// `runtime`. Denormalized onto SessionRow so the frontend's
    /// terminal pane can gate per-runtime UX decisions
    /// (clear-on-resize for full-screen TUIs, etc.) without a second
    /// role lookup. See docs/impls/archive/0011 §"Per-runtime clear-on-resize".
    pub runtime: String,
    /// Whether this role is the lead for the mission's crew.
    pub lead: bool,
    /// Native agent conversation key captured from the spawned agent.
    /// NULL means capture is pending, unavailable, or intentionally
    /// failed closed.
    pub agent_session_key: Option<String>,
}

/// One row per direct-chat *session* in the sidebar SESSION tray. Each
/// role can host multiple parallel chats — see
/// docs/impls/archive/0003-direct-chats.md — so the tray is flat (not collapsed per
/// role). Stopped/crashed rows stay listed because they can be
/// resumed via `session_resume`, which preserves the row's id and
/// `agent_session_key`.
///
/// Click behavior on the frontend:
///   - `status = "running"` → attach to the live PTY.
///   - `status = "stopped" | "crashed"` → call `session_resume` (the
///     respawn happens server-side; the row stays the same), then
///     attach.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectSessionEntry {
    pub session_id: String,
    pub project_id: Option<String>,
    pub role_id: Option<String>,
    pub handle: Option<String>,
    pub agent_runtime: String,
    pub agent_command: String,
    /// Effective model on resume, with blank values normalized to None.
    /// Filled by `session_get`; recent and archived lists return None.
    pub agent_model: Option<String>,
    /// Effective effort on resume, with blank values normalized to None.
    /// Filled by `session_get`; recent and archived lists return None.
    pub agent_effort: Option<String>,
    pub display_name: String,
    pub status: SessionStatus,
    /// User-authored label. NULL → frontend derives a default from
    /// handle + start time. Set via `session_rename`.
    pub title: Option<String>,
    pub live_title: Option<String>,
    /// Per-chat cwd override stored on the row at spawn. NULL means
    /// the chat falls back to the role's `working_dir` on
    /// resume/spawn. Surfaced for the chat header's meta line.
    pub cwd: Option<String>,
    pub started_at: Option<Timestamp>,
    pub stopped_at: Option<Timestamp>,
    /// `true` iff `agent_session_key IS NOT NULL`. Lets the UI
    /// distinguish "stopped but resumable" from "stopped and
    /// forgotten" without shipping the raw key down.
    pub resumable: bool,
    pub native_fork: bool,
    pub forkable: bool,
    /// Native agent conversation key for the active direct chat.
    /// `session_list_recent_direct` intentionally returns NULL here;
    /// `session_get` is the full-detail path RoleChat uses for the
    /// visible row.
    pub agent_session_key: Option<String>,
    /// `true` iff `pinned_at IS NOT NULL`. Pinned rows render with a
    /// pin glyph and sort to the top of the tray.
    pub pinned: bool,
    /// When set, the session has been archived: hidden from the SESSION
    /// tray and `session_list_recent_direct`. Returned here so
    /// `session_get` (unfiltered) can tell the chat page to render
    /// read-only when the user navigates to an archived session by
    /// direct URL. `listRecentDirect` filters these out at SQL, so
    /// rows from that surface always carry `archived_at: None`.
    pub archived_at: Option<Timestamp>,
}

impl DirectSessionEntry {
    pub fn preferred_title(&self, live: Option<&str>) -> Option<String> {
        self.title.clone().or_else(|| {
            if Runtime::parse(&self.agent_runtime).is_some_and(Runtime::is_shell) {
                return live
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_owned);
            }
            // A role-backed chat is an identity, not a topic. Most runtimes
            // inject the role's system prompt as the first turn, while pi uses
            // its native system-prompt channel. Either way, the agent-derived
            // topic must not replace the role identity.
            // #587 already keeps the handle on mission surfaces for this
            // reason: identity is the thing you address, so it may not
            // move under you. A name the user typed still wins above.
            if self.handle.is_some() {
                return None;
            }
            live.and_then(|title| super::session_title::provider_title(title, self.cwd.as_deref()))
                .or_else(|| {
                    self.live_title.as_deref().and_then(|title| {
                        super::session_title::provider_title(title, self.cwd.as_deref())
                    })
                })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartDirectSessionOutput {
    #[serde(flatten)]
    pub session: SpawnedSession,
    pub project_id: Option<String>,
    pub cwd: Option<String>,
}
/// Row returned to the frontend after a spawn. Subset of the DB `sessions`
/// row with the role handle denormalized so the debug page can render
/// `@coder`-style labels without a separate lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnedSession {
    pub id: String,
    pub mission_id: Option<String>,
    pub role_id: Option<String>,
    pub handle: String,
    pub pid: Option<u32>,
}
/// Busy/idle of one session. The forwarder infers it from PTY-byte
/// activity (issue #124), hook adapters may report it, the router projects
/// it per handle, and the UI reads the same projection. Serialized lowercase
/// in `session/status` events and in `session_status` rows on the mission log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionActivityState {
    Busy,
    Idle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoResumeReport {
    pub resumed: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StartDirectSessionArgs {
    /// Optional role ID. Omit it for a role-free runtime chat.
    #[serde(default)]
    pub role_id: Option<String>,
    /// Optional runtime registry name. With a role, this overrides the
    /// role's runtime; without a role, it starts a role-free chat.
    #[serde(default)]
    pub runtime: Option<super::model::Runtime>,
    /// Optional model for the selected role or runtime.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional reasoning effort for the selected role or runtime.
    #[serde(default)]
    pub effort: Option<String>,
    /// Optional per-chat Codex Speed. Omit or null to inherit.
    #[serde(default)]
    pub speed: Option<super::model::CodexSpeed>,
    /// Optional project membership. Its cwd is used when cwd is omitted.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Optional working-directory override.
    #[serde(default)]
    pub cwd: Option<String>,
}
