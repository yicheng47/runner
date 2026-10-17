# Drive Runner from agents

Use the `runner` command to start chats and missions, follow work, and send instructions from a shell or another agent. The command connects to Runner's local background service, so the app window can be closed.

## Discover the installed version

[Install the command](../getting-started/install.md#make-the-runner-command-available), then run:

```sh
runner help
runner help agents
runner crew list --json
runner role list --json
```

Runner installs a skill for detected agents when **Settings → General → Command line → Runner skill for agents** is enabled. The skill directs the agent to `runner help agents`. Ask your agent to read that guide before delegating work; its instructions match the local binary.

Roles use their unique handles. Crews and projects accept an ID or exact name. Missions and direct-chat sessions accept an ID or unique prefix. Add `--json` for structured output or `-q` for only the result ID; the flags are mutually exclusive.

## Start and follow a crew

Write a UTF-8 `brief.md` containing the task, directory assumptions, validation and authorized Git actions. In a macOS shell, run:

```sh
mission=$(runner mission start --crew "Pair coding crew" --cwd /absolute/path/to/project --goal-file brief.md -q)
runner mission feed "$mission" --follow --json
```

In Windows PowerShell:

```powershell
$mission = runner mission start --crew "Pair coding crew" --cwd 'C:\Users\you\src\project' --goal-file brief.md -q
if ($LASTEXITCODE -ne 0) { throw 'Mission start failed' }
runner mission feed $mission --follow --json
```

Replace the crew name and directory with yours. `--goal-file -` reads standard input; a file path avoids shell-specific input-redirection differences. Use `--project "My app"` instead of `--cwd` when the project already names the folder.

The follower displays new events until the mission ends or all its sessions exit. Pressing **Ctrl+C** ends your follower; it does not stop the mission. Leave it running in one terminal and use another for commands.

An agent that starts a mission must arrange a watch whose output and exit notifications reach it during later work and while idle. One unread background log is not a watch. If the host cannot deliver those events, the agent should say so and give you the feed command to run. On an interrupted watch, check the mission and resume from the last JSON event's `next_offset` using `--since <offset> --oldest-first`. See the generated [agent guide](../reference/cli.md#agent-guide) for the full watch contract.

## Send a message and answer a question

Use `mission show` to find the current handles, lead and pending questions. The following examples use placeholders you replace with those values; the command syntax is the same in macOS shells and PowerShell:

```sh
runner mission show <mission-id> --json
runner msg post --mission <mission-id> --to coder "Keep the change limited to the parser."
runner mission answer <mission-id> <question-id> <choice>
runner mission stop <mission-id>
runner mission archive <mission-id>
```

Inside a mission, members can use `runner msg post --to reviewer "Ready for review"` and `runner msg read` without supplying a mission ID. Outside a mission, writes need `--mission`. An outside caller without `--as` speaks as the person; do not use `--as` to impersonate another member.

Address several members with `runner msg post --to coder --to reviewer "Please check the parser."`, or use leading mentions: `runner msg post "@coder @reviewer Please check the parser."`. Explicit `--to` recipients take precedence. Mentions inside the message do not address it. Unknown handles and `@human` are refused; `mission show` lists valid members. Omit recipients to broadcast when everyone needs to act, and reply to the sender for ordinary follow-ups.

When an `[inbox]` notification arrives, run `runner msg read`. It shows only new inbox messages, followed by up to ten short lines of other members' directed traffic. After a resume or whenever earlier messages are no longer in context, use `runner msg read --all` for all crew messages since joining, including sent messages and traffic between others. Unfiltered reads mark only printed inbox messages read. `--since <event-id>` and `--from <handle>` never mark messages read, even with `--all`, and omit the digest. Reads also work while the daemon is unavailable. Do not send acknowledgement or standby messages.

## Start without a saved crew

Create an empty mission and add workers individually:

```sh
runner mission start --title "Small task" --cwd /absolute/path/to/project -q
runner mission add-role <returned-mission-id> coder --task-file task.md
runner mission add-role <returned-mission-id> reviewer --task "Review the coder's diff after its handoff."
```

Arm the feed watch for the returned ID as above. These members join as workers with no lead. Adding and removing roles is controlled by the person, not by an in-mission agent. The full [CLI reference](../reference/cli.md) lists overrides and lifecycle commands.

## Exit codes

| Code | Meaning | Next step |
| --- | --- | --- |
| `0` | Success | Read the result |
| `1` | Runner refused the request, a protocol error occurred, or a watch failed | Read stderr; a watch failure does not prove the mission stopped |
| `2` | Invalid usage or reference | Read that command's `--help` and check names or IDs |
| `3` | Runner is not running or could not be reached as a running service | Open Runner and retry |
| `5` | A sandbox blocked access | Retry the same command outside the sandbox with the host's permission mechanism |

Exit `5` does not establish whether Runner is running. A sandboxed agent should request access for the same command before diagnosing the app.
