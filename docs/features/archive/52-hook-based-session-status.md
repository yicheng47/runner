# 52 — Hook-based session status

> Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347)
>
> **Record only.** #347 closed won't-do 2026-08-27 (coupling to two CLIs' config schemas); the grid-scraping alternative [#455](https://github.com/yicheng47/runner/issues/455) closed 2026-08-28 — the ask was a better busy/idle detector, not Waiting, and the byte-flow `IdleDetector` with a 2 s threshold covers it. If Waiting is ever wanted, this design is the route: codex 0.150.1 ships a Claude-compatible hook system (`$CODEX_HOME/hooks.json`, `[features] codex_hooks`), so both runtimes now speak the same event names.

## Motivation

Session status today is a byte-flow heuristic: `IdleDetector` (`src-tauri/src/session/pty_runtime.rs:500`) flips Busy on any PTY byte and Idle after 750ms of silence. Both states are guesses, and the most valuable state for a human babysitting agents — *waiting on you* (permission prompt, question) — is not expressible at all: a permission prompt and an idle prompt are the same bytes-then-silence, so no threshold can separate them. TUI animation reads as Busy while the agent is actually blocked on the human; a quiet tool run can read as Idle mid-turn.

Agent CLIs already know their own state and will say so: claude-code and codex both ship hook systems that fire on prompt submission, tool use, permission requests, and turn completion. Orca (source studied 2026-07-25) demonstrated the architecture: hooks as the authoritative status source, heuristics demoted to an explicitly tracked fallback. Runner adopts the signal but rejects Orca's installation model — Orca installs managed hook scripts into user CLI configs because its panes include remotes and user-launched shells; runner spawns every CLI itself and owns argv and env, so hooks can be scoped to the session with nothing installed.

## Scope

- A loopback HTTP receiver in the Tauri backend with a per-app bearer token; hook payloads POST to it, the handler fails open (a broken hook never blocks the agent), and receiver port + token ride the existing per-session env injection.
- Spawn-scoped hook injection, never config mutation: claude-code via `--settings <file-or-json>` (verified: merges additional settings on top of the user's) carrying a runner-generated hooks config; codex via its hooks system (verified live in codex-cli 0.145.0: `pre_tool_use`, `post_tool_use`, `permission_request`, `stop`; definitions in `$CODEX_HOME/hooks.json` gated by `features.hooks`), injected through a runner-managed `CODEX_HOME` mirror or `-c`/`--enable hooks` overrides — whichever spec-time verification supports. The user's `~/.claude` and `~/.codex` are never touched.
- Per-runtime normalizers mapping hook events to `working` / `waiting` / `done` (+ `interrupted`): claude-code `UserPromptSubmit`/`PreToolUse`/`PostToolUse` → working, `PermissionRequest` or AskUserQuestion `PreToolUse` → waiting, `Stop` → done; codex equivalents.
- The normalized model extends `RunnerStatus` (Busy/Idle) with Waiting and Done+interrupted, flows through the existing `session/status` event, and carries a confidence tier (`hook` vs `heuristic`).
- The byte-flow `IdleDetector` stays as the universal fallback tier: any runtime without a hook adapter (shells, future agent CLIs) gets Busy/Idle on day one with zero integration work, and a runtime whose hooks go quiet degrades to the heuristic via freshness decay instead of freezing on stale state.
- Consumers: `chatAttention` gains a needs-you tier above `working`/`unread`; sidebar rollups, mission router nudge gating (features 47/48), the blocked-inbox indicator (feature 50), and `mission_status` pending-asks can all read the richer states.

## Out of scope

- Mutating or managing the user's global CLI configs (Orca's managed-install model, relays, remote transports).
- Replacing the byte-flow detector. It is demoted, not deleted.
- OSC in-band status protocols and interrupt-keystroke inference (Orca's hardening layers) — candidate follow-ups, not v1.
- Per-CLI transcript parsing for status (JSONL reads stay enrichment-only if ever added).

## Implementation phases

1. **Receiver + model** — loopback server, token, normalized status states with confidence tiers, `session/status` wiring, freshness decay.
2. **claude-code adapter** — hooks config generated per spawn, delivered via `--settings`; normalizer + tests.
3. **codex adapter** — verify injection route (`CODEX_HOME` mirror vs `-c` overrides), normalizer + tests; qodercli inherits the claude-shaped adapter once #341 lands.
4. **Consumers** — needs-you attention tier in sidebar/chat surfaces; router gating reads `waiting`.

## Verification

- [ ] A claude-code permission prompt flips the session to `waiting` within a hook round-trip, with no byte-timing involvement; approving it returns `working`.
- [ ] `Stop` flips to `done`; Esc-interrupted turns carry `interrupted`.
- [ ] A shell session (no hooks) behaves exactly as today via the heuristic tier.
- [ ] Killing the receiver or breaking a hook script degrades status to the heuristic tier without blocking or slowing the agent.
- [ ] The user's `~/.claude/settings.json` and `~/.codex/config.toml` are byte-identical before and after a session using injected hooks.
- [ ] Sidebar shows the needs-you state for a hidden pane whose agent is waiting on approval.
