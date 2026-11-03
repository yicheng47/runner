pub(crate) mod antigravity;
pub(crate) mod catalog;
pub(crate) mod claude_code;
pub(crate) mod codex;
pub(crate) mod copilot;
pub(crate) mod cursor_agent;
pub(crate) mod grok;
mod helpers;
pub(crate) mod opencode;
pub(crate) mod pi;
mod status;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod trae;

use crate::model::{CodexSpeed, Runtime};
use crate::router::prompt::SessionPromptKind;
use crate::router::runtime::{ForkPlan, PermissionMode, ResumePlan, FIRST_TURN_ARGV_MAX_BYTES};
use crate::session::hook_feed::HookWatcher;
use crate::session::runtime::SpawnSpec;
#[cfg(windows)]
pub(crate) use helpers::ordinary_windows_path;
pub use helpers::Permissions;
#[cfg(test)]
pub(crate) use helpers::{test_home, with_conversation_home};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

pub use runner_core::protocol::runtime::RuntimeCatalog;
pub use status::{CompactionPolicy, PermissionPromptPolicy, StatusSemantics, ToolEndPolicy};

pub fn catalogs() -> Vec<RuntimeCatalog> {
    Runtime::ALL
        .into_iter()
        .filter_map(|runtime| adapter(runtime).catalog())
        .collect()
}

pub struct LaunchContext<'a> {
    pub default_model: Option<&'a str>,
    pub env: &'a std::collections::BTreeMap<String, String>,
    pub cwd: Option<&'a Path>,
    pub role_args: &'a [String],
    pub app_data_dir: &'a Path,
    pub session_id: &'a str,
    pub resuming: bool,
    pub mission: bool,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub codex_speed: Option<CodexSpeed>,
    pub system_prompt: Option<&'a str>,
    pub first_turn: Option<&'a str>,
}

#[derive(Default)]
pub struct PromptChannels {
    pub system_prompt: bool,
    pub resend_persona_on_fresh: bool,
}

impl PromptChannels {
    pub fn split(
        &self,
        kind: SessionPromptKind,
        composed: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if !self.system_prompt {
            return (None, composed);
        }
        let Some(composed) = composed else {
            return (None, None);
        };
        match kind {
            SessionPromptKind::Direct => (Some(composed), None),
            SessionPromptKind::Member => (None, Some(composed)),
        }
    }
    pub fn member(
        &self,
        system_prompt: Option<&str>,
        crew_addendum: Option<&str>,
        mission_goal: Option<&str>,
        resuming: bool,
    ) -> (Option<String>, Option<String>) {
        let body = Some(crate::router::prompt::compose_member_first_turn(
            system_prompt,
            crew_addendum,
            mission_goal,
        ));
        if self.system_prompt && resuming {
            (body, None)
        } else {
            self.split(SessionPromptKind::Member, body)
        }
    }
    pub fn system_prompt_args(&self, prompt: Option<&str>) -> Vec<String> {
        match prompt {
            Some(prompt) if self.system_prompt && !prompt.trim().is_empty() => {
                helpers::strings(&["--append-system-prompt", prompt])
            }
            _ => Vec::new(),
        }
    }
}

pub struct ProbeContext<'a> {
    pub cwd: Option<&'a str>,
    pub role_env: &'a HashMap<String, String>,
}

#[derive(Default)]
pub struct MissingConversation {
    pub reuse_key: bool,
    pub resume_on_launch: bool,
}

pub enum KeyCapture {
    None,
    RolloutScan { sessions_root: Option<PathBuf> },
    LogTail,
    Hook,
}

pub use codex::capture::CaptureRequest as RolloutCaptureRequest;

