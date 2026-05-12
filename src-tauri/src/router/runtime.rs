// Runtime adapter: maps `runner.runtime` to the extra CLI args the child
// process needs at spawn time.
//
// Two responsibilities live here:
//
//   1. `system_prompt_args` — hand `runner.system_prompt` to the agent
//      CLI via its native argv hook, when one exists. claude-code's
//      `--append-system-prompt` / `--system-prompt` are SDK-only
//      (require `-p` / print mode); the interactive TUI silently
//      ignores them. So claude-code returns no argv from this
//      function — the prompt is delivered via stdin as a first user
//      turn instead, by `SessionManager::schedule_first_prompt`.
//      Codex follows the same stdin path: a startup permission /
//      approval dialog can swallow or misorder the positional
//      `[PROMPT]` argv, so codex also returns empty here and the
//      brief lands via stdin once the TUI has settled.
//      arch §4.2 / §4.3.
//
//   2. `resume_plan` — pass the agent CLI's *own* resumable
//      session/conversation id back to it on respawn so closing the app
//      doesn't force a fresh agent conversation. The id we persist is the
//      agent's native id (claude-code session UUID, codex rollout UUID),
//      not Runner's `sessions.id`. See `sessions.agent_session_key` and
//      migrations/0002_agent_session_key.sql.
//
// Keeping the runtime mapping in one place means both `SessionManager::spawn`
// (mission) and `spawn_direct` (chat) get the same behavior with no chance
// of drift. Lives under router/ because the router's `mission_goal`
// handler already owns prompt composition; the runtime adapter is the
// related "how do we hand prompts and identity to a real CLI" piece.

/// Compute the extra args (in declaration order) to append after the
/// runner's configured `args` so the child receives the pinned model
/// and thinking effort via the runtime's native flags. Returns an
/// empty Vec when both fields are unset (NULL on the row) or when
/// the runtime has no equivalent. Mirrors `system_prompt_args` in
/// style: pure, declaration-order-aware, easy to unit-test.
///
/// claude-code maps:
///   - `model` → `--model <name>` (e.g. `claude-opus-4-7`)
///   - `effort` → `--effort <level>`. Accepted levels per `claude
///     --help`: `low / medium / high / xhigh / max`. The flag was
///     `--thinking-effort` in earlier docs but the installed CLI
///     ships `--effort`; we pass the row's value through verbatim
///     so the CLI's own validation is the source of truth.
///
/// codex maps:
///   - `model` → `--model <name>`. Verified against `codex --help`.
///   - `effort` → `-c model_reasoning_effort=<level>`. Codex has no
///     dedicated `--reasoning-effort` flag, but its `-c key=value`
///     config-override flag (verified against `codex --help` on the
///     installed CLI) accepts the same `model_reasoning_effort` key
///     used in `~/.codex/config.toml`. The value is parsed as TOML
///     with a raw-string fallback, so passing the level unquoted is
///     fine. The level is lowercased before being formatted in:
///     codex's TOML enum is case-sensitive and rejects e.g. `High`
///     with `unknown variant 'High', expected one of 'none',
///     'minimal', 'low', 'medium', 'high', 'xhigh'`, but rows often
///     store the level title-cased ("High"). claude-code's
///     `--effort` is *not* case-sensitive (accepts `High`), so the
///     claude-code branch deliberately forwards the value verbatim
///     to avoid a regression on already-shipped behavior.
///
/// shell / unknown runtimes: no equivalent flags — degrade silently
/// so the runner row's preference is recorded but the spawn
/// doesn't reject on unknown args.
pub fn model_effort_args(runtime: &str, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    fn trim_some(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    let model = trim_some(model);
    let effort = trim_some(effort);
    if model.is_none() && effort.is_none() {
        return Vec::new();
    }
    match runtime {
        "claude-code" => {
            let mut out = Vec::new();
            if let Some(m) = model {
                out.push("--model".into());
                out.push(m.to_string());
            }
            if let Some(e) = effort {
                out.push("--effort".into());
                out.push(e.to_string());
            }
            out
        }
        "codex" => {
            let mut out = Vec::new();
            if let Some(m) = model {
                out.push("--model".into());
                out.push(m.to_string());
            }
            if let Some(e) = effort {
                // No dedicated flag; reuse the config-override path.
                // Lowercase: codex's TOML enum is case-sensitive
                // (rejects "High" with "unknown variant").
                out.push("-c".into());
                out.push(format!("model_reasoning_effort={}", e.to_ascii_lowercase()));
            }
            out
        }
        // shell / unknown
        _ => Vec::new(),
    }
}

/// Permission mode the runner-edit form's dropdown writes onto the
/// runner row's `args` column. The mode space is per-runtime: each
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Auto,
    Bypass,
}

/// Canonical argv for the runtime's permission mode. Returns an
/// empty Vec for runtimes with no equivalent (shell / unknown), for
/// `Default` (which is the natural "no flag" state), and for modes
/// the runtime doesn't natively support (e.g. AcceptEdits on codex —
/// codex's wire protocol has no edits-only middle ground). Used by
/// `commands::runner::create` / `update` to write the chosen mode
/// onto the row's `args` column at *create* time, not at spawn time
/// — so an existing row stays stable even if the form's recommended
/// default shifts.
pub fn permission_mode_args(runtime: &str, mode: PermissionMode) -> Vec<String> {
    match (runtime, mode) {
        (_, PermissionMode::Default) => Vec::new(),
        // claude-code: `--permission-mode <value>` for every
        // non-Default mode. The flag value differs from the enum
        // variant casing (acceptEdits, bypassPermissions) because
        // that's what claude-code's CLI accepts.
        ("claude-code", PermissionMode::AcceptEdits) => {
            vec!["--permission-mode".into(), "acceptEdits".into()]
        }
        ("claude-code", PermissionMode::Auto) => {
            vec!["--permission-mode".into(), "auto".into()]
        }
        ("claude-code", PermissionMode::Bypass) => {
            vec!["--permission-mode".into(), "bypassPermissions".into()]
        }
        // codex: `--ask-for-approval <cadence> --sandbox
        // workspace-write` pair for both Auto and Bypass. AcceptEdits
        // returns empty (codex has no equivalent) so a user who
        // somehow lands AcceptEdits on a codex row reads as Default.
        ("codex", PermissionMode::AcceptEdits) => Vec::new(),
        ("codex", PermissionMode::Auto) => vec![
            "--ask-for-approval".into(),
            "on-request".into(),
            "--sandbox".into(),
            "workspace-write".into(),
        ],
        ("codex", PermissionMode::Bypass) => vec![
            "--ask-for-approval".into(),
            "never".into(),
            "--sandbox".into(),
            "workspace-write".into(),
        ],
        _ => Vec::new(),
    }
}

