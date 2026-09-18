pub fn print(topic: Option<&str>) {
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
  runner session list|resume|restart
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
