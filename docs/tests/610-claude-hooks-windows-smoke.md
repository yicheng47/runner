# 610 mission 1 — Claude Code hook smoke on Windows

Branch: `feat/610-windows-hook-status`, base `b6988bf`. Machine: JASONPC, Windows 11 Pro 10.0.26200. Installed CLI: Claude Code 2.1.273; Git for Windows `sh` at `C:\Program Files\Git\usr\bin\sh.exe` is on the native PATH. Implementation, verification and working-tree review stayed uncommitted by mission authorization; the crew did not launch Runner. Jason ran the checklist below in the dev app on JASONPC on 2026-09-16 and reported an overall pass; individual items were not separately enumerated.

## Coverage boundary

On Windows, Runner now injects the same Claude Code status hooks it injects on macOS: the one `--settings` JSON carries the `/clear` rekey hook plus one `sh` status command per event (SessionStart, PermissionRequest, PermissionDenied, PostToolUseFailure, Elicitation, ElicitationResult, UserPromptSubmit, PreToolUse, PostToolUse, Notification with the verified matcher, Stop, StopFailure), each with the two-second timeout. The only Windows difference is the feed path: every hook command and `RUNNER_CLAUDE_STATUS_PATH` carry it with `/` separators (`C:/Users/…/session-status/<id>.ndjson`), because the reporter strips the payload basename at `/` only. `APPEND_SCRIPT`, `hook_command`'s shape and the `.sh` helper beside the feed are unchanged. The rekey drop path keeps its backslashes.

Claude Code runs these commands under its own Git Bash. If it cannot, the command fails, no record is written, and the session stays on the estimated baseline (plan decision 6). The status mapping (`ClaudeObservation`) and transcript correlation are untouched, so every macOS semantic in the [347 record](347-status-ui-smoke.md) applies unchanged; this record proves the transport only.

## Live evidence

One headless run through the full `--settings` value Runner composes for app data under a scratch directory, `RUNNER_CLAUDE_STATUS_GENERATION=gen-live-claude`, the `.sh` helper written as `HookFeed::start` writes it, and a temporary cwd:

```
claude -p "Reply with the single word ok." --max-turns 1 --settings '<Runner-composed JSON>'
```

It exited 0 in 8.5 s, printed `ok`, and produced the rekey drop `session-keys/probe-claude.json` (the SessionStart payload) plus these feed records, sanitized:

```
{"generation":"gen-live-claude","hook_event_name":"SessionStart","payload_file":"probe-claude.ndjson.iUg199FC"}
{"generation":"gen-live-claude","hook_event_name":"UserPromptSubmit","payload_file":"probe-claude.ndjson.qvZ4CV92"}
{"generation":"gen-live-claude","hook_event_name":"Stop","payload_file":"probe-claude.ndjson.LVe50Sqq"}
```

The SessionStart payload beside the feed, with the session id and user paths replaced:

```
{"session_id":"<session>","transcript_path":"C:\\Users\\<user>\\.claude\\projects\\<encoded cwd>\\<session>.jsonl","cwd":"<tmp>\\probe\\cwd","hook_event_name":"SessionStart","source":"startup"}
```

Each `payload_file` is a bare basename with the feed's prefix, the shape `read_report` accepts. The UserPromptSubmit payload carried `prompt_id` and `prompt`; Stop carried `prompt_id`, `stop_hook_active: false` and `last_assistant_message`, the same fields the macOS adapter reads. Git `sh` startup measured about 30 ms during planning, far inside the 2 s timeout. No file under `~/.claude` was edited or deleted.

## Jason's manual smoke checklist

Run in the dev app on JASONPC, in a direct chat and in a mission slot.

1. Submit a prompt: Working shows with no estimated tooltip, then Idle on completion. Stop and resume: the conversation returns and the next turn still moves Working → Idle.
2. A command approval dialog shows Approval needed while visible and clears on the decision. AskUserQuestion shows Answer needed as it opens, an answer returns Working, Escape returns Idle without a completion dot. `/clear` still rekeys the pane.
3. While a session runs, its feed and payload files appear under `%APPDATA%\com.wycstudios.runner-dev\session-status\` and are gone after it stops. A role with its own `--settings` shows estimated status and creates no feed.
4. In a crew, a message posted during a Claude dialog is held until the dialog resolves; posted while Working or Idle it delivers; a typed draft still protects the input.
5. The [606](../features/archive/606-rail-glyph-liveness.md) rail check: both themes, a project with a running session and one without, a shell tab, a mission row, a provider chat, a collapsed and an expanded project, a dragged row's accent drop indicator.

## Automated verification

- `session::hook_feed::tests::hooks_are_gated_per_runtime`, `windows_hook_paths_use_forward_slashes` (a `C:\` path with spaces and `'` renders with `/` inside `hook_command`), `removing_the_sh_or_ps1_reporter_reports_bridge_loss`.
- `session::claude_status::tests::git_sh_runs_the_injected_commands_with_forward_slash_feeds`: the injected commands through Git's `sh` (found on PATH, else beside `git --exec-path`, else skipped with a printed reason) with a 256 KB pretty payload holding Chinese text, a feed under a directory named `Jason's status $dir`, six events mapped Busy ×3 → Idle ×3, bridge loss when the helper is removed, an empty directory after teardown, and a late hook that still drains stdin and exits 0.
- `router::runtime::tests::claude_settings_on_windows_carry_sh_status_hooks_with_forward_slash_feeds`: the exact per-event command on Windows and the untouched backslash rekey drop; `claude_status_hooks_have_short_timeouts_and_match_verified_notifications` now runs on both platforms.
- `session::pty_runtime::tests::hook_status_argv_survives_a_batch_shim_launch_windows`: a resumed Claude Code argv with the full `--settings` (about 5.7 K characters under a long app data path) launches through a `.cmd` shim with every argument intact, below cmd.exe's 8,191-character line.
- `session::manager::tests` settings-argv composition asserts the Claude env on Windows with the forward-slash path.
- Workspace results are in the [plan log](../impls/610-windows-hook-status.md#log).
