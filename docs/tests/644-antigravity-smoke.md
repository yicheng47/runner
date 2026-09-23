# 644 — Antigravity CLI smoke checklist

Branch: `feat/644-antigravity-runtime`. Spec: [644](../features/644-antigravity-runtime.md). Phases 1–3 are implemented and unit-tested; nothing in this branch ran against a live `agy`, because a session signs in with Jason's Google account and writes under `~/.gemini`. This file is spec phase 4 plus the phase 3 hook checks, for Jason to run by hand. Installed CLI at the time of writing: `agy --version` printed `1.2.8`.

## Coverage boundary

Every spawn passes `--log-file <app data>/antigravity/logs/<Runner session id>.log`, cleared before the spawn, and `session/agy_capture.rs` tails it for the first `Created conversation <uuid>` line, which it writes into `agent_session_key`. Its unit fixture copies the glog shape from agy 1.2.8 logs on this Mac (`I0922 16:29:57.817770     580 server.go:1224] Created conversation <uuid>`); a live session is the only proof that 1.2.9 and later still print it. A resume passes `--conversation <key>` when `~/.gemini/antigravity-cli/conversations/<key>.db` exists, and otherwise starts fresh with the cold-start first turn.

Before every spawn, `session/agy_trust.rs` adds the canonical cwd to `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`, rewriting only that member. Like the Copilot seed, it skips the home directory and the filesystem root, so a chat in `~` still shows agy's trust dialog. Model and effort come only from the static catalog pairs; Accept edits is `--mode accept-edits`, Bypass `--dangerously-skip-permissions`.

On macOS every spawn also passes `--add-dir <app data>/antigravity-hooks`. Its `.agents/hooks.json` registers `PreInvocation`, `PostToolUse` (matcher `*`), `PostInvocation` and `Stop`, and no `PreToolUse`; the reporter prints `{}` for every event. Working comes from the first three, Idle from `Stop` with `fullyIdle: true`, and Response failed from `Stop` with a non-empty `error`. The failure shape is the documented one (`terminationReason: "error"`, `error: "<text>"`); no live failure has been recorded. A `Stop` with `fullyIdle: false` changes nothing, so a turn that ends with background work still running stays Working until a later event.

The provider mark is a placeholder, the generic chat bubble, until Jason designs it.

## Jason's macOS checklist

1. **Direct chat, never-trusted folder.** Make a new folder, open an Antigravity CLI chat in it with a role that has a persona. Confirm no trust dialog appears, `settings.json` gained exactly that path with every other key unchanged, the persona arrives once as the first turn, the pane paints on the alternate screen, and the tab keeps Runner's name (agy sets no title).
2. **Key capture.** Within a few seconds of the first message, confirm the row has a key (`runner` CLI or the DB) equal to the id in `<app data>/antigravity/logs/<session>.log` and in `~/.gemini/antigravity-cli/conversations/`.
3. **Blank chat.** Start a runtime-only Antigravity chat and type nothing. Confirm there is no key; send a message and confirm the key appears.
4. **Relaunch.** Quit and relaunch Runner. Confirm the chat resumes the same conversation (`--conversation <key>`, history visible) and no first turn is replayed.
5. **Missing conversation.** Stop the chat, move `conversations/<key>.db` aside, and resume. Confirm a fresh start with the first turn and a new key on the row. Put the file back afterwards if you want the old conversation.
6. **Model and effort.** Pick `gemini-3.1-pro` + `high`: the footer reads Gemini 3.1 Pro · high. Confirm the effort picker offers no medium for Pro, only Default with model on Default, and nothing for the Claude and GPT-OSS models.
7. **Permission modes.** A role in Accept edits edits files without asking; Bypass runs a shell command without asking; Default follows `toolPermission`. The dropdown offers no Auto.
8. **Crew.** Start a mission with an agy slot in Bypass mission mode in a never-trusted folder. Confirm the launch prompt lands once, `runner msg read`, `runner msg post` and `runner signal ask_human` work from `run_command` without a path prompt (`--add-dir <mission dir>`), and the slot runs unattended.
9. **Hook status.** In a chat: a running turn shows Working without the estimated tooltip, a finished turn Idle/Completed, a tool call keeps it Working. Tools still run under `request-review`, Accept edits and Bypass with Runner's hooks loaded, which proves Runner sends no `PreToolUse` decision.
10. **Hooks and Orca.** Repeat check 9 and the Bypass crew with Orca's global `~/.gemini/config/hooks.json` present and again with it moved aside. Record whether a `PreToolUse` `"ask"` from Orca overrides `--dangerously-skip-permissions` (spec open item). On 2026-09-23 the global file on this Mac had no `PreToolUse` entry.
11. **Distraction.** Ask the model what workspaces it has. Record whether `antigravity-hooks` distracts it the way the probe's `rh` folder did (spec decision 5).
12. **Response failed.** Force a failed turn (a disposable model or network failure) and capture the `Stop` payload from the status feed before it is removed: `<app data>/session-status/<session>.ndjson` plus its payload file. Confirm Runner shows Response failed, and record the real shape here.
13. **Escape.** Interrupt a running turn with Esc. Record what `Stop` carries; if its `error` is non-empty the turn will read as Response failed rather than interrupted.
14. **Subagent.** Trigger `invoke_subagent` and confirm the subagent's own `Stop` does not mark the parent Idle while it is still working.
15. **Fixture and wheel.** Record `crates/runner-terminal/fixtures/agy-first-turn.ndjson` from a live first turn and add its render test (spec phase 2). Scroll the wheel over an idle agy pane: every tick reaches agy as Up or Down. Record whether that scrolls the transcript or walks the prompt history; if it rewrites the input line, agy needs a wheel policy.
16. **Settings.** Settings → Agents shows the Antigravity CLI row with the bare `agy --version` and no Update button. Settings → Skills lists `~/.gemini/antigravity-cli/skills` and `~/.gemini/skills` with no toggle. Settings → MCP lists agy's servers from the 0-byte `~/.gemini/config/mcp_config.json`, copying a stdio server writes `{"args":[…],"command":…,"disabled":false}`, and an HTTP server's copy is refused with a pointer to `agy mcp add`.
17. **Cleanup.** Archive and delete the chat; confirm its log is gone from `<app data>/antigravity/logs/`. Confirm nothing under `~/.gemini` changed except `trustedWorkspaces` and any MCP entry you copied.

## JASONPC checklist

Antigravity CLI is off by default on Windows; enable it in Settings → Agents first.

1. Confirm where agy installs (the docs say `%LOCALAPPDATA%\Antigravity\`) and that Runner detects it.
2. Run checks 1–5, 7 and 8 above under ConPTY. Hook status is macOS-only, so status stays on the terminal-activity baseline.
3. Confirm the trust seed writes the path agy compares. Rust's `canonicalize` returns a `\\?\C:\…` verbatim path on Windows, the same shape the Copilot and Codex seeds write, and agy may not match it.
4. If it all passes, turn `default_enabled` on for Windows in `ops/runtime.rs`, the README footnote ³ and `docs/arch/windows.md`.

## Automated verification

Recorded in the pull request with each gate's exit code: `runner-backend`, `runner-app` and `runner-terminal` tests at `--profile ci`, workspace Clippy with `-D warnings`, `cargo fmt --all --check` and `git diff --check`.
