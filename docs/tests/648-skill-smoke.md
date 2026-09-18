# 648 — following and embedded skill smoke

## Coverage boundary

Mission 2 is verified without launching or restarting Runner and without starting Claude Code, Codex, pi, or TRAE sessions. Automated tests cover the CLI cursor/filter/output logic, the three socket tools, marker-gated skill installation in a temporary home, first-run bookkeeping, TRAE's catalog root, and PATH composition. They do not prove a live app socket, shell command discovery inside a real PTY, or whether each agent invokes the skill from an unprompted natural-language request; Jason's smoke below gates Mission 3.

No test writes to `~/.claude/skills`, `~/.agents/skills`, or `~/.trae/skills`. Skill filesystem tests use `tempfile` homes. `agent_skill` has no implicit home lookup, AppStore requires its skill home as an explicit constructor argument, production startup is the only real-home caller, and test startup forces that argument to `None`; the one startup-path guard injects a temporary home and proves a separate canary home stays untouched. No build is pointed at the production app's socket.

Review-time starting state on Jason's Mac is not fresh. During review at 20:54 local, the reviewer accidentally ran the case-insensitive `target/debug/runner` GUI binary instead of `target/debug/runner-agent-cli`, launching the development app for about two minutes before stopping it. No session spawned or resumed and no agent quota was spent. Startup demoted four stale development session rows to stopped, repaired the three existing marked `runner-dev` folders with the real development sidecar, recorded `.agents/skills` and `.claude/skills` in development `initializedSkillRoots` (TRAE was not eligible), reinstalled the development sidecar, and left a stale development `mcp.sock`. The reviewer touched none of that state afterward.

## Automated proof

- `mission feed --follow` starts from the returned cursor, survives empty polls, emits one or several arriving events once, deduplicates an overlapping window, checks archive state every four polls, exits on archive, and propagates the not-running exit code 3.
- `--types`, `--from`, the default noise filter, and `--all` are exercised. Follow JSON is sorted event NDJSON with no cursor line and flushes after every line; one-shot unfiltered JSON preserves the tool response bytes.
- `runner help agents` is compiled into the binary, and its declared top-level command set is compared with Clap's command tree.
- Skill install, refresh, foreign-folder preservation, debug naming and narrowed description, quoted sidecar paths, and marker-only removal use temporary directories. The debug render always names its absolute development sidecar, including for `help agents`, while the release render stays PATH-first. A marked `runner-dev` skill with a dead sidecar path is rewritten with the current development sidecar. First-run root bookkeeping proves one attempt per eligible root, a skipped unavailable root, a later install, and no reinstall after removal.
- TRAE catalogs only `~/.trae/skills`; its other documented compatibility roots are deliberately excluded.
- Fresh direct chats, runtime terminals, direct resumes, and direct forks carry `<app data>/bin`; mission sessions retain their identity shim ahead of the same folder. `compose_path` continues to join with the platform separator.
- `session_get` exposes Working and Idle manager status plus raw activity, `session_stop` preserves a resumable row, and `session_archive` archives a direct chat and refuses a mission session. A running terminal is refused before any stop call and remains Running and unarchived. The shared registry parity test includes all three tools.

## Jason's `make run` checklist