/// Strip every prior occurrence of the runtime's permission-mode
/// flags (and their values, where applicable) from `args`, preserving
/// order of the surviving args. Used by `commands::runner::create`
/// and `update` so cycling the dropdown round-trips without leaving
/// duplicate or orphan flags.
///
/// Match shape per runtime:
///   - codex: `--ask-for-approval <value>` and `--sandbox <value>`
///     (the next token is consumed as the value, regardless of what
///     it is — the dropdown is opinionated about owning these
///     flags). `--flag=value` form is also stripped.
///   - claude-code: `--permission-mode <value>` (value-bearing) and
///     `--dangerously-skip-permissions` (standalone, kept in the
///     strip set so legacy rows that still carry the deprecated flag
///     get cleaned up the next time the user touches their row).
pub fn strip_permission_flags(runtime: &str, args: &[String]) -> Vec<String> {
    // (flag_name, takes_value)
    let keys: &[(&str, bool)] = match runtime {
        "codex" => &[("--ask-for-approval", true), ("--sandbox", true)],
        "claude-code" => &[
            ("--dangerously-skip-permissions", false),
            ("--permission-mode", true),
        ],
        _ => &[],
    };
    if keys.is_empty() {
        return args.to_vec();
    }
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        // Exact-match `--flag` form. For takes_value flags we also
        // skip the next token if present (it's the value).
        if let Some(&(_, takes_value)) = keys.iter().find(|(name, _)| name == arg) {
            i += if takes_value && i + 1 < args.len() {
                2
            } else {
                1
            };
            continue;
        }
        // `--flag=value` form: strip the whole token in one go.
        if keys
            .iter()
            .any(|(name, takes_value)| *takes_value && arg.starts_with(&format!("{name}=")))
        {
            i += 1;
            continue;
        }
        out.push(arg.clone());
        i += 1;
    }
    out
}

/// Compose the runner row's stored `args` from the user-provided
/// `args` and the runner-edit-form's "Permission mode" segmented
/// control. Strips any prior occurrence of the runtime's permission
/// flags, then appends the canonical args for the chosen mode. No-op
/// for runtimes without a permission concept (shell / unknown).
pub fn apply_permission_mode(runtime: &str, args: &[String], mode: PermissionMode) -> Vec<String> {
    let mut out = strip_permission_flags(runtime, args);
    out.extend(permission_mode_args(runtime, mode));
    out
}

/// Frontend-mirror helper: inspect a runner's stored `args` and
/// decide which option of the runner-edit form's "Permission mode"
/// dropdown should render as selected. Probe order is most-aggressive
/// first: `Bypass` → `Auto` → `AcceptEdits` → `Default`. A row
/// carrying flags for multiple modes resolves to the most-aggressive
/// one because that's what claude-code / codex would actually honor
/// at spawn time (the strip-and-replace round-trip on save will then
/// converge to a single canonical pair).
///
/// Conflicting / partial / unrecognized values fall through to
/// `Default` — the user clearly didn't pick the stricter mode and
/// we don't want a row's UI to misrepresent its stored args.
///
/// Mirrors `inferPermissionMode` in `src/components/ui/runtimes.ts`
/// — the frontend hand-port is constrained by this function's tests.
///
/// `#[allow(dead_code)]` because the only direct consumer is the
/// test suite: the function exists to *pin the algorithm* the
/// frontend hand-ports, not to be called from Rust spawn paths
/// (those use `apply_permission_mode` for write-side flag
/// management). Keep it `pub` so it's discoverable from a
/// `runtime::` module search.
#[allow(dead_code)]
pub fn infer_permission_mode(runtime: &str, args: &[String]) -> PermissionMode {
    if mode_pair_matches(runtime, args, PermissionMode::Bypass) {
        return PermissionMode::Bypass;
    }
    if mode_pair_matches(runtime, args, PermissionMode::Auto) {
        return PermissionMode::Auto;
    }
    if mode_pair_matches(runtime, args, PermissionMode::AcceptEdits) {
        return PermissionMode::AcceptEdits;
    }
    PermissionMode::Default
}

#[allow(dead_code)]
fn mode_pair_matches(runtime: &str, args: &[String], mode: PermissionMode) -> bool {
    // Legacy shape: pre-rename claude-code rows used a standalone
    // `--dangerously-skip-permissions` flag for Bypass. Read it as
    // Bypass so the dropdown loads the right initial value. Strip
    // helper still cleans it up on save.
    if runtime == "claude-code"
        && mode == PermissionMode::Bypass
        && legacy_bypass_present(runtime, args)
    {
        return true;
    }
    let pairs = mode_match_pairs(runtime, mode);
    if pairs.is_empty() {
        return false;
    }
    pairs
        .iter()
        .all(|&(flag, expected)| flag_value_matches(args, flag, expected))
}

/// (flag, Some(expected_value)) pairs we use to *recognize* a mode
/// in a stored args list. Hand-synced with `permission_mode_args`.
/// `Default` has no pairs — its "match" is "no other mode's pair
/// fully matched." Modes a runtime doesn't natively support (e.g.
/// AcceptEdits on codex) return empty pairs, so they never match
/// for that runtime.
///
/// Includes a legacy-shape entry for claude-code's Bypass: rows
/// created before this PR carried the standalone
/// `--dangerously-skip-permissions` flag, which we still recognize
/// (and the strip helper still removes) so existing installs read
/// the right mode in their UI.
#[allow(dead_code)]
fn mode_match_pairs(
    runtime: &str,
    mode: PermissionMode,
) -> &'static [(&'static str, Option<&'static str>)] {
    match (runtime, mode) {
        ("claude-code", PermissionMode::AcceptEdits) => {
            &[("--permission-mode", Some("acceptEdits"))]
        }
        ("claude-code", PermissionMode::Auto) => &[("--permission-mode", Some("auto"))],
        ("claude-code", PermissionMode::Bypass) => {
            &[("--permission-mode", Some("bypassPermissions"))]
        }
        ("codex", PermissionMode::Auto) => &[
            ("--ask-for-approval", Some("on-request")),
            ("--sandbox", Some("workspace-write")),
        ],
        ("codex", PermissionMode::Bypass) => &[
            ("--ask-for-approval", Some("never")),
            ("--sandbox", Some("workspace-write")),
        ],
        _ => &[],
    }
}

