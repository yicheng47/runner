# 610 mission 1 — Codex hook smoke on Windows

Branch: `feat/610-windows-hook-status`, base `b6988bf`. Machine: JASONPC, Windows 11 Pro 10.0.26200. Installed CLI: codex-cli 0.153.4; PowerShell 7.6.5 (`pwsh`) and Windows PowerShell 5.1. Implementation, verification and working-tree review stayed uncommitted by mission authorization; the crew did not launch Runner. Jason ran the checklist below in the dev app on JASONPC on 2026-09-16 and reported an overall pass; individual items were not separately enumerated.

## Coverage boundary

On Windows, Runner now injects Codex's status hooks with the same flags and events as on macOS: `--enable hooks --dangerously-bypass-hook-trust` and `-c hooks.<Event>=[{hooks=[{type="command",command=…,timeout=2}]}]` for SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PreCompact, PostCompact, Stop, Interrupt and SessionEnd, with `toml_edit::Value::from` quoting the command. The role opt-outs (`--disable hooks`, `-c features.hooks=false`, a role's own `hooks` override) still suppress injection.

The command differs by platform. Codex 0.153.4 runs a hook on Windows as `pwsh -NoProfile -Command <command>` with the JSON payload on a stdin pipe. The Windows `CodexStatusWatcher` starts its feed with `HookFeed::start_powershell`, which writes the reporter to `<feed>.ps1` beside the feed exactly as `HookFeed::start` writes the `.sh`, and each hook command is:

```
try{& ([ScriptBlock]::Create([IO.File]::ReadAllText('<feed>.ps1'))) '<feed>' '<Event>'}catch{[Console]::OpenStandardInput().CopyTo([IO.Stream]::Null)};'{}'
```

`ScriptBlock::Create` is not subject to execution policy, so the `.ps1` is never executed as a script file. If the file is missing (setup failed or the session ended), the command drains stdin, prints `{}` and exits 0. Paths use `/` separators and single quotes with `'` doubled. The `.ps1` is the reporter-path check, so removing it releases the hook latch; `StatusFiles::drop` and `clear_leftovers` remove `.ps1` and `.ps1.tmp` files. The first cut put the body inline in every `-c` value; review measured nine copies at about 11 K characters, past cmd.exe's 8,191-character line that an npm `codex.cmd` launch goes through.

The reporter body is shared with Copilot's plugin. It copies stdin as raw bytes through `[Console]::OpenStandardInput()` into a `MemoryStream` (the console input encoding here is gb2312, so a text read would corrupt non-ASCII prompts), writes them to `<feed>.<8 hex chars>` with `CreateNew`, and appends one UTF-8 record line under the per-feed named mutex `runner-status-<hash>`, where the hash is an 8-hex-digit FNV-1a over the UTF-16 code units of the forward-slash feed path, computed the same way by the reporter and by the Rust test that holds the lock, so the kernel object name stays short (kernel names are capped at 260 characters) for any profile or app data path: it opens the existing feed with `Open` inside the lock, seeks to the end and writes. A .NET `Append` stream keeps its own position and is not an atomic append across processes, which review showed losing 96 of 100 overlapping records. The wait is bounded at one second. Every handle shares delete, so teardown always removes the feed and an in-flight payload, and a hook never recreates a removed feed. On timeout, a missing feed or any failure the payload is removed. The body has no double quote (JSON quotes come from `[char]34`, the newline from `[char]10`), no pwsh-7-only syntax, and always exits 0.

The mapping (`CodexObservation`) and rollout correlation are untouched; Approval needed and Answer needed stay unsupported for Codex exactly as in the [347 record](347-codex-hooks-smoke.md).

## Live evidence

Headless runs through the exact argv Runner composes (`codex_status_args` for a scratch app data directory), with the empty feed and, for the final shape, the `.ps1` written as the watcher writes them, and a temporary cwd:

```
codex exec --skip-git-repo-check -s read-only <Runner-composed --enable/-c argv> "Reply with the single word ok." </dev/null
```

The final `.ps1` run (`RUNNER_CODEX_STATUS_GENERATION=gen-live-codex-3`) exited 0 in 20.1 s and printed `ok`. Codex echoed `hook: SessionStart Completed`, `hook: UserPromptSubmit Completed` and `hook: Stop Completed`, and printed its expected `--dangerously-bypass-hook-trust` warning. Feed records, sanitized:

```
{"generation":"gen-live-codex-3","hook_event_name":"SessionStart","payload_file":"probe-codex.ndjson.798367bb"}
{"generation":"gen-live-codex-3","hook_event_name":"UserPromptSubmit","payload_file":"probe-codex.ndjson.9b422c50"}
{"generation":"gen-live-codex-3","hook_event_name":"Stop","payload_file":"probe-codex.ndjson.cfcda749"}
{"generation":"gen-live-codex-3","hook_event_name":"SessionEnd","payload_file":"probe-codex.ndjson.ef1a86a4"}
```

