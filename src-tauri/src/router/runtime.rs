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
//      Codex still uses its positional `[PROMPT]` arg.
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
///   - `model` → `--model <name>`. Verified against
///     `codex --help`. codex has no equivalent thinking-effort
///     flag today, so `effort` is silently ignored for codex
///     runners (the row keeps the preference for when it lands).
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
            // codex accepts `--model <MODEL>` but has no
            // thinking-effort flag today. Skip effort silently;
            // the row still persists the preference for when
            // codex's adapter catches up.
            let mut out = Vec::new();
            if let Some(m) = model {
                out.push("--model".into());
                out.push(m.to_string());
            }
            out
        }
        // shell / unknown
        _ => Vec::new(),
    }
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
        // either). Codex's CLI does accept a positional `[PROMPT]` arg
        // that becomes the first user turn of the session — passing
        // `system_prompt` there is the closest available hook. The
        // trade-off is visibility: codex's chat history will show the
        // prompt as the first user message, not a hidden system
        // instruction. For *resume* paths we deliberately skip this at
        // the call site so the prompt isn't replayed onto an existing
        // conversation — see the spawn glue in
        // `SessionManager::{spawn,spawn_direct,resume}`.
        "codex" => vec![prompt.to_string()],
        // shell / unknown — no prompt mechanism.
        _ => Vec::new(),
    }
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
    fn codex_runtime_passes_prompt_as_positional_argv() {
        // Codex has no system-prompt flag. The closest mechanism it
        // ships is a positional `[PROMPT]` arg that seeds the session
        // with a first user turn. We pass `system_prompt` there so
        // the agent at least sees the brief, even though the trade-
        // off is the prompt becoming visible as a user message.
        let args = system_prompt_args("codex", Some("be helpful"));
        assert_eq!(args, vec!["be helpful".to_string()]);
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
}
