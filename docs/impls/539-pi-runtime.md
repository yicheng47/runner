# pi runtime — implementation plan

Plan for feature [539](../features/539-pi-runtime.md) ([#539](https://github.com/yicheng47/runner/issues/539), milestone 0.11). The spec carries the per-site inventory with a phase per row, the probe evidence and the decisions; this file is the mission sequence and what has landed.

## Status (2026-09-18)

Spec refreshed on 2026-09-17 from a live probe of pi 0.85.1 on macOS; nothing is implemented. Mission 0 is done: the design is `design/specs/539-pi-runtime.pen`, frame `Spec — pi runtime (539) · v1` (`h5rNdS`), the first spec in its own file. Jason chose the mono silhouette in the theme foreground (decision 11) and signed off decision 6. Mission 1 is briefed in `docs/impls/archive/gpui-rewrite/briefs/539-m1-pi-runtime.md`; a codex peer crew is the recommendation, as for 540.

## Missions

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| 0 | Spec phase 0: the pi mark beside the four shipped marks, 12/14/16/24 px, both themes, three tints drawn and one chosen by Jason. | Jason + inline | Done 2026-09-18 |
| 1 | Spec phase 1, the runtime: enum, definitions, `model_effort_args` with lowercased thinking, the split composer in `router/prompt.rs` with the byte-identical test for the other four runtimes, the system-prompt file written at start and resume, `--append-system-prompt` on every spawn, the lead's goal as the only first turn behind `--`, resume and fork plans, the one-glob conversation probe, runtime defaults, catalog entry, `models/pi.rs` discovery, `--approve` for slots, `PI_SKIP_VERSION_CHECK`, the empty permission arms, Skills caption, provider mark, every enumerating test, README columns, arch §5 and §6. `hooks_supported(Pi)` stays false. | Picked at launch (540 ran as a codex peer) | Ready to brief |
| 2 | Spec phase 2: `session/pi_status.rs` with the embedded `-e` extension, install at startup, `HookStatusWatcher::Pi`, the status and rekey env vars, decision 5's mapping, decision 10's rekey drop file, `hooks_supported(Pi)` on, README hooks cells, `docs/tests/539-pi-hooks-smoke.md`. macOS and Windows in one mission, because the reporter is Node. | Picked at launch | Not started |
| 3 | Spec phase 3: Jason's smoke on macOS and JASONPC, `pi-first-turn.ndjson`, `pi_runtime_smoke.rs`, archive the spec and this plan. | Jason + inline | Not started |

## Decisions that bind

The spec's decisions 1–10. For mission 1: the three prompt layers go on `--append-system-prompt <file>` for every spawn and only the lead's goal is a first turn, with the other four runtimes' bodies pinned byte-identical (1); keys are caller-assigned with no capture thread, a missing conversation keeps its key, and the probe reads pi's default session directory only (2); fork is one direct spawn with its own prompt file (3); `--approve` goes on mission slots only, never chats (6); every permission function is empty for pi (7); quiet start is the env var, not `--offline` (8); the mark is the pixel-π silhouette in `theme::text()`, one path, no colour variant (11). For mission 2: hooks load through `-e` from app data and nothing is written into `~/.pi` (4); Idle comes only from `agent_settled`, `session_shutdown` clears waits, and pi older than 0.84.4 gets estimated status (5); the rekey drop file reuses `ClaudeSessionKeyWatcher` with no new Rust (10).

## Risks to watch

- **Prompt composition is shared.** The split composer must leave claude-code, codex, TRAE and Copilot bodies byte-identical; the pinning test lands in the same PR, before the pi branches. The resume path composes the system-prompt body from the current rows, which no runtime does today, so it needs the mission's crew and roster on hand where resume runs.
- **Windows is unprobed.** The npm `.cmd` shim on the batch first-turn path, Git Bash for the bash tool, and Node's `appendFileSync` beside Runner's open feed handle all get their first run on JASONPC in mission 3. The session slug is not a risk: pi's `getDefaultSessionDirPath` maps `\` and `:` to `-` the same way it maps `/`.
- **Keyboard.** pi asks for the kitty keyboard protocol and Runner does not answer; legacy key encoding is expected to work, but Shift+Enter and Esc are the keys to check.
- **Model discovery output is a table, not JSON.** `pi --list-models` prints aligned columns; the parser keys on the header row and the first two columns. A future pi that changes the layout degrades to the default option through the existing `InvalidOutput` path.

## Log

- 2026-09-17 — spec refreshed from a live probe of pi 0.85.1: `--session-id` accepts a Runner UUID, `--append-system-prompt` takes a file or inline text and is composed at startup rather than persisted (codename test: a resumed session without the flag does not know it, with the flag it does), `--fork` takes a new `--session-id`, `--thinking` is case-sensitive, `--` protects a leading dash but not a leading `@`, extension events recorded for one turn, `project_trust` reaches a `-e` extension before the trust dialog and the dialog emits no `ui_prompt_start`, terminal modes and `--list-models` output recorded, the session slug read from `dist/core/session-manager.js`. Orca's and herdr's pi extensions read as prior art. A first pass of the spec claimed trust was invisible to extensions; that was a probe script that never passed its environment to the child, corrected the same day. Jason moved the system-prompt channel from a follow-up phase into mission 1 and widened it from the persona alone to all three prompt layers.
- 2026-09-18 — mission 0: the spec frame drawn from the 540 frame's shape, with the mark three ways beside the four shipped marks, the rail, the Agents row, trust as decision 6 and the tint options live and stopped on both themes. Jason signed off decision 6, noted that mission agents bypass everything by default (already true through #527), and chose the theme-foreground silhouette (decision 11). The frame moved into `design/specs/539-pi-runtime.pen`, the first spec in its own file: `runner.pen` had 152 top-level frames and a 32,000 px spec row, and changed in 43 of its last 44 commits. `cmp/MarkPi` stays on `runner.pen`; the three-colour component lives only in the spec file.
