# Hook-based agent status — slice plan

Slices for [#347](https://github.com/yicheng47/runner/issues/347) ([spec](../../features/347-hook-based-session-status.md), [README](README.md), [log](impl_log.md)). Each lands green on its own. The spec's own "Implementation phases" section is the narrative version; this is the unit of work.

| Slice | Scope | State |
| --- | --- | --- |
| 0 | Design and capability audit: ten canvas frames, per-runtime matrix verified against installed binaries | Landed 2026-09-14, `1218288` |
| 1 | Claude Code hook *source* behind today's Busy/Idle — injection, status file, notify watcher, `hook` source and latch, interrupt recovery | Landed 2026-09-14, `9584330` |
| 2 | The status vocabulary and its UI — Working / Needs you / Ready plus lifecycle and observability states, pane header, single-pane tab bar, sidebar rollups, mission workspace | Implemented on `feat/347-status-ui`; required checks and working-tree review clean; PR workflow authorized, not landed. PR authorized after clean review; human smoke test later. |
| 3 | Codex adapter | Not started |
| 4 | TRAE CLI adapter | Not started |
| 5 | Deferred details, one at a time | Not started |
| 6 | Remove title-spinner classification | Trigger: after slice 3 |

## Slice 2 — vocabulary and UI

The one that makes the feature visible, and the only place `Answer needed` appears. Driven by Claude Code's `Notification` subtypes, which are confirmed present in 2.1.270. Implement in two internal passes: the normalized backend state model plus the pane header first, then sidebar rollups and the mission workspace. Both passes are implemented in mission `01M2EX7VV58BNFWBN7G1JQVPWC`. Pencil reconnected and the ten design frames were inspected; rendered-app visual checks and Jason's smoke test on another PC remain pending. Interruption is now a separate outcome and never synthesizes a successful completion; its tooltip remains slice 5. Per-pane unread/error acknowledgement persists in migration 0021. After clean working-tree review and required checks, Jason authorizes a PR and CI follow-through, with no merge. [Mission brief](../archive/gpui-rewrite/briefs/347-slice-2-status-ui.md).

## Slice 3 — Codex

Needs `--enable hooks --dangerously-bypass-hook-trust` with hooks passed as `-c` overrides, so nothing is persisted and the trust gate never applies. `PermissionRequest` is raw only, so it needs the hold-and-cancel interval. No `Answer needed` — Codex publishes no question event of any kind. `Interrupt` exists here and does not on the other two.

## Slice 4 — TRAE CLI

Verify the documented immediate `idle_prompt` against the binary first; Claude Code's manual would have supported the same claim an hour before its binary contradicted it. Config surface is a `hooks` list in `~/.trae/traecli.yaml` or `.trae/traecli.yaml`, or a `hooks.json`, merged by execution identity so composition is safe. Experimental in Runner, macOS-default-on, unvalidated on Windows.

## Slice 5 — deferred details

`Working · Compacting context` and `Using tools`; `Error · Response failed` (Claude Code only, via `StopFailure`); the interruption outcome in the Ready tooltip; elapsed time on a wait; the nonblocking-ask `Still working` case. Each is additive to the slice-2 layout.

## Slice 6 — remove the title heuristic

Delete title-spinner classification when every runtime that animates its title has an adapter — Claude Code and Codex, so after slice 3. Byte activity stays as the permanent baseline. Keep the heuristic if a runtime turns up that animates and has no usable hooks; it is twelve lines and comes back cheaply.

## Unscheduled

**Windows native hook transport.** Slice 2 makes baseline-only selection explicit and tests it, retaining the existing rekey hook. The shared status model and UI apply on Windows; the POSIX status helper does not. A native bridge and native Windows verification remain separate carries. See [README](README.md#open).
