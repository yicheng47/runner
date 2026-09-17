# 624 — Status details — implementation plan

Plan for feature [624](../features/624-status-details.md) ([#624](https://github.com/yicheng47/runner/issues/624)). The spec says *what*, with the per-surface strings and the hook-capability table; this file carries the decisions that bind, the mission split, and what has landed. The status model, its adapters and their decisions are the archived [347 program record](./archive/347-hook-status/README.md); the Windows port is [610](./610-windows-hook-status.md). Nothing there changes.

## Status (2026-09-17)

Design signed off by Jason on 2026-09-17: four frames on `design/runner.pen` (`z92Zy9`, `qKf92`, `HiZ5G`, `bqwlq`). Mission 1 runs on `feat/624-status-details` with the codex peer crew; brief at [`archive/gpui-rewrite/briefs/624-m1-status-details.md`](./archive/gpui-rewrite/briefs/624-m1-status-details.md). It started with the three UI-only items plus `failed_since`; on 2026-09-17 Jason folded the working detail into it, so the whole feature lands in one PR.

## Decisions that bind

1. **Nothing new in a label.** Detail and elapsed time appear in the tooltip and the runner card subtitle only. The pane header, tab bar and sidebar rows keep the shipped label and glyph for every state; the one sidebar change is the failure glyph. This is 347's own rule ("work detail belongs in the tooltip, not in a title that rewrites itself every few seconds") applied to the deferred items.
2. **Rollups count, they never time or name a detail.** `1 approval needed · 1 working` stays exactly that. A single-pane row is the pane, so its tooltip is the pane's tooltip and carries whatever the pane's does.
3. **`failed_since` mirrors `error_since`.** Set on the running session when a `Failed` outcome arrives, cleared when the pane is viewed and when the outcome clears, counted by rollups as `response failed` next to `error`, ranked with error. Not persisted: a live-process outcome does not survive a restart today and still does not. The pane's own label is still read from the outcome and is untouched by acknowledgement.
4. **Elapsed format is fixed:** seconds under a minute, whole minutes under an hour, then hours and minutes (`45s`, `2m`, `1h 12m`). The tooltip refreshes every second while shown; the card shows minutes only, nothing under a minute, and redraws on the minute. Time never changes colour, precedence, ordering or delivery.
5. **The working detail is a hook fact.** `Using tools` spans `PreToolUse` to the owning result with the ownership the waits already use; `Compacting context` spans `PreCompact` to `PostCompact`, or on Copilot to the next work event or `Stop`; compaction outranks a tool in flight. Baseline Working never carries a detail. Claude Code registers the two compaction hooks; the reporter and feed are unchanged.
6. **The nonblocking ask is cut.** No hook publishes the flag (Codex exposes it only over app-server, and Codex has no Answer needed state anyway). It stays in the 347 "unsupported until observable" list.
7. **One mission, one PR.** All four items land together: the three UI-only ones in `agent_status.rs` and its consumers plus `failed_since`, and the working detail in the observation, the three adapters and Claude's hook registration. The mission stops at a clean working-tree review; Claude lands the PR after its own pass and green CI on both platforms. (Planned as two missions at first; Jason folded them on 2026-09-17 because the adapter change is small.)

## Mission sequence

| Mission | Scope | State |
| --- | --- | --- |
| 1 | `Idle · Last response interrupted` tooltip and `Idle · Interrupted` card line; wait timers in tooltip and card; `failed_since` with sidebar glyph, rollup count and acknowledgement; `WorkDetail` on the observation, Claude `PreCompact`/`PostCompact` registration, Using tools and Compacting context in the three adapters, tooltip and card detail; smoke record | started 2026-09-17 |

## Log

- 2026-09-17 — Hook-capability audit of the five deferred items against the three adapters and the registered hook lists (table in the spec). Four design frames built as copies of the 347 spec rows and signed off. Spec, plan and mission-1 brief committed on `feat/624-status-details`; mission 1 started with the codex peer crew. Later the same day Jason folded the working detail (planned as mission 2) into mission 1; brief, plan and spec updated and the crew told through the channel.
