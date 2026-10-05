pub use runner_core::protocol::permissions::PermissionMode;

pub use runner_core::protocol::permissions::MissionPermissionMode;

pub const FIRST_TURN_ARGV_MAX_BYTES: usize = 32 * 1024;

/// Output of `resume_plan` — the args to layer onto the spawn command plus
/// the agent session key the spawn will operate under. The caller writes
/// `assigned_key` into `sessions.agent_session_key` so the next spawn for
/// the same scope (direct: same role; mission: same (mission, role))
/// can pass it back via `prior_key`.
#[derive(Debug, Clone)]
pub struct ResumePlan {
    /// Args to splice into the spawn command. For claude-code these
    /// are trailing flags; for codex and trae `resume <uuid>` is a subcommand
    /// prefix the caller must place ahead of any user-supplied args. See `prepend`.
    pub args: Vec<String>,
    /// `true` when `args` are a subcommand prefix that must precede the
    /// role's configured args (codex/trae resume). `false` when they are
    /// trailing flags safe to append (claude-code --session-id / --resume).
    pub prepend: bool,
    /// The native agent session key this spawn is bound to, when known up
    /// front. claude-code: a freshly-generated UUID we just told the CLI
    /// to use, or the prior key when resuming. codex/trae: the prior key when
    /// resuming, otherwise `None` (fresh sessions self-assign an id that
    /// Runner captures post-spawn).
    pub assigned_key: Option<String>,
    /// Whether this plan is a resume of a prior conversation. Callers can
    /// surface a "resuming previous session" hint, and on later detection
    /// of a resume failure, retry with `prior_key=None`.
    pub resuming: bool,
}

impl ResumePlan {
    pub(crate) fn fresh() -> Self {
        Self {
            args: Vec::new(),
            prepend: false,
            assigned_key: None,
            resuming: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ForkPlan {
    Direct(ResumePlan),
    Headless {
        args: Vec<String>,
        source_key: String,
    },
}
