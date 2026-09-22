pub fn print(topic: Option<&str>) {
    if topic == Some("agents") {
        println!("{AGENT_GUIDE}");
        return;
    }
    if let Some(topic) = topic {
        println!("runner help {topic}: use `runner {topic} --help` for the command reference.");
        return;
    }
    println!(
        r#"runner — operate Runner from a shell or a mission session

USAGE
  runner status
  runner project list|show|create|rename|delete
  runner role list|show|create|update|delete
  runner crew list|show|create|update|delete|add|set|remove|lead|order
  runner mission list|show|start|stop|resume|archive|unarchive|rename|pin|unpin|move|feed|answer
  runner chat start
  runner session list|show|stop|archive|resume|restart
  runner msg post|read
  runner signal <type>
  runner ask <question> | runner ask --human <prompt> --choices <a,b,...>
  runner call <tool> [<json>]

OUTPUT
  --json   print the JSON result
  -q       print only result ids

CONTEXT
  Inside a mission, msg post/read, signal, and ask use the event log directly.
  Outside, mission-scoped writes require --mission; --as names a roster handle.

RESERVED FOR #562
  spawn, ps, wait, stop <handle>, done
"#
    );
}

#[cfg(test)]
pub const AGENT_TOP_LEVEL_COMMANDS: &[&str] = &[
    "status", "project", "role", "crew", "mission", "chat", "session", "msg", "signal", "ask",
    "call", "help",
];

pub const AGENT_GUIDE: &str = r#"runner — version-matched guide for agents

DISCOVER
  runner status
  runner project --help
  runner role --help
  runner crew --help
  runner mission --help
  runner chat --help
  runner session --help
  runner msg --help
  runner signal --help
  runner ask --help
  runner call --help
  runner help

OUTPUT AND EXITS
  Add --json for structured output. Add -q when only the changed object's id is needed.
  0 success; 1 Runner refused or watch failed; 2 usage or reference error; 3 Runner is not running;
  5 a sandbox kept this command from reaching Runner.
  On exit 3, ask the user to open Runner. On exit 5, nothing is known about Runner yet:
  run the same command again outside the sandbox (in Codex, request escalated permissions
  for it) before telling the user anything.
  Use --help instead of guessing a command.

REFERENCES
  Roles use their unique handle. Crews and projects accept an id or exact name.
  Missions and direct-chat sessions accept an id or unique id prefix.
  A mission or chat started inside a project's directory belongs to it; --project overrides.

COMMON FLOW
  runner crew list --json
  mission=$(runner mission start --crew <crew> --goal-file - -q < brief.md)
  runner mission feed "$mission" --follow --json
  runner mission show "$mission" --json
  runner mission answer "$mission" <question_id_from_mission_show> <choice>
  # Use the HANDLE from the LEAD row in mission show.
  runner msg post --mission "$mission" --to <lead_handle> "message"
  runner mission stop "$mission"
  runner mission archive "$mission"

START AND WATCH (REQUIRED)
  After every successful runner mission start, arm runner mission feed <id> --follow --json
  for the exact returned ID in your host's supported background/watch facility before
  reporting delegation complete. Keep exactly one watcher per mission; reuse it on later
  turns. Events, stderr diagnostics and process exit must reach you during later work and
  while idle. An unread background log or a completion-only notification is not a watch.
  Plain mission start still returns its result and exits; it does not block on watching.
  Surface handoffs, human questions, completion, stop/crash and lost connections promptly.
  Polling is every 3 seconds; each event is flushed immediately, without notification batching.
  Routine busy/idle and inbox noise stays hidden by default; do not add --all for a watch.
  The follower ends on archive, completed/aborted mission state, or all sessions exiting.
  Busy/idle and a crew message saying "done" do not end a live mission. Resume needs a new watch.
  A watch request timeout after 30 seconds exits 1; Runner may still be running.
  Clean up on exit. On host expiry or unexpected exit, report the gap, check mission show,
  and re-arm one watch if still active. Use the recovery --since cursor when available;
  otherwise re-arm with --since 0 --oldest-first and deduplicate event IDs.
  Never silently abandon a watch. Stop any undeliverable follower before ending your turn.
  If your host cannot deliver background events, tell the user plainly:
    "Mission <id> started, but this host cannot notify me of later events;
     automatic watching is unavailable. Follow with: runner mission feed <id> --follow --json"
  Do not claim monitoring is active without a verified delivery path.

HOST NOTES (installed documentation checked 2026-09-22; availability varies by session)
  Claude Code 2.1.278: the embedded Monitor tool documentation says each stdout line becomes
  a notification and exit ends the watch. When Monitor is available, run the feed command
  with 2>&1 so stderr failures also notify you. Handle Monitor expiry by re-arming as above.
  Bash run_in_background only notifies on completion and is insufficient for a live feed.
  Codex CLI 0.155.1: embedded exec_command/write_stdin tool help documents a running session
  and explicit output reads. These alone establish active-turn consumption, not idle delivery.
  Copilot CLI 1.0.87: copilot help commands documents /tasks for subagents and shell commands;
  it does not establish per-line delivery to an idle agent. Do not infer it from /tasks.
  pi 0.85.1: installed README.md says no background bash. docs/extensions.md documents
  pi.sendMessage with triggerTurn: true and deliverAs: "followUp" or "steer"; only an already
  available extension that forwards feed output, errors and exit can provide that watch.
  For Codex, Copilot or pi without a verified event-delivery facility, use the explicit
  limitation path above. Do not install extensions or change agent configuration to claim support.

IDENTITY
  Inside a mission, mission commands carry the caller's own handle.
  Outside a mission, --as <handle> names a roster slot and is refused if that handle is not in the roster.
  Outside a mission with no --as, the caller is the person at the app.
  Do not use --as to speak as another agent's slot; it exists for a slot you hold.
"#;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn agent_guide_names_every_real_top_level_command_and_no_others() {
        let clap = crate::command::Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect::<BTreeSet<_>>();
        let guide = AGENT_TOP_LEVEL_COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(guide, clap);
        for command in guide {
            assert!(
                AGENT_GUIDE.contains(&format!("runner {command}")),
                "agent guide omits runner {command}"
            );
        }
        assert!(AGENT_GUIDE
            .contains("runner msg post --mission \"$mission\" --to <lead_handle> \"message\""));
        assert!(!AGENT_GUIDE.contains("--as <roster_handle>"));
    }

    #[test]
    fn agent_guide_requires_a_delivering_watch_after_start() {
        for rule in [
            "After every successful runner mission start",
            "runner mission feed <id> --follow --json",
            "exactly one watcher per mission",
            "while idle",
            "An unread background log or a completion-only notification is not a watch",
            "automatic watching is unavailable",
            "On host expiry or unexpected exit",
            "stderr diagnostics and process exit",
        ] {
            assert!(AGENT_GUIDE.contains(rule), "missing watch rule: {rule}");
        }
    }
}
