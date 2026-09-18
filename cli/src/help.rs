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
  0 success; 1 Runner refused; 2 usage or reference error; 3 Runner is not running.
  On exit 3, ask the user to open Runner. Use --help instead of guessing a command.

REFERENCES
  Roles use their unique handle. Crews and projects accept an id or exact name.
  Missions and direct-chat sessions accept an id or unique id prefix.

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
}
