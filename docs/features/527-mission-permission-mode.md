# 527 — Mission permission mode, bypass by default

Tracking issue: [#527](https://github.com/yicheng47/runner/issues/527). Status: specced 2026-09-09 (rewritten the same day from a per-mission column to an app setting), P1, designed. Design: `design/runner.pen`, `Settings — Missions` (`ez53n`) — a new pane under Chat in the settings nav (`cmp/SettingsNav` → `nav_missions`), holding `row_Default crew` and `row_Mission permissions` (`gbhW1`) with the select's open state beside it (`spec_permission_select_open`); `Settings — General` (`K8FVAY`) loses Default crew.

> Rewritten 2026-09-09. The first draft persisted a mode per mission (`missions.permission_mode`) with a per-slot override (`slots.permission_override`), a Start Mission select, and a migration. Jason's call: the mode is a rule about how missions run, not data about one mission — make it an app setting, default Bypass, no schema change. The first draft stays in git history.

## Motivation

A permission prompt inside a mission slot is never a real control point. Nobody is watching that PTY, so the prompt is a silent stall: the lead waits on a handoff that never arrives, and the human sees nothing. The byte-flow `IdleDetector` (`crates/runner-backend/src/session/pty_runtime.rs`) reports the stuck slot as idle, which is what a finished slot looks like too. Spec [52](./52-hook-based-session-status.md) closed the hook-based status route, so the feed will not learn to show this.

Auto mode does not remove the stall. claude-code's `--permission-mode auto` runs a classifier that still stops for a human on some actions; on 2026-09-09 it denied a `git push` twice in one session. codex's `on-request` asks to escalate whenever the sandbox blocks a command. In a direct chat the human answers; in a mission slot nobody does.

Today permission mode is a per-runner property. `ops::runner::create` / `update` write the chosen mode onto the row's `args` at create time through `router::runtime::permission_mode_args` (`crates/runner-backend/src/router/runtime.rs:258`), default Auto (`ops::runner::default_permission_mode`). Missions inherit whatever each runner carries; a slot runtime override starts from the default mode (`session::manager::resolve_runtime_override`, `crates/runner-backend/src/session/manager/mod.rs:1328`). There is no mission-level knob.

The controls that actually protect a crew are elsewhere: worktree isolation ([403](./403-mission-worktree-isolation.md)), the coder/reviewer loop, and the no-commit / no-push rules in the crew prompts. A permission prompt adds nothing to those; it only removes the mission's ability to finish.

## Behavior

### One app setting

`AppSettings.mission_permission_mode` (`crates/runner-app/src/app_settings.rs`, the settings JSON — no database change), three values, default **Bypass**:

- **Bypass** — every mission slot spawns with the runtime's bypass flags. The default.
- **Auto** — every slot spawns with the runtime's auto flags (today's effective behaviour for a default runner).
- **Runner default** — slots spawn with whatever their runner row carries, exactly as today.

The setting is read at spawn time for every slot of every mission, including resume and fork: a slot spawned after the setting changes gets the new mode, a running slot is untouched. There is no per-mission or per-slot value; a runner whose row carries its own permission flags is converged to the setting's mode like any other (the runner's other args are never stripped).

### Settings → Missions

A new **Missions** pane in the settings nav, directly under Chat in the App group (`SettingsPane::Missions`, slug `missions`, a rocket icon). It opens with one card, **Defaults**: **Default crew** moves here from General (it is a mission default — "pre-selected when starting a new mission"), and **Mission permissions** sits under it as a `SettingsRow` with a `StyledSelect` (`Bypass` / `Auto` / `Runner default`; the open state shows each option with a one-line description). Subtitle: *"Applied to every slot when a mission starts. Bypass never prompts — nobody is watching a mission slot to answer. Direct chats keep their runner's own mode."* General keeps Defaults (working directory, file links) and Window. Later mission-wide settings (worktree isolation from 403, auto-archive) land on this pane rather than in General.