/// Legacy-shape recognizer: claude-code rows created before the
/// rename to `--permission-mode bypassPermissions` carried a
/// standalone `--dangerously-skip-permissions` flag. The strip
/// helper drops both (so a save converges to the new flag) but
/// `infer_permission_mode` also needs to read the legacy flag as
/// `Bypass` for the dropdown's initial value. Layered as a check
/// inside `mode_pair_matches` for `claude-code` + `Bypass` only.
#[allow(dead_code)]
fn legacy_bypass_present(runtime: &str, args: &[String]) -> bool {
    runtime == "claude-code" && args.iter().any(|a| a == "--dangerously-skip-permissions")
}

/// `--flag <expected>` (separated) OR `--flag=<expected>` (equals)
/// for value-bearing flags; bare-token presence for value-less
/// flags. A wrong value at one site doesn't invalidate a later
/// canonical site (we keep scanning), so duplicate entries with
/// mixed values resolve toward "match found".
///
/// Helper for `infer_permission_mode`; same dead-code caveat.
#[allow(dead_code)]
fn flag_value_matches(args: &[String], flag: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return args.iter().any(|a| a == flag);
    };
    let equals_token = format!("{flag}={expected}");
    for (i, arg) in args.iter().enumerate() {
        if arg == &equals_token {
            return true;
        }
        if arg == flag {
            if let Some(next) = args.get(i + 1) {
                if next == expected {
                    return true;
                }
            }
        }
    }
    false
}

/// Compute the extra args (in declaration order) to append after the
/// runner's configured `args` so the child receives `system_prompt` via the
/// runtime's native flag. Returns an empty Vec when no prompt is set or
/// when the runtime delivers prompts through stdin instead of argv.
pub fn system_prompt_args(runtime: &str, system_prompt: Option<&str>) -> Vec<String> {
    let prompt = match system_prompt {
        Some(p) if !p.trim().is_empty() => p,
        _ => return Vec::new(),
    };
    let _ = prompt;
    match runtime {
        // claude-code's --append-system-prompt and --system-prompt
        // are documented as SDK-only — they require `-p` (print
        // mode), which is incompatible with our interactive TUI
        // launches. Passing them in interactive mode is silently
        // dropped. Workaround: the call site delivers the prompt via
        // stdin once the TUI is up (see
        // SessionManager::schedule_first_prompt).
        "claude-code" => Vec::new(),
        // codex has no system-prompt flag (we tried `--instructions` and
        // it's rejected; `~/.codex/config.toml` doesn't expose one
        // either). Codex's positional `[PROMPT]` argv was the closest
        // available hook, but it loses races with codex's startup
        // permission / approval dialog: when the TUI shows that
        // dialog before hitting its main loop, the positional prompt
        // is swallowed, replayed stale, or misordered. So we fall
        // through to the same stdin-injection path claude-code uses —
        // see `SessionManager::schedule_first_prompt`, which waits
        // for the TUI to settle and then types the brief + Enter.
        "codex" => Vec::new(),
        // shell / unknown — no prompt mechanism.
        _ => Vec::new(),
    }
}

/// Defense-in-depth ceiling on the positional `[PROMPT]` argv payload.
/// Persistence-layer validation in `commands::runner` /
/// `commands::mission` / `commands::crew` caps the individual fields
/// (`system_prompt`, `mission_goal`, `crew.goal`) so the composed
/// body never approaches this number. Set well below macOS `ARG_MAX`
/// (~256 KB) but high enough that no realistic user input can hit it
/// once the persist-time caps are honored. `debug_assert!`-trips on
/// overshoot — surfaces a logic bug, not a user error.
pub const FIRST_TURN_ARGV_MAX_BYTES: usize = 32 * 1024;

/// Map a runtime + composed first-turn body to the positional argv
/// the agent CLI reads as its first user turn at process spawn.
///
/// claude-code and codex both accept a positional `[PROMPT]` argument
/// (verified against claude-code and codex-cli 0.130.0). Delivering
/// the first turn at spawn-time eliminates the post-spawn paste race
/// the original `inject_paste_with_verify` machinery was working
/// around: the agent reads its argv during init, before the TUI binds
/// raw input, before any trust-folder dialog, before the input editor
/// even exists. See `docs/impls/0007-spawn-time-prompt-delivery.md`.
///
/// Returns empty when:
///   - the body is None or blank,
///   - the runtime has no positional first-turn convention (`shell`).
///
/// In debug builds, panics if the body exceeds
/// `FIRST_TURN_ARGV_MAX_BYTES` — that indicates a missing
/// persist-time validation upstream. Release builds silently truncate
/// the argv to empty to fail safe.
///
/// The old comment block in `system_prompt_args` claiming codex's
/// positional gets swallowed by a startup approval dialog was stale
/// (likely pre-0.130.0); modern codex has no startup dialog by
/// default. `--ask-for-approval` modes gate in-session *command*
/// approvals, not boot.
pub fn first_turn_argv(runtime: &str, body: Option<&str>) -> Vec<String> {
    let body = match body {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Vec::new(),
    };
    debug_assert!(
        body.len() <= FIRST_TURN_ARGV_MAX_BYTES,
        "first-turn argv body exceeds {FIRST_TURN_ARGV_MAX_BYTES} bytes \
         (got {}) — persistence-layer validation should have caught this",
        body.len()
    );
    if body.len() > FIRST_TURN_ARGV_MAX_BYTES {
        return Vec::new();
    }
    match runtime {
        "claude-code" | "codex" => vec![body.to_string()],
        _ => Vec::new(),
    }
}