The two earlier runs (the inline first cut, and the inline body with the mutex) recorded the same four events with exit 0. The SessionStart payload (446 bytes in the first run), with the session id and user paths replaced:

```
{"session_id":"<session>","transcript_path":"C:\\Users\\<user>\\.codex\\sessions\\2026\\09\\16\\rollout-<timestamp>-<session>.jsonl","cwd":"<tmp>\\probe\\cwd","hook_event_name":"SessionStart","model":"<model>","permission_mode":"default","source":"startup"}
```

UserPromptSubmit and Stop carried the same `turn_id`, the field the adapter keys turns on. Measured reporter runs in the unit tests: pwsh 7 about 240–560 ms per hook (the first run of a test process and 256 KB payloads at the high end), Windows PowerShell 5.1 about 150–190 ms, against the 2 s timeout. No file under `~/.codex` was edited or deleted.

## Jason's manual smoke checklist

Run in the dev app on JASONPC, in a direct chat and in a mission slot.

1. Submit a prompt: Working shows with no estimated tooltip, then Idle on completion. Stop and resume: the conversation returns and the next turn still moves Working → Idle.
2. Working holds through quiet tool work and becomes Idle on Stop. Escape or Ctrl+C reaches Idle after the rollout's `turn_aborted`. No Approval needed or Answer needed appears, as on macOS.
3. While a session runs, its feed, `.ps1` and payload files appear under `%APPDATA%\com.wycstudios.runner-dev\session-status\` and are gone after it stops. A role with its own `-c hooks.…`, `-c features.hooks=false` or `--disable hooks` shows estimated status and creates no feed.
4. In a crew, a message posted while Codex is Working or Idle delivers, and a typed draft still protects the input.
5. If an npm `codex.cmd` install is available, repeat item 1 with it.
6. The [606](../../features/archive/606-rail-glyph-liveness.md) rail check: both themes, a project with a running session and one without, a shell tab, a mission row, a provider chat, a collapsed and an expanded project, a dragged row's accent drop indicator.

## Automated verification

- `session::hook_feed::tests::powershell_reporter_keeps_raw_bytes_and_wakes_the_file_watch`, under both `pwsh` and `powershell`: a pretty-printed payload over 200 KB holding `你好，世界 ✓` returns byte-identical through `HookFeed`; the feed lives under `Jason's app data` and is opened through its forward-slash path; the append from the separate hook process sets the single-file `notify` watch's dirty flag within 900 ms, before the one-second re-read could; a feed in a missing directory exits 0 with nothing created; teardown leaves no `<feed>.*` files.
- `session::hook_feed::tests::powershell_reporter_serializes_concurrent_appends_under_the_feed_mutex`, under both shells: while the test holds the feed mutex through `CreateMutexW`, a hook writes its payload but no record, and appends after release; then 100 writers in waves of 25, each wave started and blocked on stdin before all stdins close together, produce all 100 records. Its first run caught stale seeks from a feed handle opened before the lock.
- `session::hook_feed::tests::powershell_reporter_in_flight_at_teardown_leaves_nothing_behind`, under both shells: a hook blocked on the mutex with its payload written survives `HookFeed` teardown, which removes both files; once released, the hook exits 0 without recreating the feed.
- `session::hook_feed::tests::removing_the_sh_or_ps1_reporter_reports_bridge_loss`, `startup_clears_powershell_reporters_left_by_a_crash`, `failed_powershell_setup_removes_its_reporter`.
- `session::codex_status::tests::real_powershell_reporter_large_malformed_partial_generation_quoting_and_teardown`, under both shells: the Windows counterpart of the unix real-helper test with a file name holding `'`, `'''`, `$`, a backtick, spaces and Chinese; stdout is exactly `{}`; malformed, mismatched and stale-generation records are skipped, a partial line waits for its end; removing the `.ps1` reports bridge loss; a late hook after teardown drains and leaves the directory empty.
- `router::runtime::tests::codex_injection_on_windows_calls_the_session_reporter_script`: the exact `-c` values and their TOML round trip, no `"` or `\` in any command, a command without its `.ps1` draining 256 KB and printing `{}` under both shells, and the whole `codex_status_args` line under 8,191 characters for a realistic app data path.
- `session::pty_runtime::tests::hook_status_argv_survives_a_batch_shim_launch_windows`: a resumed Bypass Codex argv (about 4.7 K characters with the hooks) and a Claude Code argv with its `--settings` (about 5.7 K characters) under a long app data path both launch through a `.cmd` shim, and the program receives every argument unchanged.
- `session::pty_runtime::tests::codex_powershell_hooks_bridge_failure_and_teardown_windows`: a ConPTY child with the Codex env, four hooks run through `pwsh`, the monitor thread delivers Working → Idle → Working → Unavailable/Interrupted, removing the `.ps1` yields `StatusBridgeFailed`, stop closes the stream and the status directory is empty.
- `inject_codex_hooks` and the manager's Codex spawn test now assert injection on Windows.
- Workspace results are in the [plan log](../../impls/archive/610-windows-hook-status.md#log).
