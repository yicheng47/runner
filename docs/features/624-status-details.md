# 624 — Status details: working detail, interrupted outcome, wait timers, failure attention

> Tracking issue: [#624](https://github.com/yicheng47/runner/issues/624)
> Priority: P2. Platforms: macOS and Windows, one implementation.
> Continues [347](./archive/347-hook-based-session-status.md), whose slice 5 deferred these details, and lands on top of [610](./610-windows-hook-status.md), which put the three hook adapters on both platforms (PR #627).
> Design: `design/runner.pen`, band `STATUS DETAILS (624)` right of the 606 and brand-mark specs. One frame per item: `z92Zy9` working detail, `qKf92` interrupted, `HiZ5G` wait timers, `bqwlq` failure attention. Jason signed the four frames off on 2026-09-17.
> Implementation plan: [`docs/impls/624-status-details.md`](../impls/624-status-details.md).

## Motivation

The shipped status model shows Working, Idle, Approval needed, Answer needed and Response failed, and the adapters already receive the events for finer detail; today that detail is collapsed into the plain labels. A failed response in a pane out of view is invisible, an interrupted turn reads like a finished one, and a wait gives no sense of how long it has been open. Each item below is additive to the shipped pane header, tab bar, sidebar row and mission card layout, and none of them changes what gates crew delivery.

## What the hooks can prove

Checked on 2026-09-17 against the adapters in `crates/runner-backend/src/session/` and the hook lists Runner registers in `router/runtime.rs`. Since #627 the adapters run on both platforms, so the table holds for macOS and Windows alike.

| Item | Claude Code | Codex | Copilot |
| --- | --- | --- | --- |
| Using tools | `PreToolUse` / `PostToolUse` registered and mapped; pending tools already tracked by ID | registered and mapped | registered and mapped, no tool ID |
| Compacting context | the CLI publishes `PreCompact` / `PostCompact`; Runner does not register them yet | `PreCompact` / `PostCompact` registered and mapped to Working | `PreCompact` registered and mapped; Copilot publishes no `PostCompact` |
| Interrupted outcome | Runner's own Escape / Ctrl+C signal, as shipped | native `Interrupt` hook plus the correlated `turn_aborted` record | Runner's own Escape / Ctrl+C signal, as shipped |
| Wait timestamp | every interaction carries `since` | no waits, by 347 decision 9 | every interaction carries `since` |
| Failed outcome | `StopFailure` | no fatal-failure hook | `ErrorOccurred` registered but unmapped until a live fatal fixture exists |
| Nonblocking ask | every ask blocks | the flag exists only over app-server, absent from hooks and rollout | every ask blocks |

The nonblocking-ask `Still working` case is therefore cut from this feature: no hook publishes it and Codex has no Answer needed state to attach it to. It stays in the 347 record's "unsupported until observable" list.

## Status model

`AgentObservation` gains one field, a working detail, and nothing else:

- `detail: Option<WorkDetail>` with `WorkDetail::UsingTools` and `WorkDetail::CompactingContext`. Set only from hook events, never from the baseline. Cleared whenever the activity leaves Working and whenever a new turn starts.
- `Using tools` starts at `PreToolUse` and ends at the owning `PostToolUse` / `PostToolUseFailure` or the correlated transcript result, using the same ownership the waits already use. While any tool is in flight the detail is `UsingTools`.
- `Compacting context` starts at `PreCompact` and ends at `PostCompact`. On Copilot, which publishes no `PostCompact`, it ends at the next work event or `Stop`. Compaction outranks a tool in flight while it lasts.
- Claude Code registers `PreCompact` and `PostCompact` in `claude_settings_args`; the reporter and the feed are unchanged.

The outcome and the wait timestamp already exist: `TurnOutcome::Interrupted` on all three runtimes, `TurnOutcome::Failed` on Claude Code, and `HumanInteraction.since` on every wait. `AgentStatus` gains `failed_since: Option<i64>`, the failure counterpart of `error_since`: set when a `Failed` outcome arrives on a running session, cleared when the pane is viewed and when the outcome clears. Nothing new is persisted; a live-process outcome does not survive a restart today and still does not.

## UI rules

The rule that binds every surface: the pane header, the single-pane tab bar and the runner card subtitle show the detail in the label, in one short form produced by one function so the three can never drift: `Idle · Interrupted`, `Approval needed · 2m`, `Working · Using tools`, `Working · Compacting context`. Wherever that label is visible there is no tooltip at all; only a glyph-only indicator (a narrow header, a sidebar row) keeps a tooltip as its name, with the seconds and the concurrent conditions. Jason overturned the 347 tooltip-only rule on 2026-09-17 after testing the first build (nobody hovers a header) and dropped the duplicate tooltip after testing the second. The label slot reserves its width so a toggling detail never shifts the title or the buttons; when the pane is too narrow the detail drops first, then the label goes icon-only as in 347. Sidebar rows stay glyph-only and gain one glyph, for a failed response.

### Working detail (`z92Zy9`)

- Pane header and single-pane tab bar: the label reads `Working · Using tools` or `Working · Compacting context`, plain `Working` between details. No tooltip while the label is visible.
- Runner card subtitle: `Working · Using tools` or `Working · Compacting context`, the label at its current weight and the detail regular, as the 347 card frame draws it. Plain `Working` between details. Anything longer than the 216 px line truncates with an ellipsis; the card never grows.
- Sidebar: unchanged. A single-pane row's tooltip is the pane's tooltip and therefore carries the detail; multi-pane, project, section and mission rollups count working panes and never name a detail. The mission tab strip keeps the glyph only.
- An estimated Working never carries a detail, and a needs-you wait outranks both details as it does today.
- A manual `/compact` from Idle fires no Stop on Claude Code and Codex. It shows `Working · Compacting context` while it runs and returns to Idle with the previous outcome when it ends; each adapter decides at `PreCompact` whether a turn was live and keeps that across the compact-sourced `SessionStart`, which must neither reset it nor drop the observation to Unavailable. An in-turn compaction resumes Working.

### Interrupted outcome (`qKf92`)

- Pane header and tab bar: the Idle ring is unchanged. The label reads `Idle · Interrupted` when the last turn ended with `TurnOutcome::Interrupted`; a glyph-only indicator says `Idle · Last response interrupted` in its tooltip.
- Runner card subtitle: `Idle · Interrupted`, muted, same ring. An interruption is not a failure and never turns red or amber.
- Sidebar: nothing to chase. An interrupted pane out of view gets no unread dot (there is no response to read), contributes to no rollup count, and a single-pane row's tooltip says the same sentence as the pane header.
- The outcome lasts until the next prompt, a resume or a restart; new work clears it. Baseline sessions keep `Idle · estimated from terminal activity` and never claim an interruption. The tooltip never says whether the outcome came from a hook or from Runner's own interrupt signal.

### Wait timers (`HiZ5G`)

- Elapsed is now minus the interaction's `since`, per interaction. With several waits open the pane reports its oldest, which is also the one a rollup click opens.
- Format: seconds under a minute (`45s`), whole minutes under an hour (`2m`), then hours and minutes (`1h 12m`).
- Pane header and tab bar: the amber is unchanged. The label ends with the elapsed minutes once the wait is a minute old, `Approval needed · 2m`, redrawn on the minute and plain under a minute. A glyph-only indicator's tooltip has the seconds, `Waiting for you to approve a command or plan · 2m`, `Waiting for your answer · 45s`, and refreshes every second while it is shown.
- Runner card subtitle: `Approval needed · 2m`, `Answer needed · 1h 12m`; the state keeps its amber and its weight, the timer is regular and muted. Minutes only: under a minute the line stays at the plain label, and the card redraws on the minute, so a rail of cards never flickers with seconds.
- Sidebar: rollups count and never time. `1 approval needed · 1 working` stays exactly that. A single-pane row's tooltip is the pane's and ends with the time, with a masked unread response appended as a second clause; a row whose dominant glyph is the unread dot keeps `1 unread response` as in 347.
- Time never changes colour, precedence, ordering or delivery. No timer on Working, Idle or errors: a long turn is not a wait.

### Failure attention (`bqwlq`)

- Sidebar single-pane row: the red `circle-alert` glyph in the attention slot, the same glyph an exited process shows; the tooltip `Response failed · Agent is still connected` separates the two.
- Rollups (multi-pane tab, project, section, mission row): Response failed ranks with error, above needs-you, working, unread and unavailable, and the tooltip lists it as its own count, `1 response failed`, next to `1 error`, so a failed turn and a dead process are never merged. Clicking a rollup opens the oldest unresolved error or failure first.
- Acknowledgement: viewing the pane clears the sidebar attention, as it does for error; the pane's own header label and the runner card stay `Response failed` until the next prompt, a resume or a restart. The header, its tooltip and the card are unchanged from 347.
- Source today is Claude Code's `StopFailure`; Copilot's `ErrorOccurred` joins once a live fatal fixture lands, and Codex publishes no fatal-failure hook. Baseline sessions never claim a failure.

## Out of scope

The nonblocking ask (cut above); any new state; any approval or answer control in Runner; Codex human waits; Copilot `ErrorOccurred` mapping; Claude MCP URL and browser flows; the title-heuristic removal ([#625](https://github.com/yicheng47/runner/issues/625)); light-theme design frames (the 347 light axis covers the shared vocabulary).

## Implementation phases

1. Design: done, the four frames above, signed off 2026-09-17.
2. Mission 1, codex peer crew: all four items in one PR: the three UI-only ones (interrupted tooltip, wait timers, failure attention) plus `failed_since`, and the working detail (model field, the three adapters, Claude's two new hook registrations). Planned as two missions; folded into one on 2026-09-17.
3. Smoke on both platforms in the shape of `docs/tests/347-status-ui-smoke.md`, recorded in `docs/tests/624-status-details-smoke.md`.

## Verification

- Unit: `StatusPresentation` tooltip and card strings for every state above, including elapsed formatting at the three boundaries; `StatusRollup` priority and tooltip with a failed response alongside an error and a wait; `failed_since` set, cleared by viewing and cleared by the next turn; the working detail set and cleared per adapter from synthetic reports, including Copilot's end-on-next-event rule and Claude's registration list.
- Smoke, per platform and runtime: a tool-heavy prompt shows `Working · Using tools` in the tooltip and on the card; `/compact` shows `Working · Compacting context`; Escape mid-turn shows `Idle · Last response interrupted` and no unread dot; an approval left open shows a rising timer in the tooltip and `· 1m` on the card after a minute; a forced `StopFailure` raises the red glyph in the sidebar and clears it on viewing the pane while the header label stays.
- Every existing adapter and status-UI test unchanged.