### Runtime mappings

Applied through the existing `router::runtime::apply_permission_mode` (strip the runtime's permission flags, append the canonical pair), so a runner row carrying its own flags converges to one shape.

- claude-code Bypass: `--permission-mode bypassPermissions` (unchanged).
- trae Bypass: `--permission-mode bypass_permissions` (unchanged).
- **codex Bypass for mission slots: `--ask-for-approval never --sandbox danger-full-access`.** Today's codex Bypass pair keeps `--sandbox workspace-write`. With `never`, codex does not ask to escalate out of the sandbox, so a command that needs the network or writes outside the tree (`cargo` fetching crates, `gh pr create`, `git push`) fails silently. The runner-level Bypass mapping (direct chats, a human watching) is left as is.
- shell and unknown runtimes: no-op, as today.

### Where it shows

The mission metadata panel gains one line next to the crew link, `Permissions: bypass` (or `auto` / `runner default`), so the behaviour a mission was started with is visible even after the setting changes. Read from the session's recorded argv, not from the setting, for exactly that reason.

### Direct chats

Unchanged. A direct chat has a human in front of it; the runner row's mode stays the contract.

## Non-goals

- A per-mission or per-slot mode. If a read-only reviewer is ever wanted, that is a `--sandbox read-only` argument on the reviewer's runner row, which is never stripped; the mode setting only replaces the permission-mode flags.
- A schema change. Nothing about missions or slots is persisted for this.
- Detecting a permission prompt from the terminal grid. Spec 52 and #455 closed that route; this spec removes the prompt instead of finding it.
- A global "bypass everything" default for runners. Runner-level modes stay per runner; only mission slots follow the setting.
- Sandboxing beyond what the runtimes offer. Worktree isolation (403) is the blast-radius control and is not changed here.

## Implementation Phases

1. **Setting + spawn.** `mission_permission_mode` on `AppSettings` with serde default Bypass (pre-feature settings files load as Bypass); the value threaded to the backend spawn path the way `default_runtime` / `file_link_editor` are (`AppSettings` → `AppCore` or the spawn input for mission slots), resolved per slot at spawn through `apply_permission_mode`; the codex full-access mapping for mission-slot Bypass; resume and fork spawn through the same path. Tests: each mode's argv per runtime, a runner row with its own flags converging, a runner row with unrelated args keeping them, resume honouring the current setting, pre-feature settings decoding to Bypass.
2. **UI.** `SettingsPane::Missions` between Chat and Appearance (`settings_page.rs`: enum, slug, label, icon, the App pane list, route, nav search entry); the pane per the frame — Default crew moved out of General, the permissions `SettingsRow` + `StyledSelect` — saving through the existing debounced settings write; the metadata panel line. A `VisualTestContext` test pins the pane at two rem sizes; the settings-nav search count test adjusted for the new entry.
3. **Optional, later — permission-prompt signal.** For slots resolved to Auto or Runner default, inject a claude-code `Notification` hook with matcher `permission_prompt` through the `--settings` composer Runner already uses (`router::runtime::claude_settings_args`), posting a needs-you signal to the feed. codex 0.150+ has Claude-compatible hooks; verify before promising it there. Its own issue when felt.

## Verification

- Fresh settings (no key): Settings → Missions shows Default crew and Mission permissions at `Bypass`; General no longer shows Default crew; start a Peer Coding Crew mission on the runner repo, have the coder run `git push` to a scratch branch and `gh pr view`. No prompt in either slot; the codex slot reaches the network; the metadata panel says `Permissions: bypass`.
- Switch to Auto, start another mission: the claude slot stops on the push, proving the setting is honoured; the earlier mission's metadata still says bypass.
- Runner default: a runner whose row carries `--permission-mode plan` spawns with exactly that.
- Quit and relaunch with auto-resume on: resumed slots carry the flags for the current setting.
- `make verify` green; no new migration under `crates/runner-backend/migrations`.