/// Compose the runtime-specific trailing args (model/effort flags +
/// any `system_prompt` argv + first-turn body positional) in the
/// order the runtime's CLI expects.
///
/// `system_prompt_args` still returns empty for both supported
/// runtimes (claude-code's `--append-system-prompt` is SDK-only; codex
/// has no equivalent flag). The first user turn — composed launch
/// prompt for a mission lead, worker preamble for non-leads, persona
/// for direct chats — rides on `first_turn_argv` instead and lands as
/// the trailing positional.
///
/// `plan_resuming` suppresses both the system_prompt argv (legacy,
/// no-op today) and the first_turn argv — replaying a launch prompt
/// onto a resumed conversation would inject a duplicate user turn.
/// Resume paths rely on the agent CLI's own session resume to restore
/// context; the rare resume-fresh-fallback case is handled by
/// `Router::fire_lead_launch_prompt` via paste delivery instead.
pub fn trailing_runtime_args(
    runtime: &str,
    plan_resuming: bool,
    model: Option<&str>,
    effort: Option<&str>,
    system_prompt: Option<&str>,
    first_turn: Option<&str>,
) -> Vec<String> {
    let mut out = model_effort_args(runtime, model, effort);
    let prompt_for_argv = if plan_resuming { None } else { system_prompt };
    out.extend(system_prompt_args(runtime, prompt_for_argv));
    let first_turn_for_argv = if plan_resuming { None } else { first_turn };
    out.extend(first_turn_argv(runtime, first_turn_for_argv));
    out
}

/// Output of `resume_plan` — the args to layer onto the spawn command plus
/// the agent session key the spawn will operate under. The caller writes
/// `assigned_key` into `sessions.agent_session_key` so the next spawn for
/// the same scope (direct: same runner; mission: same (mission, runner))
/// can pass it back via `prior_key`.
#[derive(Debug, Clone)]
pub struct ResumePlan {
    /// Args to splice into the spawn command. For claude-code these are
    /// trailing flags; for codex `resume <uuid>` is a subcommand prefix the
    /// caller must place ahead of any user-supplied args. See `prepend`.
    pub args: Vec<String>,
    /// `true` when `args` are a subcommand prefix that must precede the
    /// runner's configured args (codex resume). `false` when they are
    /// trailing flags safe to append (claude-code --session-id / --resume).
    pub prepend: bool,
    /// The native agent session key this spawn is bound to, when known up
    /// front. claude-code: a freshly-generated UUID we just told the CLI to
    /// use, or the prior key when resuming. codex: the prior key when
    /// resuming, otherwise `None` (fresh codex sessions self-assign an id;
    /// post-spawn capture is a follow-up).
    pub assigned_key: Option<String>,
    /// Whether this plan is a resume of a prior conversation. Callers can
    /// surface a "resuming previous session" hint, and on later detection
    /// of a resume failure, retry with `prior_key=None`.
    pub resuming: bool,
}

impl ResumePlan {
    fn fresh() -> Self {
        Self {
            args: Vec::new(),
            prepend: false,
            assigned_key: None,
            resuming: false,
        }
    }
}

/// Decide how to launch `runtime` so it either resumes the agent's prior
/// conversation (when `prior_key` is set and the runtime supports resume)
/// or starts fresh with a key we can capture for the next spawn.
///
/// Failure modes:
///   - Unknown runtime → fresh spawn, no key (degrade silently).
///   - claude-code with no `prior_key` → fresh spawn but with a self-assigned
///     UUID via `--session-id`, so the very next respawn can resume.
///   - codex with no `prior_key` → fresh spawn, no key. Capturing the codex
///     rollout id post-spawn is tracked as a follow-up; until then, codex
///     resumes only if the user has previously triggered a captured key by
///     other means (manual seed, future capture path).
///
/// `prior_key` should be the value of `sessions.agent_session_key` from the
/// most recent prior session in the same scope. The caller decides how to
/// scope: direct chats look up the most recent session for the same
/// `runner_id` with `mission_id IS NULL`; mission spawns look up the most
/// recent session for the same `(mission_id, runner_id)`.
pub fn resume_plan(runtime: &str, prior_key: Option<&str>) -> ResumePlan {
    match runtime {
        "claude-code" => match prior_key {
            Some(k) if is_uuid(k) => ResumePlan {
                // `--resume <uuid>` is required on resume. We tried
                // `--session-id <uuid>` previously, but claude-code
                // refuses to start with a session id it already
                // recognises as in use ("Session ID … is already in
                // use") — it treats `--session-id` as fresh-only. The
                // edge case `--resume` exposes ("session not found"
                // when the conversation file was never persisted)
                // is now masked by `schedule_first_prompt`, which
                // always sends a first user turn to claude-code on
                // fresh spawn so the conversation file lands on disk
                // before any future resume tries to load it. If a
                // resume still fails (e.g. the user killed the app
                // within ~1.5s of spawn before the first turn went
                // through), the reader thread's `resume_failed`
                // heuristic wipes `agent_session_key` and the next
                // launch starts fresh.
                args: vec!["--resume".into(), k.to_string()],
                prepend: false,
                assigned_key: Some(k.to_string()),
                resuming: true,
            },
            _ => {
                // Self-assign a UUID so next time we can resume by that id.
                // claude-code's `--session-id` requires a valid UUID and
                // binds that id to the new conversation.
                let id = uuid::Uuid::new_v4().to_string();
                ResumePlan {
                    args: vec!["--session-id".into(), id.clone()],
                    prepend: false,
                    assigned_key: Some(id),
                    resuming: false,
                }
            }
        },
        "codex" => match prior_key {
            Some(k) if is_uuid(k) => ResumePlan {
                // `codex resume <uuid>` is a subcommand prefix. The caller
                // places these args ahead of any user-supplied args.
                args: vec!["resume".into(), k.to_string()],
                prepend: true,
                assigned_key: Some(k.to_string()),
                resuming: true,
            },
            _ => ResumePlan::fresh(),
        },
        // shell / unknown — no resume concept. Custom wrappers can be wired
        // up later.
        _ => ResumePlan::fresh(),
    }
}

fn is_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

