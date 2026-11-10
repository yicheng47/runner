# #1246 — Agent launch options

Jason approved the codex trio for this mission on 2026-11-10. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1246-agent-launch-options` on `feat/1246-agent-launch-options`, based on `origin/main` at `1cd12945485634416381f854e676825a581cc5b6`.

The spec is [`docs/features/1246-agent-launch-options.md`](../../features/1246-agent-launch-options.md); it wins on behavior. The design is `runner/desktop/specs/1246-agent-launch-options.pen` in `~/repos/yeelinks/yeelinks_design` (pull it first, read it through the Pencil MCP only, never edit it); it wins on layout and copy, except the card caption the spec rewrites. Frames: `VvFIf` and `LoBEK` (Codex, Chat and Mission tabs), `LOEzL` (Claude Code), `LJcuZ` (pi), `dkDAX` (switches per runtime).

## Problem

Settings → Agents has no launch options and shows no launch command. People want Bypass in chats, Codex without its ~30 MB remote plugin catalog download on every start (openai/codex#47483), and a way to turn status hooks off in chats (folded in from #1051). The page also gets a read-only Chat/Mission command preview.

## Read first

- `crates/runner-core/src/protocol/runtime_metadata.rs`: `Permissions`, `PermissionMode`, Codex's `mission_bypass`.
- `crates/runner-daemon/src/session/manager/spawn_spec.rs`: how chat and mission args are composed, `mission_dir_args`.
- `crates/runner-daemon/src/runtimes/codex/mod.rs` (`launch_args`, `codex_status_args`, `inject_codex_hooks`, the headless `codex exec fork`), `codex/codex_status.rs`, `claude_code/mod.rs` (`claude_settings_args`), `pi/mod.rs`, and the other runtimes' hook installation.
- `crates/runner-daemon/src/session/manager/startup.rs`: the hooks vs output-settle startup gate.
- `crates/runner-daemon/src/db/app_state.rs` (`_app_state`, `set_runtime_override`) and `ops/runtime.rs`.
- `crates/runner-app/src/surfaces/settings/agents/` (`render.rs` Defaults section, `mod.rs`).

## Deliverables

- The Launch card on every agent page, with the switches and scope labels the spec's table names, the Command row with Chat/Mission tabs, Copy and the annotated command block, and the caption under the card.
- Switch behavior exactly as the spec's Switch behavior section says: Bypass for every Codex and Claude Code chat after stripping stored role permission flags (Jason’s 2026-11-10 correction); the plugin catalog flag on every Codex launch, fork included; status hooks off in chats keeping the identity hook; missions unchanged by the Bypass and hooks switches; changes apply at next start.
- The options persist in the daemon's `_app_state` under one key, with no migration; client-protocol requests to set an option and to fetch the preview, built from the same code as real launches. The CLI is unchanged.
- Guide and architecture pages the spec names, in English and `zh-CN`.
- Non-goals: Bypass for other runtimes, changing a role's stored Codex Bypass, the #1051 comparison, editing agents' own config files.

## Validation and handoffs

The coder implements, adds the spec's automated tests, and runs the affected `runner-daemon`, `runner-core` and `runner-app` tests, workspace Clippy with warnings denied and `cargo fmt --all --check`, in this worktree's own target directory. The coder hands each candidate to the reviewer, who reviews the diff only and names the live checks in its clean review.

After a clean review, `qa` runs the spec's five live checks in the development app on the reviewed commit, taking the qa lock first, with Codex and Claude Code chats on an inexpensive model and low effort in fresh `/tmp` folders. Process arguments can be read with `ps`. qa restores every launch switch it changed to its default before cleanup, reports pass or fail per check with screenshots, and records them in the PR description.

## Authorization

The codex trio may commit, push this branch and open one PR against `main`, with one risk label, `needs-live-test` until qa passes, and the `core-review` label the workflow adds.
