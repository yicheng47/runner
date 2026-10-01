/// Permission mode the role-edit form's dropdown writes onto the
/// role row's `args` column. The mode space is per-runtime: each
/// runtime exposes only the modes it natively supports.
///
/// claude-code (4 modes — `--permission-mode <value>`):
/// - **Default** — no flag. Reads only auto-approve; everything else
///   prompts.
/// - **AcceptEdits** — `--permission-mode acceptEdits`. Auto-accepts
///   file edits and common filesystem commands (`mkdir`, `touch`,
///   `mv`, `cp`, etc.). Available on every plan.
/// - **Auto** — `--permission-mode auto`. The "real" auto with a
///   server-side classifier blocking irreversible / destructive /
///   external actions. Requires Max / Team / Enterprise / API plan
///   AND a supported model (Opus 4.7 on Max; Sonnet 4.6 / Opus 4.6 /
///   Opus 4.7 on Team / Enterprise / API). NOT available on Pro,
///   Bedrock, Vertex, or Foundry. See
///   <https://code.claude.com/docs/en/permission-modes#eliminate-prompts-with-auto-mode>.
/// - **Bypass** — `--permission-mode bypassPermissions`. Skip every
///   check. Triggers claude-code's one-time consent dialog the first
///   time per user account, which is why it's NOT the recommended
///   default.
///
/// codex (3 modes — codex doesn't have a separate "accept edits"
/// middle ground, so AcceptEdits is treated as Default for this
/// runtime):
/// - **Default** — no flag. Codex's built-in default approval
///   cadence (`untrusted`).
/// - **Auto** — `--ask-for-approval on-request --sandbox
///   workspace-write`. The model decides when to ask the user for
///   approval; otherwise auto-runs in the workspace. (Codex's
///   `on-failure` value is deprecated per `codex --help` and not
///   exposed here.)
/// - **Bypass** — `--ask-for-approval never --sandbox
///   workspace-write`. Never ask.
///
/// copilot exposes Default (no flag), Accept edits (`--allow-tool=write`), and Bypass (`--yolo`); Auto has no supported equivalent.
///
/// antigravity exposes Default (no flag), Accept edits (`--mode accept-edits`), and Bypass
/// (`--dangerously-skip-permissions`); Auto has no equivalent.
///
/// trae accepts `default` (ask), `plan` (plan-only), and
/// `bypass_permissions` (never ask). It has no auto-approve middle ground:
/// - **Default** — no flag; use TRAE CLI's configured default.
/// - **Auto** / **AcceptEdits** — no flag; no native equivalent.
/// - **Bypass** — `--permission-mode bypass_permissions`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Auto,
    Bypass,
}

/// App-wide permission mode for mission slots (feature 527). Read at
/// spawn time for every mission slot; attended chats assert no
/// permission posture. `RoleDefault` leaves the row's args untouched.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MissionPermissionMode {
    #[default]
    Bypass,
    Auto,
    #[serde(alias = "runner-default")]
    RoleDefault,
}

impl MissionPermissionMode {
    pub const ALL: [Self; 3] = [Self::Bypass, Self::Auto, Self::RoleDefault];

    pub fn key(self) -> &'static str {
        match self {
            Self::Bypass => "bypass",
            Self::Auto => "auto",
            Self::RoleDefault => "role-default",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        if value == "runner-default" {
            Some(Self::RoleDefault)
        } else {
            Self::ALL.into_iter().find(|mode| mode.key() == value)
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bypass => "Bypass",
            Self::Auto => "Auto",
            Self::RoleDefault => "Role default",
        }
    }
}

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
