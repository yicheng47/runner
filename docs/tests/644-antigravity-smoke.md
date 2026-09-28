# 644 — Antigravity CLI smoke checklist

Follow-up [#747 validation](747-antigravity-followups.md) records the 2026-09-28 CLI probes and automated results. Native Runner UI and JASONPC checks in this checklist remain pending where the follow-up marks them so.

Branch: `feat/644-antigravity-runtime`. Spec: [644](../features/644-antigravity-runtime.md). Phases 1–3 are implemented and unit-tested. Initial live macOS checks against `agy` 1.2.12 in runner-dev are recorded below; the remaining phase 4 and hook checks stay on the checklist.

## Coverage boundary

Every spawn passes `--log-file <app data>/antigravity/logs/<Runner session id>.log`, cleared before the spawn, and `session/agy_capture.rs` tails it for the first `Created conversation <uuid>` line, which it writes into `agent_session_key`. Its unit fixture copies the glog shape from agy 1.2.8 logs on this Mac (`I0922 16:29:57.817770     580 server.go:1224] Created conversation <uuid>`); the runner-dev smoke below confirms agy 1.2.12 still prints it. A resume passes `--conversation <key>` when `~/.gemini/antigravity-cli/conversations/<key>.db` exists, and otherwise starts fresh with the cold-start first turn.

The capture thread exits after that first key. Runner therefore does not follow an in-process conversation change through agy's `/clear` (`/new`), `/resume`, or `/fork`; the stored key can remain on the previous conversation. This is a pending capability in the README matrix, separate from capturing a fresh key when a process resume falls back to a new conversation.

Before every spawn, `session/agy_trust.rs` adds the canonical cwd to `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`, rewriting only that member. Like the Copilot seed, it skips the home directory and the filesystem root, so a chat in `~` still shows agy's trust dialog. Model and effort come only from the static catalog pairs; Accept edits is `--mode accept-edits`, Bypass `--dangerously-skip-permissions`.

On macOS every spawn also passes `--add-dir <app data>/antigravity-hooks`. Its `.agents/hooks.json` registers `PreInvocation`, `PostToolUse` (matcher `*`), `PostInvocation` and `Stop`, and no `PreToolUse`; the reporter prints `{}` for every event. Working comes from the first three, Idle from `Stop` with `fullyIdle: true`, and Response failed from `Stop` with a non-empty `error`. The failure shape is the documented one (`terminationReason: "error"`, `error: "<text>"`); no live failure has been recorded. A `Stop` with `fullyIdle: false` changes nothing, so a turn that ends with background work still running stays Working until a later event.

The sidebar uses Antigravity's full-color provider mark from its official press assets.

## Runner-dev macOS smoke, 2026-09-28

Runner-dev was running this feature worktree's `target/debug/Runner`; the installed `agy --version` was 1.2.12. The following checks used a separate `/tmp/runner-644-smoke.Aw0ZbV` folder and temporary role, leaving Jason's existing Antigravity chat untouched.

- **Pass — blank chat and trust seed.** A runtime-only chat launched with a per-session `--log-file` and `--add-dir <dev app data>/antigravity-hooks`. After it became idle, `agent_session_key` remained null. Runner added only the canonical `/private/tmp/runner-644-smoke.Aw0ZbV` path to `trustedWorkspaces`; all other settings members and prior trusted paths were preserved.
- **Pass — first turn, key, model and hook status.** A role chat launched with `--model gemini-3.8-flash --effort high -i <first turn>`. Its log printed `Created conversation <uuid>`, the session row held the same key, and `conversations/<uuid>.db` existed. The row moved from Working to `ready`/`completed` with `source: hook`; the feed recorded `PreInvocation`, `PostInvocation` and `Stop`. The terminal text itself was not inspected.
- **Pass — stop/resume and missing conversation.** Stopping and resuming the role chat launched `--conversation <same key>` without `-i`; its conversation database still had two steps, so the first turn was not replayed. After stopping and moving that test database aside, resume launched with `-i` and no `--conversation`, captured a new key, and completed. The old database was restored afterward.
- **Boundary — direct chat permissions.** A role set to Accept edits still launched an attended direct chat without `--mode accept-edits`, as Runner's direct-chat permission contract requires. No file had appeared before that test session was stopped, so mission Accept edits remains untested.

The CLI cannot type into or scroll the dev app's terminal, and this environment blocks native UI scripting. Relaunch, visual rendering, wheel input, Settings UI, permission prompts, and deleting a chat remain unverified here.

**Update and usage audit.** Installed agy 1.2.12 exposes `agy update`, and its [documentation](https://antigravity.google/docs/cli/troubleshooting/) describes a background self-updater. Runner sets Antigravity's `update_args` to empty, so Settings → Agents has no manual Update button. The #644 release fetched only Claude Code and Codex quotas; [#747 validation](747-antigravity-followups.md) records the later read-only `agy -p /usage --output-format json` integration.

## Jason's macOS checklist

1. **Direct chat, never-trusted folder.** Make a new folder, open an Antigravity CLI chat in it with a role that has a persona. Confirm no trust dialog appears, `settings.json` gained exactly that path with every other key unchanged, the persona arrives once as the first turn, the pane paints on the alternate screen, and the tab keeps Runner's name (agy sets no title).
2. **Key capture.** Within a few seconds of the first message, confirm the row has a key (`runner` CLI or the DB) equal to the id in `<app data>/antigravity/logs/<session>.log` and in `~/.gemini/antigravity-cli/conversations/`.
3. **Blank chat.** Start a runtime-only Antigravity chat and type nothing. Confirm there is no key; send a message and confirm the key appears.
4. **Relaunch.** Quit and relaunch Runner. Confirm the chat resumes the same conversation (`--conversation <key>`, history visible) and no first turn is replayed.
5. **Missing conversation.** Stop the chat, move `conversations/<key>.db` aside, and resume. Confirm a fresh start with the first turn and a new key on the row. Put the file back afterwards if you want the old conversation.
6. **Model and effort.** Pick `gemini-3.1-pro` + `high`: the footer reads Gemini 3.1 Pro · high. Confirm the effort picker offers no medium for Pro, only Default with model on Default, and nothing for the Claude and GPT-OSS models.
7. **Mission permissions.** Start an agy mission slot and confirm its argv contains `--dangerously-skip-permissions`, including when a stored role carries an old Default or Accept edits permission flag. Confirm file edits and a harmless shell command run without prompting. Direct chats carry no Runner permission override and follow agy's own settings. The global and role permission selectors are no longer shown.
8. **Crew.** Start a mission with an agy slot whose stored role carries an old Default or Accept edits permission flag in a never-trusted folder. Confirm the mission still launches with Bypass, runs unattended, delivers the launch prompt once, and `runner msg read`, `runner msg post` and `runner signal ask_human` work from `run_command` without a path prompt (`--add-dir <mission dir>`).
9. **Hook status.** In a direct chat using agy's native permission settings, a running turn shows Working without the estimated tooltip, a finished turn Idle/Completed, and a tool call keeps it Working. Repeat the tool call in a fixed-Bypass mission slot; tools still run with Runner's hooks loaded, showing Runner sends no `PreToolUse` decision.
10. **Hooks and Orca.** Repeat check 9 with the existing Orca global `~/.gemini/config/hooks.json` intact. If agy supports process-scoped hook configuration, use that in a disposable session to test a `PreToolUse` `"ask"`; do not move aside the global file while other sessions may use it. Record whether it overrides `--dangerously-skip-permissions` (spec open item). On 2026-09-23 the global file on this Mac had no `PreToolUse` entry.
11. **Distraction.** Ask the model what workspaces it has. Record whether `antigravity-hooks` distracts it the way the probe's `rh` folder did (spec decision 5).
12. **Response failed.** Force a failed turn (a disposable model or network failure) and capture the `Stop` payload from the status feed before it is removed: `<app data>/session-status/<session>.ndjson` plus its payload file. Confirm Runner shows Response failed, and record the real shape here.
13. **Escape and Ctrl+C.** Interrupt a running turn with each key. If no completed or failed `Stop` was already drained, the session should remain Interrupted even when late `PostToolUse` or `PostInvocation` hooks arrive; a new `PreInvocation` starts Working again. Record any `Stop` payload. A completed or failed `Stop` drained before cancellation takes precedence over Interrupted.
14. **Subagent.** Trigger `invoke_subagent` and confirm the subagent's own `Stop` does not mark the parent Idle while it is still working.
15. **Fixture and wheel.** The #747 first-turn fixture and render test are recorded. Its idle reply is on the primary screen without mouse reporting, where Runner's wheel encoder emits no input and local viewport scrolling applies. Check an actual wheel over an idle agy pane in the native Runner UI; any different terminal mode or input behavior needs a recorded repro before a runtime policy changes.
16. **Settings.** Settings → Agents shows the Antigravity CLI row with the bare `agy --version` and no Update button. Settings → Skills lists `~/.gemini/antigravity-cli/skills` and `~/.gemini/skills` with no toggle. Settings → MCP lists agy's servers from the 0-byte `~/.gemini/config/mcp_config.json`, copying a stdio server writes `{"args":[…],"command":…,"disabled":false}`, and an HTTP server's copy is refused with a pointer to `agy mcp add`.
17. **Cleanup.** Archive and delete the chat; confirm its log is gone from `<app data>/antigravity/logs/`. Confirm nothing under `~/.gemini` changed except `trustedWorkspaces` and any MCP entry you copied.

## JASONPC checklist

1. Confirm where agy installs (the docs say `%LOCALAPPDATA%\Antigravity\`), that Runner detects and enables it by default, and that switching it off in Settings → Agents persists after relaunch.
2. Run checks 1–5, 7 and 8 above under ConPTY. Hook status is macOS-only, so status stays on the terminal-activity baseline.
3. Confirm the trust seed writes the path agy compares. Rust's `canonicalize` returns a `\\?\C:\…` verbatim path on Windows, the same shape the Copilot and Codex seeds write, and agy may not match it.

## Automated verification

Recorded in the pull request with each gate's exit code: `runner-backend`, `runner-app` and `runner-terminal` tests at `--profile ci`, workspace Clippy with `-D warnings`, `cargo fmt --all --check` and `git diff --check`.