pub struct CaptureContext {
    pub manager: Weak<crate::session::manager::SessionManager>,
    pub session_id: String,
    pub mission_id: Option<String>,
    pub app_data_dir: PathBuf,
    pub row_started_at: String,
    pub stop: Arc<AtomicBool>,
    pub pool: Arc<crate::db::DbPool>,
    pub events: Arc<dyn crate::session::manager::SessionEvents>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalEvent {
    CodexAttention(bool),
    Activity(crate::session::runtime::SessionActivityState),
    Title(crate::session::runtime::SessionActivityState),
    Ready(crate::session::runtime::SessionActivityState),
}
impl TerminalEvent {
    pub(crate) fn state(self) -> Option<crate::session::runtime::SessionActivityState> {
        match self {
            Self::CodexAttention(_) => None,
            Self::Activity(state) | Self::Title(state) | Self::Ready(state) => Some(state),
        }
    }
}
#[derive(Default)]
pub struct TerminalInput {
    pub(crate) state: Option<crate::session::runtime::SessionActivityState>,
    pub(crate) refresh: bool,
    pub(crate) announce: bool,
}
pub trait TerminalAdapter: Send {
    fn on_output(&mut self, bytes: &[u8]) -> Option<TerminalEvent>;
    /// Drain the latest native wait-state change after `on_output`, independently of activity.
    /// `true` holds delivery for human attention; `false` releases that native hold.
    fn take_attention(&mut self) -> Option<bool> {
        None
    }
    // Called after on_output; adapters may release deferred byte evidence here.
    fn output_activity_bytes(&self, bytes: &[u8]) -> usize {
        bytes.len()
    }
    fn on_input(&mut self, bytes: &[u8]) -> TerminalInput;
    fn held_activity(&self) -> Option<crate::session::runtime::SessionActivityState>;
    fn hooks_unavailable(&mut self);
    fn accept_event(&mut self, event: &crate::session::state::agent::AgentEvent) -> bool;
}

pub trait StatusHooks: Send + Sync {
    fn start_receiver(
        &self,
        _spec: &SpawnSpec,
        _receiver: crate::session::hook_queue::HookReceiver,
    ) -> Option<Box<dyn HookWatcher>> {
        None
    }
    fn supported(&self, windows: bool) -> bool;
    fn install(&self, _app_data_dir: &Path) {}
    fn install_with_env(
        &self,
        app_data_dir: &Path,
        _env: &std::collections::BTreeMap<String, String>,
    ) {
        self.install(app_data_dir);
    }
    fn cleanup(&self, _app_data_dir: &Path) {}
    fn tracks_pending_turn(&self) -> bool {
        false
    }
    fn env(
        &self,
        role_args: &[String],
        plan: &ResumePlan,
        app_data_dir: &Path,
        spec: &SpawnSpec,
    ) -> std::collections::BTreeMap<String, String>;
}

pub use runner_core::protocol::runtime::RuntimeCapabilities;

pub struct ModelDiscoverySource {
    pub order: u8,
    pub method: &'static str,
    pub query: fn(
        &str,
        &crate::shell_path::LoginShellEnv,
    ) -> Result<
        crate::runtime_status::models::ModelCatalog,
        crate::runtime_status::models::Reason,
    >,
    pub config_home: fn() -> Option<PathBuf>,
}

pub struct UsageSource {
    pub fetch: fn(
        &str,
        &crate::shell_path::LoginShellEnv,
        bool,
    ) -> Result<Vec<crate::usage::UsageWindow>, crate::usage::UnavailableReason>,
}

pub type SkillState = Box<dyn Fn(&crate::skills::SkillEntry) -> crate::skills::GlobalState>;
pub type SkillToggle =
    fn(&Path, Option<&Path>, &Path, bool) -> crate::error::Result<crate::skills::SkillCatalog>;

pub struct SkillSupport {
    pub roots: &'static [&'static str],
    pub load: fn(&Path, Option<&Path>) -> SkillState,
    pub manual: fn(&Path, &crate::skills::SkillDocument) -> bool,
    pub hidden: fn(&crate::skills::SkillDocument) -> bool,
    pub toggle: Option<SkillToggle>,
}

