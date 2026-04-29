// Runtime adapter: maps `runner.runtime` to the extra CLI args the child
// process needs at spawn time.
//
// Two responsibilities live here:
//
//   1. `system_prompt_args` — pass `runner.system_prompt` to the agent CLI
//      via its native flag (claude-code: `--append-system-prompt`).
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
/// runner's configured `args` so the child receives `system_prompt` via the
/// runtime's native flag. Returns an empty Vec when no prompt is set or the
/// runtime is unrecognized — unrecognized runtimes degrade silently rather
/// than failing the spawn (the user might be prototyping a custom CLI).
pub fn system_prompt_args(runtime: &str, system_prompt: Option<&str>) -> Vec<String> {
    let prompt = match system_prompt {
        Some(p) if !p.trim().is_empty() => p,
        _ => return Vec::new(),
    };
    match runtime {
        // claude-code accepts --append-system-prompt <text>; the flag layers
        // onto its built-in default rather than replacing it, which is what
        // we want — we're appending the runner's brief, not overwriting.
        "claude-code" => vec!["--append-system-prompt".into(), prompt.to_string()],
        // codex / shell / unknown — no flag. We tried `codex --instructions`
        // first but the installed Codex CLI rejects it ("unexpected argument
        // --instructions found"). Until a verified prompt mechanism lands
        // (e.g. a documented flag on a pinned Codex version, or a wrapper
        // script convention), Codex runners spawn without the brief; the
        // prompt is still on the runner row and the user can configure
        // their own wrapper. Tracked for follow-up.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_appends_system_prompt() {
        let args = system_prompt_args("claude-code", Some("be helpful"));
        assert_eq!(args, vec!["--append-system-prompt", "be helpful"]);
    }

    #[test]
    fn codex_runtime_omits_flag_until_verified_mechanism() {
        // The installed `codex` CLI rejects `--instructions`. Until a
        // documented prompt flag is verified, the codex runtime degrades
        // to no-flag rather than crashing the spawn.
        assert!(system_prompt_args("codex", Some("be helpful")).is_empty());
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