- [ ] Start `make run`. From the current review-time state, confirm the three marked `runner-dev` folders are refreshed from the pre-MF1 body to say `Always invoke` with the real development sidecar path. `.trae/skills` is not recorded as initialized because TRAE was detected but not enabled in the development settings, so that root was not eligible. A fresh observation of first-run creation requires Jason to choose to remove the folders and reset `initializedSkillRoots` first; no agent should reset either on its own.
- [ ] Delete one `runner-dev` folder, restart the development app, and confirm it stays deleted. Confirm a same-name folder without `.runner-managed` is reported but unchanged.
- [ ] Run the development sidecar's `runner status`; confirm all three skill roots show `managed`, `missing`, or `foreign` correctly.
- [ ] In a Runner agent chat and a Runner terminal, run `which runner` (or `Get-Command runner` on Windows) and confirm the development app's sidecar wins. A Runner terminal is a login shell, and macOS `/etc/zprofile` `path_helper` may reorder PATH even though Runner supplied the sidecar folder first, so command resolution is the proof.
- [ ] Run `runner mission feed <id> --follow`, append events, and confirm each appears once. Archive the mission and confirm exit 0. Repeat with `--follow --json | jq -c .` and confirm clean NDJSON with no cursor records.
- [ ] Read `runner help agents` end to end and confirm every copied flow works.
- [ ] Run `runner session show <id>` on one Working and one Idle chat; confirm lifecycle, semantic activity, source, outcome/detail/waits, and raw activity. Run `session stop`, resume from the app, then `session archive` on a direct chat. Confirm archive refuses a mission slot.
- [ ] Open Settings → Skills → TRAE CLI and confirm it names and lists only `~/.trae/skills`, with no Runner toggle.
- [ ] For the development-build agent-flow check, start only the runtime Jason chooses and ask: “using the Runner dev build, start a mission with the peer coding crew to say hello and follow it”. Confirm it loads `runner-dev`, invokes the absolute development sidecar for every command, reads that sidecar's `help agents`, and follows the mission. This proves the development executable path and flow, not the release skill trigger.

## Results against `make run`, 2026-09-18

Driven from a plain terminal through the dev sidecar, every script guarded on `runner status` reporting the `com.wycstudios.runner-dev` socket.

- The dev app started at 21:10 and refreshed the three marked `runner-dev` folders twenty seconds later to the absolute-sidecar body; `runner status` lists all three roots as `managed`.
- `mission feed --follow`: four followers on a throwaway shell-crew mission while six messages and an `ask_human` were posted. The NDJSON stream parsed line for line, carried each event exactly once with no duplicate ids, no `session_status` or `inbox_read`, and no `next_offset` line; `--types message --from worker2` returned only that slot's three messages; SIGINT exited 0; all followers exited 0 two seconds after the mission was archived.
- `session show` returns the live agent status with its source (`working`, `baseline` for a fresh pi chat); `session list` has the ACTIVITY column; `session stop` left a resumable row and `session resume` brought it back; `session archive` on a running chat stopped it first and removed it from the list; archive refused a terminal-class shell session without stopping it, and refused a mission session by full id.
- `which runner` inside a session the dev app spawned prints the dev sidecar (Jason). The process environment of zsh and pi cannot be read from outside on macOS, so this one is a human check.
- Not exercised: deleting a managed folder and restarting, an unmarked same-name folder, the TRAE row in the Skills pane, the dev agent-flow prompt, and exit 3 when the app quits mid-follow (unit-tested).

## Mission 3 four-runtime gate

Run this gate against a nightly or release build, not `make run`, because the release `runner` skill is what must fire for an ordinary Runner request; `runner-dev` intentionally triggers only when the prompt names dev, a development build, `make run`, or `runner-dev`. Switch the Runner entry off in Settings → MCP. In a fresh plain-terminal session for each runtime, ask exactly: “start a mission with the peer coding crew to say hello and follow it”. Do not supply CLI hints. Record whether the release skill fired without prompting, whether the agent read `runner help agents`, and whether it started and followed the mission. Tune the release skill description if any runtime misses, then repeat that runtime.

| Runtime | Skill fired unprompted | Read guide | Started mission | Followed feed | Notes |
| --- | --- | --- | --- | --- | --- |
| Claude Code | [ ] | [ ] | [ ] | [ ] | |
| Codex | [ ] | [ ] | [ ] | [ ] | |
| pi | [ ] | [ ] | [ ] | [ ] | |
| TRAE CLI | [ ] | [ ] | [ ] | [ ] | |

Mission 3 does not start until all four rows pass.
