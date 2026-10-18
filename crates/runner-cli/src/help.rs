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
  runner daemon status|stop
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
  msg read optionally queries the existing daemon for a read cursor; offline reads still work.
  Outside, mission-scoped writes require --mission; --as names a roster handle.

RESERVED FOR #562
  spawn, ps, wait, stop <handle>, done
"#
    );
}

#[cfg(test)]
pub const AGENT_TOP_LEVEL_COMMANDS: &[&str] = &[
    "status", "project", "role", "crew", "mission", "chat", "session", "msg", "signal", "ask",
    "call", "help", "daemon",
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
  runner daemon --help
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

MESSAGING INSIDE YOUR MISSION
  Inside a mission, Runner pushes inbox notifications to every participant: leads, workers
  and members of a team with no lead. When waiting for a teammate and no independent work
  remains, finish your turn. Do not start a feed watcher, poll, or sleep-loop to wait for messages.
  Iterate with teammates across turns. External monitoring instructions do not apply to
  receiving messages in your own mission.
  Run runner msg read when an [inbox] notification arrives: only new inbox messages print,
  followed by a short digest of others' directed traffic. Use runner msg read --all for all
  crew messages since joining after a resume or when earlier messages are no longer in context.
  Default reads already track unread messages: do not add --since or --from to consume your inbox.
  --all acknowledges only your inbox. --since and --from are inspection filters and never mark
  messages read, even with --all. If unread messages remain, filtered reads report the count and
  an unfiltered-read hint on stderr, including when no messages match.
  Address one or several members:
    runner msg post --to coder --to reviewer "please check"
    runner msg post "@coder @reviewer please check"
  Explicit --to wins; mid-text mentions do not address. Unknown handles and @human are refused.
  Reply to the sender. Broadcast only when everyone must act. Do not post receipt-only replies
  or standby messages; consuming your inbox with runner msg read is still required.

EXTERNAL OPERATOR FLOW (A DIFFERENT MISSION)
  mission=$(runner mission start --title "Small task" -q)
  runner mission add-role "$mission" <role> --task-file - < task.md
  # Or start from a reusable crew:
  runner crew list --json
  mission=$(runner mission start --crew <crew> --goal-file - -q < brief.md)
  runner mission feed "$mission" --follow --json
  runner mission add-role "$mission" <role> --task-file - < task.md
  runner mission remove-role "$mission" <handle>
  runner mission show "$mission" --json
  runner mission answer "$mission" <question_id_from_mission_show> <choice>
  # Use the HANDLE from the LEAD row in mission show, or any member when there is no lead.
  runner msg post --mission "$mission" --to <lead_handle> "message"
  runner mission stop "$mission"
  runner mission archive "$mission"

EXTERNAL OPERATORS: START AND WATCH (REQUIRED)
  These watch rules apply when you launch a different mission as an external operator.
  That mission's updates are not automatically pushed into your chat.
  After every successful runner mission start, arm runner mission feed <id> --follow --json
  for the exact returned ID in your host's supported background/watch facility before
  reporting delegation complete. Keep exactly one watcher per mission; reuse it on later
  turns. Events, stderr diagnostics and process exit must reach you during later work and
  while idle. An unread background log or a completion-only notification is not a watch.
  Plain mission start still returns its result and exits; it does not block on watching.
  Surface handoffs, human questions, completion, stop/crash and lost connections promptly.
  The daemon pushes new events; each event is flushed immediately, without notification batching.
  Routine busy/idle and inbox noise stays hidden by default; do not add --all for a watch.
  The follower ends on archive, completed/aborted mission state, or all sessions exiting.
  Busy/idle and a crew message saying "done" do not end a live mission. Resume needs a new watch.
  A watch request timeout after 30 seconds exits 1; Runner may still be running.
  Each JSON event line carries next_offset, the cursor that resumes after that event.
  Clean up on exit. On host expiry or unexpected exit, report the gap, check mission show,
  and re-arm one watch if still active with --since <next_offset of the last event line
  you received> --oldest-first; a watch error prints its own resume command. Only when no event
  line arrived, re-arm with --since 0 --oldest-first and skip event IDs already seen.
  Never silently abandon a watch. Stop any undeliverable follower before ending your turn.
  If your host cannot deliver background events, tell the user plainly:
    "Mission <id> started, but this host cannot notify me of later events;
     automatic watching is unavailable. Follow with: runner mission feed <id> --follow --json"
  Do not claim monitoring is active without a verified delivery path.

EXTERNAL WATCH FACILITIES
  A facility qualifies only if its own documentation says each output line and the exit
  reach you while idle. Claude Code's Monitor does: run the feed command with 2>&1 so stderr
  diagnostics notify you too, set timeout_ms to its maximum, and re-arm on expiry as above.
  A completion-only background command, a PTY session you must poll, a task list or a log
  file does not qualify. Without a qualifying facility, take the limitation path above.
  Do not install extensions or change agent configuration to create one.

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
    fn agent_guide_explains_pushed_inbox_messaging_and_non_acking_filters() {
        for rule in [
            "Inside a mission, Runner pushes inbox notifications to every participant: leads, workers",
            "and members of a team with no lead",
            "When waiting for a teammate and no independent work\n  remains, finish your turn",
            "Do not start a feed watcher, poll, or sleep-loop to wait for messages",
            "Iterate with teammates across turns",
            "External monitoring instructions do not apply to\n  receiving messages in your own mission",
            "only new inbox messages",
            "msg read --all",
            "Default reads already track unread messages",
            "--since and --from are inspection filters and never mark",
            "messages read, even with --all",
            "unfiltered-read hint on stderr",
            "--to coder --to reviewer",
            "mid-text mentions do not address",
            "Do not post receipt-only replies",
            "consuming your inbox with runner msg read is still required",
        ] {
            assert!(AGENT_GUIDE.contains(rule), "missing {rule}");
        }
        assert!(
            AGENT_GUIDE.find("MESSAGING INSIDE YOUR MISSION").unwrap()
                < AGENT_GUIDE.find("EXTERNAL OPERATOR FLOW").unwrap()
        );
    }

    #[test]
    fn agent_guide_names_agent_commands_and_excludes_person_only_remote() {
        let clap = crate::command::Cli::command()
            .get_subcommands()
            .filter(|command| !command.is_hide_set() && command.get_name() != "remote")
            .map(|command| command.get_name().to_owned())
            .collect::<BTreeSet<_>>();
        let guide = AGENT_TOP_LEVEL_COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(guide, clap);
        assert!(!AGENT_GUIDE.contains("runner remote"));
        assert!(crate::command::Cli::command()
            .find_subcommand("hook")
            .unwrap()
            .is_hide_set());
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
    fn agent_guide_requires_a_delivering_watch_for_external_operators_after_start() {
        for rule in [
            "EXTERNAL OPERATOR FLOW (A DIFFERENT MISSION)",
            "These watch rules apply when you launch a different mission as an external operator",
            "That mission's updates are not automatically pushed into your chat",
            "EXTERNAL WATCH FACILITIES",
            "After every successful runner mission start",
            "runner mission feed <id> --follow --json",
            "exactly one watcher per mission",
            "while idle",
            "An unread background log or a completion-only notification is not a watch",
            "automatic watching is unavailable",
            "On host expiry or unexpected exit",
            "stderr diagnostics and process exit",
            "--since <next_offset of the last event line",
            "set timeout_ms to its maximum",
        ] {
            assert!(AGENT_GUIDE.contains(rule), "missing watch rule: {rule}");
        }
    }
}
