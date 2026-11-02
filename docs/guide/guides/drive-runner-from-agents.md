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

The start-and-watch instructions in this section are for external operators launching a different mission. That mission's updates are not automatically pushed into an outside agent's chat. Participants receive inbox notifications for their own mission as described below.

Write a UTF-8 `brief.md` containing the task, directory assumptions, validation and authorized Git actions. In a macOS shell, run:

```sh
mission=$(runner mission start --crew "Pair coding crew" --title "Fix the login bug" --cwd /absolute/path/to/project --goal-file brief.md -q)
runner mission feed "$mission" --follow --json
```

In Windows PowerShell:

```powershell
$mission = runner mission start --crew "Pair coding crew" --title "Fix the login bug" --cwd 'C:\Users\you\src\project' --goal-file brief.md -q
if ($LASTEXITCODE -ne 0) { throw 'Mission start failed' }
runner mission feed $mission --follow --json
```

Replace the crew name, title and directory with yours. `--goal-file -` reads standard input; a file path avoids shell-specific input-redirection differences. Use `--project "My app"` instead of `--cwd` when the project already names the folder.

The follower displays new events until the mission ends or all its sessions exit. Pressing **Ctrl+C** ends your follower; it does not stop the mission. Leave it running in one terminal and use another for commands.

An external agent that starts a different mission must arrange a watch whose output and exit notifications reach it during later work and while idle. One unread background log is not a watch. If the host cannot deliver those events, the agent should say so and give you the feed command to run. On an interrupted watch, check the mission and resume from the last JSON event's `next_offset` using `--since <offset> --oldest-first`. See the generated [agent guide](../reference/cli.md#agent-guide) for the full watch contract.

## Send a message and answer a question

Use `mission show` to find the current handles, roles and pending questions. The following examples use placeholders you replace with those values; the command syntax is the same in macOS shells and PowerShell:

```sh
runner mission show <mission-id> --json
runner msg post --mission <mission-id> --to coder "Keep the change limited to the parser."
runner mission answer <mission-id> <question-id> <choice>
runner mission stop <mission-id>
runner mission archive <mission-id>
```

`mission answer` takes a question ID that `mission show` lists as pending, or the ID of the member's `ask_human` event from the feed. Any other ID, including one for a question that has already been answered, is refused with the pending question IDs, so an answer is never dropped silently.

Inside a mission, members can use `runner msg post --to reviewer "Ready for review"` and `runner msg read` without supplying a mission ID. Outside a mission, writes need `--mission`. An outside caller without `--as` speaks as the person; do not use `--as` to impersonate another member.

Runner pushes inbox notifications to every mission member. When waiting for a teammate and no independent work remains, finish your turn. Do not start a feed watcher, poll, or sleep-loop to wait for messages. Iterate with teammates across turns. External monitoring instructions do not apply to receiving messages in your own mission. Bounded diagnostic inspection, independent work and watching CI for an authorized PR remain valid.

Address several members with `runner msg post --to coder --to reviewer "Please check the parser."`, or use leading mentions: `runner msg post "@coder @reviewer Please check the parser."`. Explicit `--to` recipients take precedence. A leading address token is `@handle`, optionally followed by one `:` or `,`, so `@coder, @reviewer: Please check.` addresses both members. Handles use lowercase letters, digits, `_` and `-`, starting with a letter or digit. Any other token ends the leading run: `@reviewer's audit` and a bare `@` stay text and do not address anyone. Mentions inside the message do not address it. Unknown handles and `@human` are refused; `mission show` lists valid members. Omit recipients to broadcast when everyone needs to act, and reply to the sender for ordinary follow-ups.

When an `[inbox]` notification arrives, run `runner msg read`. It shows only new inbox messages, followed by up to ten short lines of other members' directed traffic. After a resume or whenever earlier messages are no longer in context, use `runner msg read --all` for all crew messages since joining, including sent messages and traffic between others. Unfiltered reads mark only printed inbox messages read. `--since <event-id>` and `--from <handle>` never mark messages read, even with `--all`, and omit the digest. Default reads already track unread messages; do not maintain a `--since` cursor or use `--from` as your inbox-consumption loop. When a filtered read leaves unread messages, stderr reports the unread count from that read and a hint to run unfiltered `runner msg read`, even if the filter returns no messages. Message output on stdout is unchanged. Reads also work while the daemon is unavailable. Do not post receipt-only replies such as “received” or standby messages; consuming the inbox with `runner msg read` is still required.

## Start without a saved crew

Create an empty mission and add workers individually:

```sh
runner mission start --title "Small task" --cwd /absolute/path/to/project -q
runner mission add-role <returned-mission-id> coder --task-file task.md
runner mission add-role <returned-mission-id> reviewer --task "Review the coder's diff after its handoff."
```

As an external operator, arm the feed watch for the returned ID as above. These members receive the shared goal, their own task and their own inbox notifications. Adding and removing roles is controlled by the person, not by an in-mission agent. The full [CLI reference](../reference/cli.md) lists overrides and lifecycle commands.

## Exit codes

| Code | Meaning | Next step |
| --- | --- | --- |
| `0` | Success | Read the result |
| `1` | Runner refused the request, a protocol error occurred, or a watch failed | Read stderr; a watch failure does not prove the mission stopped |
| `2` | Invalid usage or reference | Read that command's `--help` and check names or IDs |
| `3` | Runner is not running or could not be reached as a running service | Open Runner and retry |
| `5` | A sandbox blocked access | Retry the same command outside the sandbox with the host's permission mechanism |

Exit `5` does not establish whether Runner is running. A sandboxed agent should request access for the same command before diagnosing the app.
