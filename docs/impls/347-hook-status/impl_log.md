# Hook-based agent status — log

Dated record for the #347 program ([README](README.md), [plan](plan.md)). Newest entries at the bottom; keep entries short: what landed, deviations, carries, blockers.

## Current state (update with each entry)

- **Landed**: slice 0 (design + capability audit) in `1218288`; slice 1 (Claude Code hook source, Busy/Idle only) in `9584330`. Both 2026-09-14.
- **Next**: slice 2, the vocabulary and its UI — gated on Jason's review of the canvas frames, not on an unknown.
- **CLI versions verified against**: Claude Code 2.1.270, Codex 0.154.0, TRAE CLI 0.120.52, all checked against installed binaries 2026-09-14.
- **Open**: Windows (slice 1's hook is POSIX-only, fails safe to the baseline); TRAE's documented immediate `idle_prompt` is unverified.

## 2026-09-14 — design and capability audit landed

Ten frames on `design/runner.pen` (ids in the spec's UI design table): vocabulary, pane header, single-pane tab, sidebar rollups, mission workspace, two full screens, three light equivalents. Designing it produced decisions the spec had not carried — a one-pane tab has no pane header at all, so the tab bar must carry status; the rollup priority had no slot for Status unavailable; the generic needs-you glyph is a triangle because circle-exclamation is Error; the mission avatar's presence dot drops back to process liveness.

The capability audit was redone against installed binaries and was wrong in both directions on documentation alone: it had treated TRAE as a Codex derivative with no hook story (it has 13 events and is Claude Code-shaped), and credited Codex with question and surfaced-prompt events it does not have. Four corrections followed, each recorded in the spec: the mechanism is a port of cmux rather than a new design; `--dangerously-bypass-hook-trust` is the sanctioned path for launcher-injected hooks, not a workaround; status gates delivery only for needs-you, because delivery injects a nudge rather than a message body; and terminal observation is a permanent baseline rather than something the adapters replace.

## 2026-09-14 — slice 1 landed

`9584330`, via mission `01M2EM1GJQSHQT783Y7W8TEVQR` on `codex-crew` (~65 min, brief at [`347-slice-1-claude-hook-bridge.md`](../archive/gpui-rewrite/briefs/347-slice-1-claude-hook-bridge.md)). Extends the per-spawn `--settings` injection that already carried the `claude_rekey` `SessionStart` hook, so nothing is written into the user's config. 700 backend tests; `make verify` run independently before landing.

Two mission-goal instructions turned out to be wrong and were reversed mid-flight. **Do not key Idle on `Stop`** was built on the premise that `Notification(idle_prompt)` is a turn boundary; the reviewer found `messageIdleNotifThresholdMs:60000` in the 2.1.270 binary, making it a delayed, user-disableable idle notice. Keying on `Stop` is safe because Busy/Idle gates no behaviour — a continuation costs one wrong glyph, self-corrected. **Align the two spawn gates** narrowed the rekey stale-report cleanup for runners with a user-supplied `--settings`; caught in re-review and restored.

Driving the real app then found the blocker the documentation pass had missed: Claude Code publishes no interrupt event and its `Stop` excludes interruptions, so an Esc-interrupted turn stayed Busy forever with the baseline latched off — worse than the byte detector it replaced. Recovered through the input layer instead of a timeout. Escape is provisional and display-only: excluded from completion recording so it cannot fire a false turn-finished badge, and skipped in the tab's completion loop so it cannot consume a sibling pane's real one. That suppression is a slice-1 compromise; see [README](README.md#open).
