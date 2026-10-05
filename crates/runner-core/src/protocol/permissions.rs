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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