impl Default for SkillSupport {
    fn default() -> Self {
        Self {
            roots: &[],
            load: |_, _| Box::new(|_| crate::skills::GlobalState::On),
            manual: |_, document| document.value("disable-model-invocation") == Some("true"),
            hidden: |_| false,
            toggle: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum McpFormat {
    Json,
    Toml,
}

pub type McpJsonParser = fn(&serde_json::Value) -> Option<crate::ops::mcp::McpServerDefinition>;

pub struct McpConfig {
    pub wire_name: &'static str,
    pub serialized_name: &'static str,
    pub order: u8,
    pub config_file: &'static str,
    pub format: McpFormat,
    pub translate: fn(
        &crate::ops::mcp::McpServerDefinition,
    ) -> crate::error::Result<crate::ops::mcp::NativeEntry>,
    pub preserve_disabled: bool,
    pub supports_http: bool,
}

pub fn unbatched_status_semantics() -> StatusSemantics {
    // Standalone reducer events historically use Claude's hook semantics.
    claude_code::ClaudeCode.status_semantics()
}

pub trait RuntimeAdapter: Send + Sync {
    fn status_semantics(&self) -> StatusSemantics {
        StatusSemantics::default()
    }
    fn catalog(&self) -> Option<RuntimeCatalog>;
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::default()
    }
    fn native_defaults(&self, _home: &Path) -> crate::runtime_defaults::RuntimeDefaults {
        crate::runtime_defaults::RuntimeDefaults::default()
    }
    fn native_defaults_with_env(
        &self,
        home: &Path,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> crate::runtime_defaults::RuntimeDefaults {
        self.native_defaults(home)
    }
    fn discovery_home(&self, _env: &crate::shell_path::LoginShellEnv) -> Option<PathBuf> {
        self.model_discovery()
            .and_then(|source| (source.config_home)())
    }
    fn model_discovery(&self) -> Option<&'static ModelDiscoverySource> {
        None
    }
    fn discovered_efforts(
        &self,
        _catalog: &crate::runtime_status::models::ModelCatalog,
    ) -> Option<Vec<crate::ops::runtime::RuntimeCatalogOption>> {
        None
    }
    fn npm_dist_tag(&self) -> &'static str {
        "latest"
    }
    fn usage(&self) -> Option<&'static UsageSource> {
        None
    }
    fn mcp_path(
        &self,
        home: &Path,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> Option<PathBuf> {
        self.mcp()
            .map(|config| home.join(config.config_file.trim_start_matches("~/")))
    }
    fn mcp(&self) -> Option<&'static McpConfig> {
        None
    }
    fn mcp_json_parser(&self) -> Option<McpJsonParser> {
        None
    }
    fn read_mcp_entries(
        &self,
        _path: &Path,
    ) -> Option<
        crate::error::Result<std::collections::BTreeMap<String, crate::ops::mcp::NativeEntry>>,
    > {
        None
    }
    fn write_mcp_entry(
        &self,
        _path: &Path,
        _name: &str,
        _entry: Option<crate::ops::mcp::NativeEntry>,
        _merge: bool,
    ) -> Option<crate::error::Result<()>> {
        None
    }
    fn skills(&self) -> SkillSupport {
        SkillSupport::default()
    }
    fn permissions(&self) -> &'static Permissions;
    fn native_permission_mode(&self, _args: &[String]) -> Option<String> {
        None
    }
    fn skill_roots_with_env(
        &self,
        _home: &Path,
        _codex_home: Option<&Path>,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> Option<Vec<PathBuf>> {
        None
    }
    fn skill_state_with_env(
        &self,
        home: &Path,
        codex_home: Option<&Path>,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> SkillState {
        (self.skills().load)(home, codex_home)
    }
    fn toggle_skill_with_env(
        &self,
        _home: &Path,
        _path: &Path,
        _enabled: bool,
        _env: &std::collections::BTreeMap<String, String>,
    ) -> Option<crate::error::Result<crate::skills::SkillCatalog>> {
        None
    }
    fn launch_role_args(&self, args: &[String]) -> Vec<String> {
        args.to_vec()
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String>;
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan;
    fn model_effort_args(&self, _model: Option<&str>, _effort: Option<&str>) -> Vec<String> {
        Vec::new()
    }
    fn prompt_channels(&self) -> PromptChannels {
        PromptChannels::default()
    }
    fn launch_args(&self, ctx: &LaunchContext<'_>) -> Vec<String> {
        self.first_turn_argv(if ctx.resuming { None } else { ctx.first_turn })
    }
    fn launch_env(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    fn launch_context_env(
        &self,
        _ctx: &LaunchContext<'_>,
    ) -> std::collections::BTreeMap<String, String> {
        Default::default()
    }
    fn launch_gate(&self, _session_id: &str, _state: &Mutex<Option<Instant>>) {}
    fn seed_trust(
        &self,
        _session_id: &str,
        _cwd: Option<&Path>,
        _copilot_home: Option<&str>,
        _claude_config_dir: Option<&str>,
    ) -> crate::error::Result<()> {
        Ok(())
    }
    fn capture_prompt(
        &self,
        _session_id: &str,
        first_turn: Option<String>,
    ) -> (Option<String>, Option<String>) {
        (first_turn, None)
    }
    fn prepare_capture(&self, _app_data_dir: &Path, _session_id: &str) {}
    fn start_capture(&self, _ctx: CaptureContext) {}
    fn start_rollout_capture(&self, _request: RolloutCaptureRequest) {}
    fn cleanup_capture(&self, _app_data_dir: &Path, _pool: &crate::db::DbPool) {}
    fn remove_capture(&self, _app_data_dir: &Path, _session_id: &str) {}
    fn probe_uses_shell_env(&self) -> bool {
        false
    }
    fn key_capture(&self) -> KeyCapture {
        KeyCapture::None
    }
    fn key_capture_for_spawn(&self, _spec: &SpawnSpec) -> KeyCapture {
        self.key_capture()
    }
    fn terminal_adapter(&self, _pending_turn: Option<bool>) -> Option<Box<dyn TerminalAdapter>> {
        None
    }
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks> {
        None
    }
    fn startup_delivery_uses_hooks(&self, _pending_turn: bool) -> bool {
        false
    }
    /// Whether a resumed or forked conversation sends its startup hook, so a report's
    /// gate can wait for it (#748).
    fn startup_hook_on_reopen(&self) -> bool {
        true
    }
    /// Whether a fresh start with `args` in `cwd` opens with no folder-trust screen, as
    /// its config already records the decision, so a report's gate need not wait for a
    /// hook the runtime sends only with the first turn (#748).
    fn starts_trusted(
        &self,
        _cwd: Option<&Path>,
        _args: &[String],
        _env: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        false
    }
    fn mission_dir_args(&self, _dir: Option<&Path>) -> Vec<String> {
        Vec::new()
    }
    fn fork_plan(&self, _source_key: &str, _source_label: &str) -> Option<ForkPlan> {
        None
    }
    fn materialize_fork(
        &self,
        _spec: &SpawnSpec,
        _plan: &ForkPlan,
        _timeout: Duration,
    ) -> crate::error::Result<String> {
        Err(crate::error::Error::msg(
            "runtime does not support headless fork materialization",
        ))
    }
    fn conversation_exists(&self, _key: &str, _ctx: &ProbeContext<'_>) -> Option<bool> {
        None
    }
    /// The conversation a row should resume instead of `missing_key`, when
    /// the runtime can prove which one the row owns.
    fn recover_conversation(
        &self,
        _session_id: &str,
        _missing_key: &str,
        _ctx: &ProbeContext<'_>,
    ) -> Option<String> {
        None
    }
    fn missing_conversation(&self) -> MissingConversation {
        MissingConversation::default()
    }
}

pub struct NoAgent;
static NO_PERMISSIONS: Permissions = Permissions {
    offered: &[],
    strip_flags: &[],
    equals_on_bool: false,
    variadic_flag: None,
    args: |_| Vec::new(),
    matches: |_, _| false,
    mission_bypass: None,
    strip_on_mission_resume: false,
};
impl RuntimeAdapter for NoAgent {
    fn catalog(&self) -> Option<RuntimeCatalog> {
        None
    }
    fn permissions(&self) -> &'static Permissions {
        &NO_PERMISSIONS
    }
    fn first_turn_argv(&self, body: Option<&str>) -> Vec<String> {
        helpers::first_turn_body(body);
        Vec::new()
    }
    fn resume_plan(&self, _prior_key: Option<&str>) -> ResumePlan {
        ResumePlan::fresh()
    }
}

pub fn adapter(runtime: Runtime) -> &'static dyn RuntimeAdapter {
    match runtime {
        Runtime::Codex => &codex::Codex,
        Runtime::ClaudeCode => &claude_code::ClaudeCode,
        Runtime::Antigravity => &antigravity::Antigravity,
        Runtime::Pi => &pi::Pi,
        Runtime::Copilot => &copilot::Copilot,
        Runtime::Trae => &trae::Trae,
        Runtime::Cursor => &cursor_agent::CursorAgent,
        Runtime::Grok => &grok::Grok,
        Runtime::OpenCode => &opencode::OpenCode,
        Runtime::Shell => &NoAgent,
    }
}

pub fn for_key(key: &str) -> &'static dyn RuntimeAdapter {
    Runtime::parse(key).map(adapter).unwrap_or(&NoAgent)
}