/// True iff claude-code's conversation file for `(cwd, uuid)` exists on
/// disk. Used by `SessionManager::resume` to skip `--resume <uuid>` when
/// the agent never persisted a turn (lead PTYs reset within the
/// schedule_first_prompt window, fast Stop after spawn) — passing
/// `--resume` against a missing file makes claude-code print
/// "No conversation found with session ID …" and leave the TUI sitting
/// in a half-initialised state. Path scheme:
/// `$HOME/.claude/projects/<cwd-with-/-as--dashes>/<uuid>.jsonl`. We
/// resolve `cwd` with the same precedence the spawn used (mission /
/// runner override) and skip the check when no concrete cwd is known.
pub fn claude_code_conversation_exists(cwd: Option<&str>, uuid: &str) -> bool {
    // `cfg(test)` short-circuits the filesystem check so unit tests
    // for the resume flow don't have to fake out
    // `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. The path
    // encoding is exercised directly by `claude_code_conversation_*`
    // tests below; the SessionManager-level resume tests just want
    // the production semantic of "prior conversation present" to
    // hold so they can assert key preservation.
    #[cfg(test)]
    {
        let _ = (cwd, uuid);
        true
    }
    #[cfg(not(test))]
    {
        let Some(cwd) = cwd else {
            // No cwd → claude-code falls back to the parent's, which we
            // can't reproduce here. Be permissive: let `--resume` try and
            // surface its own error rather than masking it.
            return true;
        };
        let Some(home) = std::env::var_os("HOME") else {
            return true;
        };
        // claude-code encodes the project dir by replacing both `/` and
        // `.` with `-`. e.g. `/Users/jason/go/src/github.com/yicheng47`
        // → `-Users-jason-go-src-github-com-yicheng47`. Confirmed against
        // `~/.claude/projects/` directory listings. Only swapping `/`
        // would miss every cwd containing a `.` (most repos), causing
        // `path.exists()` to return false even when the conversation
        // file is on disk — every resume would then spuriously fall back
        // to a fresh spawn.
        let encoded: String = cwd
            .chars()
            .map(|c| if c == '/' || c == '.' { '-' } else { c })
            .collect();
        let path = std::path::PathBuf::from(home)
            .join(".claude")
            .join("projects")
            .join(encoded)
            .join(format!("{uuid}.jsonl"));
        path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_returns_no_argv_for_system_prompt() {
        // claude-code's --append-system-prompt is SDK-only; the
        // interactive TUI ignores it. The argv path returns empty,
        // and SessionManager::schedule_first_prompt delivers the
        // prompt as a first user turn via stdin instead.
        let args = system_prompt_args("claude-code", Some("be helpful"));
        assert!(args.is_empty());
    }

    #[test]
    fn codex_runtime_returns_no_argv_for_system_prompt() {
        // Codex's positional `[PROMPT]` argv races codex's startup
        // permission / approval dialog (the prompt gets swallowed,
        // replayed stale, or misordered when the TUI shows the dialog
        // before hitting its main loop). The brief is now delivered via
        // stdin once the TUI has settled — see
        // `SessionManager::schedule_first_prompt` — so the argv path
        // returns empty for codex too.
        let args = system_prompt_args("codex", Some("be helpful"));
        assert!(
            args.is_empty(),
            "codex system_prompt is delivered via stdin, not positional argv: {args:?}",
        );
    }

    #[test]
    fn codex_runtime_omits_argv_when_prompt_is_blank() {
        assert!(system_prompt_args("codex", None).is_empty());
        assert!(system_prompt_args("codex", Some("")).is_empty());
        assert!(system_prompt_args("codex", Some("   ")).is_empty());
    }

    #[test]
    fn shell_runtime_omits_flag() {
        assert!(system_prompt_args("shell", Some("ignored")).is_empty());
    }

    #[test]
    fn missing_or_blank_prompt_omits_flag() {
        assert!(system_prompt_args("claude-code", None).is_empty());
        assert!(system_prompt_args("claude-code", Some("")).is_empty());
        assert!(system_prompt_args("claude-code", Some("   ")).is_empty());
    }

    #[test]
    fn unknown_runtime_degrades_to_no_flag() {
        assert!(system_prompt_args("aider-future", Some("hi")).is_empty());
    }

    #[test]
    fn claude_code_fresh_self_assigns_session_id() {
        let plan = resume_plan("claude-code", None);
        assert!(!plan.resuming);
        assert!(!plan.prepend);
        assert_eq!(plan.args.len(), 2);
        assert_eq!(plan.args[0], "--session-id");
        let assigned = plan.assigned_key.as_deref().unwrap();
        assert_eq!(plan.args[1], assigned);
        assert!(is_uuid(assigned), "assigned key must be a UUID");
    }

    #[test]
    fn claude_code_resumes_with_prior_uuid() {
        // `--resume <uuid>` is the right flag. `--session-id` would
        // be rejected as "already in use" because claude-code treats
        // it as fresh-only. `schedule_first_prompt` ensures the
        // conversation file exists before any resume attempt by
        // pushing a first user turn on initial spawn.
        let prior = uuid::Uuid::new_v4().to_string();
        let plan = resume_plan("claude-code", Some(&prior));
        assert!(plan.resuming);
        assert!(!plan.prepend);
        assert_eq!(plan.args, vec!["--resume", &prior]);
        assert_eq!(plan.assigned_key.as_deref(), Some(prior.as_str()));
    }

    #[test]
    fn claude_code_falls_back_to_fresh_on_invalid_prior_key() {
        // A non-UUID prior key would crash claude-code's --resume parser.
        // Treat it as missing and start fresh.
        let plan = resume_plan("claude-code", Some("not-a-uuid"));
        assert!(!plan.resuming);
        assert_eq!(plan.args[0], "--session-id");
    }

    #[test]
    fn codex_fresh_returns_empty_plan() {
        let plan = resume_plan("codex", None);
        assert!(plan.args.is_empty());
        assert!(plan.assigned_key.is_none());
        assert!(!plan.resuming);
    }

    #[test]
    fn codex_resume_uses_subcommand_prefix() {
        let prior = uuid::Uuid::new_v4().to_string();
        let plan = resume_plan("codex", Some(&prior));
        assert!(plan.resuming);
        assert!(plan.prepend, "codex resume is a subcommand, must prepend");
        assert_eq!(plan.args, vec!["resume", &prior]);
    }

    #[test]
    fn unknown_runtime_returns_empty_resume_plan() {
        let plan = resume_plan("aider-future", Some("anything"));
        assert!(plan.args.is_empty());
        assert!(plan.assigned_key.is_none());
        assert!(!plan.resuming);
    }

    #[test]
    fn claude_code_emits_model_and_effort_flags() {
        let args = model_effort_args("claude-code", Some("claude-opus-4-7"), Some("xhigh"));
        assert_eq!(
            args,
            vec![
                "--model".to_string(),
                "claude-opus-4-7".to_string(),
                "--effort".to_string(),
                "xhigh".to_string(),
            ]
        );
    }

    #[test]
    fn codex_emits_model_and_reasoning_effort_override() {
        // Issue #41: codex was silently dropping `effort`. Codex has no
        // dedicated reasoning-effort flag; the canonical wiring is via
        // its `-c key=value` config-override flag using the same
        // `model_reasoning_effort` key as `~/.codex/config.toml`.
        let args = model_effort_args("codex", Some("gpt-5-codex"), Some("high"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "gpt-5-codex"),
            "expected --model flag, got: {args:?}",
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
            "expected `-c model_reasoning_effort=high`, got: {args:?}",
        );
    }

    #[test]
    fn codex_emits_only_model_when_effort_unset() {
        let args = model_effort_args("codex", Some("gpt-5-codex"), None);
        assert_eq!(args, vec!["--model".to_string(), "gpt-5-codex".to_string()]);
    }

    #[test]
    fn codex_lowercases_effort_for_case_sensitive_toml_enum() {
        // Codex's `model_reasoning_effort` is a case-sensitive TOML
        // enum and rejects "High" with `unknown variant 'High',
        // expected one of 'none', 'minimal', 'low', 'medium', 'high',
        // 'xhigh'`. Rows often store the level title-cased ("High"),
        // so the codex branch normalises before forwarding.
        let args = model_effort_args("codex", Some("gpt-5-codex"), Some("High"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
            "expected lowercased effort override, got: {args:?}",
        );
    }

    #[test]
    fn codex_lowercases_mixed_case_effort() {
        let args = model_effort_args("codex", None, Some("XHIGH"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=xhigh"),
            "expected lowercased effort override, got: {args:?}",
        );
    }

    #[test]
    fn claude_code_forwards_effort_verbatim() {
        // Asymmetric on purpose: claude-code's `--effort` is case-
        // insensitive (accepts `High`), so we forward the row's
        // value verbatim rather than risk regressing already-shipped
        // behavior. Only the codex branch normalises.
        let args = model_effort_args("claude-code", None, Some("High"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--effort" && w[1] == "High"),
            "expected verbatim effort for claude-code, got: {args:?}",
        );
    }

    #[test]
    fn codex_trailing_args_omit_positional_prompt() {
        // Codex's positional `[PROMPT]` argv races codex's startup
        // permission / approval dialog, so we deliver the brief via
        // stdin instead (see `SessionManager::schedule_first_prompt`).
        // The trailing args MUST NOT include the brief as positional
        // argv on either fresh or resume spawns. Model/effort flags
        // still ride along so the runner row's pinned settings reach
        // the spawned CLI.
        for plan_resuming in [false, true] {
            // `system_prompt` (the role/persona stub) still rides via
            // stdin for both runtimes — `system_prompt_args` returns
            // empty. The first-user-turn body is delivered separately
            // via `first_turn` (spawn-time positional argv) per
            // `docs/impls/0007-spawn-time-prompt-delivery.md`.
            let args = trailing_runtime_args(
                "codex",
                plan_resuming,
                Some("gpt-5-codex"),
                Some("high"),
                Some("be helpful"),
                None,
            );
            assert!(
                !args.iter().any(|a| a == "be helpful"),
                "codex trailing args must not contain the brief as positional argv \
                 (plan_resuming={plan_resuming}): {args:?}",
            );
            assert!(
                args.windows(2)
                    .any(|w| w[0] == "--model" && w[1] == "gpt-5-codex"),
                "expected --model flag to survive (plan_resuming={plan_resuming}): {args:?}",
            );
            assert!(
                args.windows(2)
                    .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
                "expected reasoning-effort override to survive \
                 (plan_resuming={plan_resuming}): {args:?}",
            );
        }
    }

    #[test]
    fn permission_mode_args_per_runtime() {
        // Default → no flags for any runtime / any mode.
        assert!(permission_mode_args("claude-code", PermissionMode::Default).is_empty());
        assert!(permission_mode_args("codex", PermissionMode::Default).is_empty());
        // claude-code: AcceptEdits / Auto / Bypass each emit
        // `--permission-mode <value>` with a runtime-specific value.
        assert_eq!(
            permission_mode_args("claude-code", PermissionMode::AcceptEdits),
            vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
        );
        assert_eq!(
            permission_mode_args("claude-code", PermissionMode::Auto),
            vec!["--permission-mode".to_string(), "auto".to_string()],
        );
        assert_eq!(
            permission_mode_args("claude-code", PermissionMode::Bypass),
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );
        // codex: AcceptEdits has no equivalent (returns empty);
        // Auto uses on-request (on-failure is deprecated per
        // `codex --help`); Bypass uses never.
        assert!(permission_mode_args("codex", PermissionMode::AcceptEdits).is_empty());
        assert_eq!(
            permission_mode_args("codex", PermissionMode::Auto),
            vec![
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );
        assert_eq!(
            permission_mode_args("codex", PermissionMode::Bypass),
            vec![
                "--ask-for-approval".to_string(),
                "never".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );
        // Unknown runtime → empty for every mode.
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            assert!(permission_mode_args("shell", mode).is_empty());
            assert!(permission_mode_args("aider-future", mode).is_empty());
        }
    }

    #[test]
    fn apply_permission_mode_codex_appends_auto_pair() {
        let user = vec!["--debug".to_string(), "-v".to_string()];
        let out = apply_permission_mode("codex", &user, PermissionMode::Auto);
        assert_eq!(
            out,
            vec![
                "--debug".to_string(),
                "-v".to_string(),
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
            "user-provided args come first; canonical auto pair appended",
        );
    }

    #[test]
    fn apply_permission_mode_codex_appends_bypass_pair() {
        let user = vec!["--debug".to_string()];
        let out = apply_permission_mode("codex", &user, PermissionMode::Bypass);
        assert_eq!(
            out,
            vec![
                "--debug".to_string(),
                "--ask-for-approval".to_string(),
                "never".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );
    }

    #[test]
    fn apply_permission_mode_codex_accept_edits_is_no_op() {
        // Codex has no edits-only middle; AcceptEdits maps to the
        // empty arg list, so applying it strips any pre-existing
        // permission flags but doesn't write codex's pair.
        let user = vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "on-request".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ];
        let out = apply_permission_mode("codex", &user, PermissionMode::AcceptEdits);
        assert_eq!(
            out,
            vec!["--debug".to_string()],
            "AcceptEdits on codex strips permission flags and adds none",
        );
    }

    #[test]
    fn apply_permission_mode_codex_dedupes_existing_flags() {
        // Cycling between modes with stale/conflicting flags already
        // in args must replace them with the canonical pair, not
        // stack duplicates. Covers both `--flag value` and
        // `--flag=value` shapes plus an unrelated user arg in the
        // middle.
        let user = vec![
            "--ask-for-approval".to_string(),
            "untrusted".to_string(),
            "--debug".to_string(),
            "--sandbox=read-only".to_string(),
        ];
        let out = apply_permission_mode("codex", &user, PermissionMode::Bypass);
        assert_eq!(
            out,
            vec![
                "--debug".to_string(),
                "--ask-for-approval".to_string(),
                "never".to_string(),
                "--sandbox".to_string(),
                "workspace-write".to_string(),
            ],
        );
    }

    #[test]
    fn apply_permission_mode_codex_default_strips_all_flags() {
        let user = vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ];
        let out = apply_permission_mode("codex", &user, PermissionMode::Default);
        assert_eq!(out, vec!["--debug".to_string()]);
    }

    #[test]
    fn apply_permission_mode_claude_code_each_mode() {
        for (mode, expected_extra) in [
            (
                PermissionMode::AcceptEdits,
                vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
            ),
            (
                PermissionMode::Auto,
                vec!["--permission-mode".to_string(), "auto".to_string()],
            ),
            (
                PermissionMode::Bypass,
                vec![
                    "--permission-mode".to_string(),
                    "bypassPermissions".to_string(),
                ],
            ),
        ] {
            let user = vec!["--mcp-debug".to_string()];
            let out = apply_permission_mode("claude-code", &user, mode);
            let mut want = vec!["--mcp-debug".to_string()];
            want.extend(expected_extra);
            assert_eq!(out, want, "mode={mode:?}");
        }
    }

    #[test]
    fn apply_permission_mode_claude_code_cycles_cleanly() {
        // AcceptEdits → Auto → Bypass → Default with a custom flag
        // in the middle. Each transition must end with the canonical
        // args for the chosen mode, never an accumulation.
        let mut args = vec!["--mcp-debug".to_string()];
        args = apply_permission_mode("claude-code", &args, PermissionMode::AcceptEdits);
        assert_eq!(
            args,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "acceptEdits".to_string(),
            ],
        );
        args = apply_permission_mode("claude-code", &args, PermissionMode::Auto);
        assert_eq!(
            args,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
            ],
            "cycling to Auto must replace the prior --permission-mode value, not stack",
        );
        args = apply_permission_mode("claude-code", &args, PermissionMode::Bypass);
        assert_eq!(
            args,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        );
        args = apply_permission_mode("claude-code", &args, PermissionMode::Default);
        assert_eq!(args, vec!["--mcp-debug".to_string()]);
    }

    #[test]
    fn apply_permission_mode_claude_code_strips_legacy_dangerous_flag() {
        // Pre-rename rows carried `--dangerously-skip-permissions`
        // for the Bypass state. The strip helper must drop it on
        // any mode change so the row converges to the new
        // `--permission-mode <value>` shape rather than carrying
        // both side-by-side.
        let user = vec![
            "--mcp-debug".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let out = apply_permission_mode("claude-code", &user, PermissionMode::Bypass);
        assert_eq!(
            out,
            vec![
                "--mcp-debug".to_string(),
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
            "legacy flag stripped; canonical --permission-mode bypassPermissions added",
        );
    }

    #[test]
    fn apply_permission_mode_no_op_for_unsupported_runtime() {
        let user = vec!["--whatever".to_string()];
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            assert_eq!(
                apply_permission_mode("shell", &user, mode),
                user,
                "shell must be a no-op (mode={mode:?})",
            );
        }
    }

    #[test]
    fn infer_permission_mode_codex_separated_form() {
        let args = vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ];
        assert_eq!(
            infer_permission_mode("codex", &args),
            PermissionMode::Bypass,
        );
    }

    #[test]
    fn infer_permission_mode_codex_equals_form() {
        // Equals-form must match too — same bug coverage as the
        // pre-rewrite test that locked in the frontend hand-port.
        let args = vec![
            "--ask-for-approval=never".to_string(),
            "--sandbox=workspace-write".to_string(),
        ];
        assert_eq!(
            infer_permission_mode("codex", &args),
            PermissionMode::Bypass,
        );
    }

    #[test]
    fn infer_permission_mode_codex_auto_pair() {
        let args = vec![
            "--ask-for-approval".to_string(),
            "on-request".to_string(),
            "--sandbox=workspace-write".to_string(),
        ];
        assert_eq!(infer_permission_mode("codex", &args), PermissionMode::Auto);
    }

    #[test]
    fn infer_permission_mode_codex_partial_match_falls_back_to_default() {
        // --sandbox is present, --ask-for-approval is not. Neither
        // pair fully matches → default.
        let args = vec!["--sandbox=workspace-write".to_string()];
        assert_eq!(
            infer_permission_mode("codex", &args),
            PermissionMode::Default,
        );
    }

    #[test]
    fn infer_permission_mode_codex_deprecated_value_falls_back_to_default() {
        // `on-failure` is deprecated per `codex --help` and not
        // exposed in the dropdown. A row carrying it (e.g. created
        // by an older Runner build) reads as Default — neither
        // Auto nor Bypass match — so the dropdown lands on Default
        // and a save converges the row to one of the new canonical
        // shapes.
        let args = vec![
            "--ask-for-approval=on-failure".to_string(),
            "--sandbox=workspace-write".to_string(),
        ];
        assert_eq!(
            infer_permission_mode("codex", &args),
            PermissionMode::Default,
        );
    }

    #[test]
    fn infer_permission_mode_claude_code_each_state() {
        assert_eq!(
            infer_permission_mode("claude-code", &["--mcp-debug".into()]),
            PermissionMode::Default,
        );
        assert_eq!(
            infer_permission_mode(
                "claude-code",
                &["--permission-mode".into(), "acceptEdits".into()],
            ),
            PermissionMode::AcceptEdits,
        );
        assert_eq!(
            infer_permission_mode("claude-code", &["--permission-mode=acceptEdits".into()]),
            PermissionMode::AcceptEdits,
        );
        assert_eq!(
            infer_permission_mode("claude-code", &["--permission-mode".into(), "auto".into()],),
            PermissionMode::Auto,
        );
        assert_eq!(
            infer_permission_mode(
                "claude-code",
                &["--permission-mode".into(), "bypassPermissions".into()],
            ),
            PermissionMode::Bypass,
        );
    }

    #[test]
    fn infer_permission_mode_claude_code_legacy_dangerous_flag_reads_as_bypass() {
        // Pre-rename rows used `--dangerously-skip-permissions` for
        // Bypass. The dropdown must still load Bypass for those rows
        // so a save converges them to `--permission-mode
        // bypassPermissions`.
        let args = vec!["--dangerously-skip-permissions".to_string()];
        assert_eq!(
            infer_permission_mode("claude-code", &args),
            PermissionMode::Bypass,
        );
    }

    #[test]
    fn infer_permission_mode_claude_code_bypass_wins_over_accept_edits() {
        // A row carrying both `--permission-mode acceptEdits` AND
        // the legacy `--dangerously-skip-permissions` flag resolves
        // to Bypass — bypass is strictly more aggressive, and the
        // strip-and-replace round-trip on save converges the row to
        // a single canonical pair.
        let args = vec![
            "--permission-mode".to_string(),
            "acceptEdits".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        assert_eq!(
            infer_permission_mode("claude-code", &args),
            PermissionMode::Bypass,
        );
    }

    #[test]
    fn infer_permission_mode_unsupported_runtime_default() {
        let args = vec!["--whatever".to_string()];
        assert_eq!(
            infer_permission_mode("shell", &args),
            PermissionMode::Default,
        );
        assert_eq!(
            infer_permission_mode("aider-future", &args),
            PermissionMode::Default,
        );
    }

    #[test]
    fn strip_permission_flags_handles_dangling_value() {
        // If `--ask-for-approval` is the last token (no value follows
        // — the user mid-typed), strip just the flag and don't panic
        // on the missing pair.
        let user = vec!["--debug".to_string(), "--ask-for-approval".to_string()];
        let out = strip_permission_flags("codex", &user);
        assert_eq!(out, vec!["--debug".to_string()]);
    }

    #[test]
    fn strip_permission_flags_drops_claude_code_permission_mode() {
        // Cycling through modes shouldn't leave orphan
        // `--permission-mode acceptEdits` when the user picks Bypass
        // or Default afterward.
        let user = vec![
            "--permission-mode".to_string(),
            "acceptEdits".to_string(),
            "--debug".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let out = strip_permission_flags("claude-code", &user);
        assert_eq!(out, vec!["--debug".to_string()]);
    }

    #[test]
    fn claude_code_trailing_args_unaffected_by_resume_flag_when_first_turn_absent() {
        // claude-code's `system_prompt_args` is empty (the persona
        // stub rides via stdin). With `first_turn = None`, the
        // `plan_resuming` flag has no effect — the trailing args
        // are just the model/effort pair.
        let fresh = trailing_runtime_args(
            "claude-code",
            false,
            Some("claude-opus-4-7"),
            Some("xhigh"),
            Some("be helpful"),
            None,
        );
        let resuming = trailing_runtime_args(
            "claude-code",
            true,
            Some("claude-opus-4-7"),
            Some("xhigh"),
            Some("be helpful"),
            None,
        );
        assert_eq!(fresh, resuming);
        assert_eq!(
            fresh,
            vec![
                "--model".to_string(),
                "claude-opus-4-7".to_string(),
                "--effort".to_string(),
                "xhigh".to_string(),
            ]
        );
    }

    #[test]
    fn first_turn_rides_trailing_argv_on_fresh_spawn_for_both_runtimes() {
        for runtime in ["claude-code", "codex"] {
            let body = "You are the architect. Goal: ship 0007.";
            let args = trailing_runtime_args(
                runtime,
                false,
                Some("model-x"),
                Some("high"),
                Some("persona"),
                Some(body),
            );
            // Body lands as the trailing positional after model/effort
            // flags. system_prompt is unchanged (still empty) for both
            // runtimes.
            assert_eq!(args.last().map(String::as_str), Some(body));
            assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "model-x"));
        }
    }

    #[test]
    fn first_turn_suppressed_on_resume_for_both_runtimes() {
        for runtime in ["claude-code", "codex"] {
            let body = "You are the architect. Goal: ship 0007.";
            let args = trailing_runtime_args(
                runtime,
                true,
                Some("model-x"),
                Some("high"),
                Some("persona"),
                Some(body),
            );
            assert!(
                !args.iter().any(|a| a == body),
                "resume must not replay the first-turn body as positional argv ({runtime}): {args:?}"
            );
        }
    }

    // The pre-#88 `first_turn_argv_skipped_when_body_exceeds_arg_max_threshold`
    // test exercised the post-spawn paste fallback for oversized bodies.
    // Plan 0007 retired that fallback in favour of persistence-layer
    // validation (`commands::runner::MAX_SYSTEM_PROMPT_BYTES` /
    // `commands::mission::MAX_MISSION_GOAL_BYTES`). The debug_assert
    // inside `first_turn_argv` is now defense-in-depth — exercising it
    // here would just trip the assertion. Validation tests live in
    // `commands::runner` / `commands::mission` / `commands::crew`.

    #[test]
    fn first_turn_argv_empty_for_blank_or_unsupported_runtime() {
        assert!(first_turn_argv("claude-code", Some("   \n  ")).is_empty());
        assert!(first_turn_argv("claude-code", None).is_empty());
        assert!(first_turn_argv("shell", Some("body")).is_empty());
        assert!(first_turn_argv("unknown", Some("body")).is_empty());
    }
}
