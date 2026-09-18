# 610 mission 1 — GitHub Copilot CLI hook smoke on Windows

Branch: `feat/610-windows-hook-status`, base `b6988bf`. Machine: JASONPC, Windows 11 Pro 10.0.26200. Installed CLI: GitHub Copilot CLI 1.0.85 (winget); PowerShell 7.6.5. Implementation, verification and working-tree review stayed uncommitted by mission authorization; the crew did not launch Runner. Jason ran the checklist below in the dev app on JASONPC on 2026-09-16 and reported an overall pass; individual items were not separately enumerated.

## Coverage boundary

On Windows, Runner now installs `<app data>/copilot-hooks/` at startup and adds `--plugin-dir <app data>/copilot-hooks` to every Copilot invocation, as on macOS, and sets `RUNNER_COPILOT_STATUS_PATH` (with `/` separators) and `RUNNER_COPILOT_STATUS_GENERATION` in the spawn environment. Every event entry in `hooks/hooks.json` keeps its `command` (the `sh` reporter, unchanged, which macOS keeps running) and gains a `powershell` key beside it with the same matcher on Notification and the same `timeout: 2`. Copilot picks the slot by OS, so the plugin file is identical on both platforms. The `powershell` body is the inline reporter shared with Codex (see the [Codex record](610-codex-hooks-windows-smoke.md#coverage-boundary)), reading the feed from `$env:RUNNER_COPILOT_STATUS_PATH` and the generation from `$env:RUNNER_COPILOT_STATUS_GENERATION`; with the path env empty it drains stdin and exits 0, like `report.sh`, and parallel hooks serialize their records under the same per-feed mutex. `report.sh` is still written on Windows, so `plugin_available` and `start_external`'s reporter check keep working, and removing it still reports bridge loss.

The mapping (`CopilotObservation`) and transcript correlation are untouched. Copilot's Windows transcript is `%USERPROFILE%\.copilot\session-state\<session>\events.jsonl`, the path `copilot_home` already yields. `ErrorOccurred` stays unmapped and the folder-trust seed is not changed by this mission.

## Live evidence

One headless run with the plugin exactly as `install_plugin` writes it, mounted through the argv `copilot_status_args` returns, `RUNNER_COPILOT_STATUS_GENERATION=gen-live-copilot`, the empty feed created as the watcher creates it, from a temporary cwd:

```
copilot -p "Reply with the single word ok." -s --allow-all-tools --plugin-dir "<tmp>\probe\app data\copilot-hooks" </dev/null
```

It exited 0 in 10.8 s with `ok` and an empty stderr. No folder-trust dialog can appear in `-p` mode, so this run neither shows nor rules out the 1.0.85 trust-seed change the plan flags; item 1 below is where it would surface. Feed records, sanitized:

```
{"generation":"gen-live-copilot","hook_event_name":"UserPromptSubmit","payload_file":"probe-copilot.ndjson.4cead1b2"}
{"generation":"gen-live-copilot","hook_event_name":"SessionStart","payload_file":"probe-copilot.ndjson.9bd50b46"}
{"generation":"gen-live-copilot","hook_event_name":"Stop","payload_file":"probe-copilot.ndjson.c728a192"}
{"generation":"gen-live-copilot","hook_event_name":"SessionEnd","payload_file":"probe-copilot.ndjson.b6e798e7"}
```

After the per-feed mutex fix the run was repeated and recorded the same four events with exit 0. Every payload name is eight hex characters, the PowerShell reporter's form (`mktemp` would give mixed-case letters), so Copilot ran the `powershell` slot. The SessionStart payload (342 bytes), with the session id and user paths replaced:

```
{"hook_event_name":"SessionStart","session_id":"<session>","timestamp":"2026-09-16T15:04:16.124Z","cwd":"<tmp>\\probe\\cwd","source":"new","initial_prompt":"Reply with the single word ok."}
```

As on 1.0.83, UserPromptSubmit arrived before SessionStart. Stop carried `stop_reason: "end_turn"` and `transcript_path` under `%USERPROFILE%\.copilot\session-state\<session>\events.jsonl`. No file under `~/.copilot` was edited or deleted.

## Jason's manual smoke checklist

Run in the dev app on JASONPC, in a direct chat and in a mission slot.

1. Submit a prompt: Working shows with no estimated tooltip, then Idle on completion. Stop and resume: the conversation returns and the next turn still moves Working → Idle. Note whether the first spawn shows a folder-trust dialog (a 540 follow-up if it does).
2. `echo` auto-approves with no Approval needed. An Edit in Default mode shows Approval needed while its dialog is open and clears on Enter. `ask_user` shows Answer needed as it opens, and Escape ends Idle/Interrupted without a completion dot.
3. A disposable repository with `.github/copilot/settings.local.json` holding `{"disableAllHooks":true}` stays estimated. A role's own `--plugin-dir` composes with Runner's.
4. While a session runs, its feed and payload files appear under `%APPDATA%\com.wycstudios.runner-dev\session-status\` and are gone after it stops.
5. In a crew, a message posted during a Copilot dialog is held until the dialog resolves; posted while Working or Idle it delivers; a typed draft still protects the input.
6. The [606](../../features/archive/606-rail-glyph-liveness.md) rail check: both themes, a project with a running session and one without, a shell tab, a mission row, a provider chat, a collapsed and an expanded project, a dragged row's accent drop indicator.

## Automated verification

- `session::copilot_status::tests::plugin_generation_has_the_exact_event_catalog_and_reporter_commands`: every entry keeps its exact `command` and gains a `powershell` value equal to the shared body over `$env:RUNNER_COPILOT_STATUS_PATH`, ending in `;exit 0`, naming its event, with no `"`.
- `session::copilot_status::tests::powershell_entry_drains_missing_path_handles_spaces_large_payload_and_teardown`, under both `pwsh` and `powershell`, running the `powershell` value read back from `hooks.json`: an empty path env drains a 256 KB payload and exits 0 silently; a pretty 256 KB payload with Chinese text under `Jason's runner app data` / `session with spaces` produces Working; removing `report.sh` reports bridge loss; teardown leaves the directory empty and a late hook writes nothing.
- `session::pty_runtime::tests::copilot_powershell_plugin_hooks_bridge_failure_and_teardown_windows`: a ConPTY child with the Copilot env, three plugin hooks run through `pwsh`, the monitor thread delivers Working → Idle → Working, removing `report.sh` yields `StatusBridgeFailed`, stop closes the stream and the status directory is empty.
- `router::runtime::tests::copilot_status_plugin_args_require_a_complete_installed_plugin` and the manager's Copilot direct-spawn test now assert `--plugin-dir` and the env on Windows.
- Workspace results are in the [plan log](../../impls/archive/610-windows-hook-status.md#log).
