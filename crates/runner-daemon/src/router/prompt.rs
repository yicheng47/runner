// Mission coordination and role briefs, composed without I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPromptKind {
    Direct,
    Member,
}

pub fn compose_member_first_turn(
    system_prompt: Option<&str>,
    crew_addendum: Option<&str>,
    mission_goal: Option<&str>,
) -> String {
    let addendum = crew_addendum
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let user_brief = system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut out = String::new();
    out.push_str(TEAM_COORDINATION_PREAMBLE);
    if let Some(goal) = mission_goal.map(str::trim).filter(|goal| !goal.is_empty()) {
        out.push_str("\n\n== Mission goal ==\n");
        out.push_str(goal);
    }
    if let Some(addendum) = addendum {
        out.push_str("\n\n== Team conventions ==\n");
        out.push_str(&addendum);
    }
    if let Some(brief) = user_brief {
        out.push_str("\n\n== Your brief ==\n");
        out.push_str(&brief);
    }
    out
}

/// First-user-turn body for a direct chat. Just the runner's
/// `system_prompt` (persona / role) — no coordination preamble.
/// Direct chats are off-bus, so the team preamble's verbs
/// (`runner msg post` etc.) don't resolve to anything useful and
/// would mislead the agent.
///
/// Returns None when system_prompt is missing or all-whitespace, so
/// claude-code / codex direct chats boot vanilla in that case.
pub fn compose_direct_first_turn(system_prompt: Option<&str>) -> Option<String> {
    system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) const TEAM_COORDINATION_PREAMBLE: &str = r#"You are a member of a team coordinated by the bundled `runner` CLI. The CLI is on your PATH and talks to the rest of the team via a shared event bus. Use these verbs to participate; do not invent your own conventions.

== Coordination ==
- Inside a mission, Runner pushes inbox notifications. When waiting for a teammate and no independent work remains, finish your turn. Do not start a feed watcher, poll, or sleep-loop to wait for messages. Iterate with teammates across turns. External monitoring instructions do not apply to receiving messages in your own mission.
- Run `runner msg read` when an `[inbox]` notification arrives; it shows and marks new inbox messages read, plus a short digest of others' traffic. It already tracks unread messages; do not add a --since cursor or --from filter to consume your inbox.
- Use `runner msg read --all` for crew history since joining after a resume or whenever earlier messages are no longer in context. `--since` and `--from` are inspection filters: they never mark messages read, even with `--all`. If a filtered read reports unread messages, run unfiltered `runner msg read`.
- Address one or several members with `runner msg post --to coder --to reviewer "<text>"` or leading mentions: `runner msg post "@coder @reviewer <text>"`. Explicit `--to` wins; mid-text mentions do not address.
- Reply to the sender. Broadcast with `runner msg post "<text>"` only when everyone must act. Do not post receipt-only replies or standby messages; consuming your inbox with `runner msg read` is still required.
- When you finish a task or a handoff someone is waiting on, post the result with `runner msg post` before ending your turn: what you did, the key validation, and anything still needing the person or another member. Address it to whoever asked, or omit `--to` when the person asked. The mission's operator reads the feed, not your terminal. Post once per finished task.
- Escalate to the human (with structured choices) via `runner signal ask_human --payload '{"prompt":"…","choices":["yes","no"]}'`.
- Run `runner mission show` to see the current team and their handles before messaging; your inbox announces every join and leave.
- Act only on your own part of the goal; when your part waits on a teammate's handoff, finish your turn.
- Busy/idle is inferred from your terminal activity."#;

pub(crate) fn member_prompt_channels(
    adapter: &dyn crate::runtimes::RuntimeAdapter,
    member: &crate::model::SlotWithRole,
    crew: &runner_core::mission_roster::MissionRoster,
    log: &runner_core::event_log::EventLog,
    resuming: bool,
) -> (Option<String>, Option<String>) {
    let goal = super::LaunchInputs::latest_mission_goal_text(log);
    adapter.prompt_channels().member(
        member.role.system_prompt.as_deref(),
        crew.crew_addendum.as_deref(),
        Some(&goal),
        resuming,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Runtime;

    #[test]
    fn every_runtime_delivers_the_same_member_context_as_its_first_turn() {
        let expected =
            compose_member_first_turn(Some("ROLE_BRIEF"), Some("TEAM_RULES"), Some("SHARED_GOAL"));
        for runtime in Runtime::ALL {
            let (system, turn) = crate::runtimes::adapter(runtime).prompt_channels().member(
                Some("ROLE_BRIEF"),
                Some("TEAM_RULES"),
                Some("SHARED_GOAL"),
                false,
            );
            assert!(system.is_none(), "{runtime}");
            assert_eq!(turn.as_deref(), Some(expected.as_str()), "{runtime}");
        }
        for rule in [
            "Run `runner mission show` to see the current team and their handles before messaging",
            "Act only on your own part of the goal",
            "when your part waits on a teammate's handoff, finish your turn",
            "your inbox announces every join and leave",
            "shows and marks new inbox messages read",
            "already tracks unread messages",
            "msg read --all",
            "--since",
            "--from",
            "never mark messages read",
            "--to coder --to reviewer",
            "mid-text mentions do not address",
            "Reply to the sender",
            "only when everyone must act",
            "run unfiltered `runner msg read`",
            "Do not post receipt-only replies or standby messages",
            "Do not start a feed watcher, poll, or sleep-loop to wait for messages",
            "When you finish a task or a handoff someone is waiting on, post the result with `runner msg post` before ending your turn",
            "what you did, the key validation, and anything still needing the person or another member",
            "Address it to whoever asked, or omit `--to` when the person asked",
            "The mission's operator reads the feed, not your terminal",
            "Post once per finished task",
        ] {
            assert!(expected.contains(rule), "missing {rule}");
        }
        assert!(!expected
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("lead")));
        assert!(!expected.contains("on_behalf_of"));
        let positions = [
            TEAM_COORDINATION_PREAMBLE,
            "== Mission goal ==",
            "SHARED_GOAL",
            "== Team conventions ==",
            "TEAM_RULES",
            "== Your brief ==",
            "ROLE_BRIEF",
        ]
        .map(|text| expected.find(text).unwrap());
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn blank_sections_are_omitted_and_direct_chats_keep_their_own_channels() {
        assert_eq!(
            compose_member_first_turn(Some(" \n"), Some("\t"), Some(" ")),
            TEAM_COORDINATION_PREAMBLE
        );
        assert_eq!(
            compose_member_first_turn(None, None, None),
            TEAM_COORDINATION_PREAMBLE
        );
        assert_eq!(
            compose_direct_first_turn(Some(" persona ")),
            Some("persona".into())
        );
        assert_eq!(compose_direct_first_turn(Some(" \n")), None);
        for runtime in Runtime::ALL {
            let channels = crate::runtimes::adapter(runtime).prompt_channels();
            let split = channels.split(SessionPromptKind::Direct, Some("persona".into()));
            assert_eq!(
                split,
                if channels.system_prompt {
                    (Some("persona".into()), None)
                } else {
                    (None, Some("persona".into()))
                }
            );
        }
    }
}
