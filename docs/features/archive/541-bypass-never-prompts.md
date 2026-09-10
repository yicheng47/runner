# 541 — Bypass mode never waits on a first-run dialog

> Tracking issue: [#541](https://github.com/yicheng47/runner/issues/541) (bug)
> Priority: P1.
> Status: **shipped 2026-09-10 in [#544](https://github.com/yicheng47/runner/pull/544)**; archived 2026-09-10.

## Motivation

Since [527](./527-mission-permission-mode.md) shipped on 2026-09-09, every mission slot spawns with its runtime's bypass flags by default. For claude-code that is `--permission-mode bypassPermissions`, and Claude Code 2.1.267 answers it on first use with a consent dialog ("WARNING: Claude Code running in Bypass Permissions mode … By proceeding, you accept all responsibility") before the TUI takes its first turn. Runner does nothing about it: no preseed, no detection, no feed signal. The slot sits on the dialog, the byte-flow idle detector reports it idle, and the mission stalls silently — the exact failure 527 set out to remove.

Observed on 2026-09-10 in the first `538 runtime enum` mission (`01M24HHTKXC17P5VF94V8N59T4`, codex peer crew): the codex coder posted its first message 24 s after start, the claude-code reviewer flapped busy/idle for ~80 s with no output, and the human found the dialog in the pane and archived the mission. Accepting it wrote `skipDangerousModePermissionPrompt: true` into `~/.claude/settings.json`, so it will not recur on this machine, but every colleague's first Bypass mission hits the same wall.

How Claude Code decides, read from the 2.1.267 binary:

- The dialog is skipped when `skipDangerousModePermissionPrompt` is true in any settings layer — `userSettings` (`~/.claude/settings.json`), `localSettings`, `flagSettings`, or `policySettings` — or when the legacy `bypassPermissionsModeAccepted` flag is set in `~/.claude.json`. Accepting the dialog sets the user-settings key.
- `flagSettings` is the `--settings <file-or-json>` command-line layer, which Runner already uses: `claude_settings_args` in `crates/runner-backend/src/router/runtime.rs` passes `--settings {"tui":"fullscreen","hooks":{…}}` on every claude-code spawn whose runner args do not carry their own `--settings`.
- An enterprise `policySettings.permissions.disableBypassPermissionsMode: "disable"` forbids bypass outright; Claude Code then reports "Bypass permissions mode was disabled by settings" and runs without it. Runner cannot and should not override that.

codex needs nothing: the mission Bypass pair (`--ask-for-approval never --sandbox danger-full-access`) started the coder without a prompt in the same mission, and the workspace trust dialog is already preseeded by [0045](../../impls/archive/0045-codex-trust-preseed.md). TRAE's `--permission-mode bypass_permissions` is unverified (not installed on the author's machine).

## Scope

### In scope

- **Preseed through the argv Runner already owns.** When the effective permission mode of a claude-code spawn is Bypass, `claude_settings_args` adds `"skipDangerousModePermissionPrompt": true` to the `--settings` JSON it composes. It receives the runner args after `apply_mission_permission_mode` and `apply_permission_mode` have written the canonical flags, so `infer_permission_mode(Some(Runtime::ClaudeCode), runner_args) == PermissionMode::Bypass` decides it with no new parameter. Mission slots under the app-wide Bypass setting and direct chats whose runner row carries Bypass are both covered; Auto, AcceptEdits, and Default spawns are unchanged.
- **Scoped to the spawn, not the user's config.** Runner writes nothing to `~/.claude/settings.json` or `~/.claude.json`. The flag lives in one process's argv, so a user who runs `claude --permission-mode bypassPermissions` in their own terminal still gets Claude Code's dialog.
- **The consent is Runner's Bypass setting.** Settings → Missions already says "Bypass never prompts — nobody is watching a mission slot to answer." The runner-form description for claude-code Bypass in `crates/runner-app/src/surfaces/runners.rs` (`permission_mode_description`) currently promises a one-time consent dialog; it changes to say Runner accepts Claude Code's bypass disclaimer for the sessions it spawns.
- **The own-`--settings` rule stays.** A runner whose args pass `--settings` themselves gets nothing from Runner today (no fullscreen pair either); that stays, and such a runner keeps the dialog. The runner form's Bypass description names this.
- **Trust dialog check.** Claude Code also asks "Do you trust the files in this folder?" on the first launch in a cwd it has not seen (`hasTrustDialogAccepted` for the cwd or an ancestor up to the git root in `~/.claude.json`). Whether that dialog appears under `bypassPermissions` is unverified. It matters twice: the same trust check gates every configured hook, and until it is accepted Claude Code silently skips them ("Skipping SessionStart hook execution - workspace trust not accepted"), which includes Runner's `/clear` rekey hook from 459. Hooks supplied through `--settings` need no consent of their own. Phase 3 checks the dialog in a never-opened cwd; if it appears, it is filed as its own bug with a preseed shaped like 0045's rather than folded in here.

### Out of scope

- Detecting a permission or consent dialog from PTY bytes and surfacing it in the feed. With the preseed the bypass dialog cannot appear on a Runner-spawned session except in the own-`--settings` case; 527's optional phase 3 (`permission_prompt` hook signal) remains the route for prompts in Auto mode.
- Prompts in Auto or Default mode. Those modes ask by design; 527 documents why missions default to Bypass.
- Restarting a slot that is already parked on a dialog. That is [#542](https://github.com/yicheng47/runner/issues/542).
- TRAE's bypass behaviour, until a TRAE install is available to probe.

## Implementation Phases

### Phase 1 — adapter

- `claude_settings_args` in `crates/runner-backend/src/router/runtime.rs`: compute `infer_permission_mode(runtime, runner_args)` and insert `skipDangerousModePermissionPrompt: true` into the settings object when it is `Bypass`. Keep the early return for runners that carry `--settings`.
- Tests beside the existing `claude_settings_args` tests: Bypass args → key present alongside `tui` and the `SessionStart` hook; Auto / AcceptEdits / Default args → key absent; legacy `--dangerously-skip-permissions` rows → present (it infers as Bypass); runner with its own `--settings` → nothing, unchanged; mission path: `apply_mission_permission_mode(Bypass)` then `trailing_runtime_args` → the `--settings` value contains the key.

### Phase 2 — copy

- `permission_mode_description(claude-code, Bypass)` in `runners.rs`: "Skip every check. Runner accepts Claude Code's bypass disclaimer for the sessions it spawns; a runner that passes its own `--settings` still sees it."

### Phase 3 — smoke

- Remove `skipDangerousModePermissionPrompt` from `~/.claude/settings.json` on a test account (or use a fresh account), leave Settings → Missions at Bypass, start a mission with a claude-code slot: no dialog, the launch prompt lands, the feed shows the slot busy within seconds.
- Direct chat from a runner set to Auto: the dialog behaviour is unchanged (none in Auto), and the runner's other `--settings` content is intact.
- Direct chat from a Bypass runner in a cwd Claude Code has never opened: record whether the trust dialog appears; file it if it does.
- Windows: repeat the mission smoke on JASONPC.

### Phase 4 — docs

- `docs/arch/arch.md` §3.2's runtime-argv sentence names the skip key beside the fullscreen pair.
- 527's archived spec gets a one-line pointer to this fix under its Runtime mappings.

## Verification

- [ ] A Bypass claude-code spawn's `--settings` JSON contains `"skipDangerousModePermissionPrompt":true`; Auto, AcceptEdits, and Default spawns' JSON does not.
- [ ] A mission with a claude-code slot on an account that has never accepted the dialog reaches its first turn without human input.
- [ ] `~/.claude/settings.json` and `~/.claude.json` are byte-identical before and after that mission.
- [ ] A runner whose args carry `--settings` spawns exactly as before.
- [ ] The runner form's claude-code Bypass description no longer promises a consent dialog.
- [ ] Trust-dialog behaviour under bypass in a fresh cwd is recorded in the handoff.
