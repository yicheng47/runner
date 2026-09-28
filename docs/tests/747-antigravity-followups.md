# 747 — Antigravity CLI follow-ups validation

> 2026-09-28, macOS, installed `agy` 1.2.12. This extends the [original smoke checklist](644-antigravity-smoke.md). Tests run in this branch's worktree; live CLI probes used disposable temporary directories and the existing sign-in. No Runner app was started or driven.

| Check | Result | Evidence |
| --- | --- | --- |
| Active conversation after `/new` | Pass, CLI PTY and automated watcher test | A first turn logged `Created conversation A` then `Streaming conversation A`. `/new` logged `Created conversation B` and `Streaming conversation B` before the next message. The next message used B. The watcher follows the stream line for every switch. |
| Active conversation after `/fork` | Pass, CLI PTY and automated watcher test | `/fork` logged `Forked conversation B -> C`, followed by `Streaming conversation C`. |
| Active conversation after `/resume` | Pass, CLI PTY and automated watcher test | Selecting B in `/resume` logged `Streaming conversation B`, with no `Resuming conversation` line. The stored key is updated; the existing Runner resume test verifies `--conversation <stored key>` on the next spawn. A live Runner relaunch remains pending. |
| Repeated changes, lazy creation, stale process | Pass, automated | The manager test exercises A → B → C → A, blank chat creation, and next-spawn argv. The guarded repository rekey test rejects a prior `started_at` and a stopped row. The watcher continues until process stop. |
| Models | Pass, CLI and automated | `agy models` returned tab-separated IDs and labels for Gemini 3.8/3.7/3.6 Flash, Gemini 3.1 Pro, Claude Sonnet/Opus and GPT-OSS. `agy --model gemini-3.8-flash --effort high -p /model --output-format json` resolved to `gemini-3.8-flash-high`; `--model claude-sonnet-4-6` resolved to that exact ID. The parser groups only the already proven Gemini aliases into effort choices, retains other full IDs, and keeps last good cache on refresh failure. Newly discovered full IDs pass through `--model` without an unverified `--effort` alias. |
| Runner skill | Pass, automated | Temporary homes cover `runner` and `runner-dev` install/reconciliation/removal under `~/.gemini/antigravity-cli/skills`. A marked symlink remains foreign. The global switch removal test includes the added root. Native Settings UI remains pending. |
| Quota source | Pass, CLI and automated | `agy -p /usage --output-format json` exited 0 with `status: SUCCESS`, `command.name: usage`, two provider groups and weekly/5-hour buckets. Each bucket has `remaining_fraction` and `reset_time`; the parser converts to percent **used** as `(1 - remaining_fraction) × 100` and keeps reset times. A GPUI layout test confirms all three providers' rows remain reachable by scrolling at 800×600 and 100%/150% zoom. The output reported no agent turns or tokens. No credential file or token was read or logged. |
| First-turn rendering | Pass, CLI PTY and automated | `agy-first-turn.ndjson` contains real PTY bytes from a minimal `Reply with exactly OK.` turn, including alternate-screen entry and control sequences. Account and temporary path were sanitized. `fixture_replay` checks its render snapshot. |
| Wheel input | PTY input probed; Runner UI pending | Manually injected Up (`ESC [ A`) recalled `/resume` in an agy prompt and Down (`ESC [ B`) cleared it; PageUp/PageDown produced no visible redraw. The recorded first turn enters then exits the alternate screen before its idle reply. Fixture replay confirms idle mode has no alternate screen or mouse reporting, and Runner's existing wheel encoder returns no input bytes in that mode, leaving the wheel for local viewport scrolling. No agy-specific input policy was added. An actual Runner UI wheel check remains pending. |
| Permission flags | Partial | Isolated `agy --mode accept-edits -p /usage --output-format json` and `agy --dangerously-skip-permissions -p /usage --output-format json` exited 0. The latter logged auto-approving permissions. These read-only probes do not prove write/command approval or the fixed mission Bypass behavior in native Runner. |
| Hooks with Orca present | Partial | A process-scoped `--add-dir` hook in print mode recorded PreInvocation, PostInvocation, and Stop (`fullyIdle: true`, `terminationReason: NO_TOOL_CALL`, empty error). The existing global file had `orca-status` entries for those events and PostToolUse; it was not moved. This did not exercise a tool call, forced failure, Esc interrupt, or subagent. |
| MCP copy and log cleanup | Automated only | Existing backend tests cover agy's stdio MCP JSON and session log deletion. Settings copy UI and archive/delete UI remain pending. |
| JASONPC / ConPTY | Pending | Requires the Windows machine and native UI. |

The permission prompt/override, hook failure and interrupt, Orca `PreToolUse` interaction, subagent Stop isolation, MCP copy UI, Runner relaunch, chat cleanup UI and actual Runner wheel checks require a native app session or a reliable live trigger. The global Orca hooks were left intact because other agy sessions may use them. A mocked or CLI-only result above does not count as a native Runner smoke pass.

## 2026-09-29 live UI follow-up

Jason observed session `01M3KXZE9NRQ1V3V6KYT592Z5Z` stuck on Working with agy back at its prompt after interrupting a tool-using turn. agy 1.2.12 logged cancellation at 08:44:28, and Runner's hook feed ended with `PreInvocation` and no matching `Stop`. The status watcher now treats a successfully sent standalone Esc or Ctrl+C as Interrupted if the last hook still says Working; it processes queued hooks first so a real `Stop` takes precedence. The interrupted state ignores delayed PostToolUse, PostInvocation and Stop hooks until a new PreInvocation begins. Focused watcher and PTY regressions passed. A rebuilt native Runner session still needs to confirm the visible status returns from Working after Esc and Ctrl+C.

Jason also observed that four Antigravity quota buckets make the usage popover tall. The approved sidebar variant in `design/runner.pen` (`Runner chat — weekly usage pill (747)`, node `v2ZJw`) adds a thin pill above Settings with up to three recently used, installed, enabled runtime marks in Runner's runtime order and their weekly percent used; if none was recently used, installed and enabled runtimes appear so the pill stays visible. Antigravity displays the Gemini Models weekly bucket. At the supported 200px sidebar width, the label hides so three 100% values fit; a rendered GPUI regression covers 100% and 150% zoom. Settings fills the footer except for an available update button. The pill opens the detailed popover, whose windows now use the compact one-line layout from `design/specs/706-agent-usage.pen` (`cmp/UsagePopover`, node `y655u`); Antigravity's Gemini and Claude/GPT buckets are grouped under separate headings while their percentages and reset times remain visible. Native layout and interaction still need a rebuilt Runner UI check.

Jason also chose fixed Bypass permissions for unattended missions. The 2026-09-29 `Settings — Missions` canvas frame removes the global selector, and the role pages remove their permission rows because mission launches override role flags while direct chats use the CLI's own defaults. Old `missionPermissionMode` settings are ignored and omitted on save; `SessionManager` continues to default to Bypass, including fresh and resumed mission slots. Backend permission argv tests cover convergence from old role flags. A rebuilt app still needs to confirm the Settings and role surfaces and an agy mission command run without a prompt.

## 2026-09-29 user smoke report

Jason reported that his smoke tests passed in the rebuilt Runner UI and authorized merging PR #749. This is a user-reported macOS result; no per-check capture was supplied in this record. The preceding pending UI notes describe the earlier agent-held evidence. JASONPC/ConPTY remains unverified here.

The later UI polish removes the mission-card avatar presence dot, since the card already states Working or its terminal status, and removes the weekly usage pill tooltip while renaming its label to `WEEKLY USAGE`. The root Pencil canvas and branch copy match. A rendered layout regression confirms that the longer label and three full percentage values do not overlap at the supported sidebar widths and zoom levels.

## Automated gates

All commands ran in `feat/747-antigravity-followups` using this worktree's own `target/`. Exit codes on 2026-09-28:

| Command | Exit |
| --- | ---: |
| `cargo test --locked --workspace --no-fail-fast --profile ci --timings` | 0 |
| `cargo test --locked -p runner-backend --profile ci antigravity` | 0 |
| `cargo test --locked -p runner-terminal --profile ci` | 0 |
| `cargo test --locked -p runner-app --profile ci usage_popover_scrolls_all_provider_rows_in_short_window -- --nocapture` | 0 |
| `cargo test --locked -p runner-app --profile ci app_store::skill_defaults::tests` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |

2026-09-29 follow-up checks, after the interrupted-status fix and compact usage UI:

| Command | Exit |
| --- | ---: |
| `cargo test --locked -p runner-backend --profile ci interrupted_invocation_without_stop_returns_to_ready` | 0 |
| `cargo test --locked -p runner-backend --profile ci antigravity_escape_releases_working_without_a_stop_hook` | 0 |
| `cargo test --locked -p runner-backend --profile ci recently_used_runtimes_includes_missions_and_effective_overrides` | 0 |
| `cargo test --locked -p runner-app --profile ci usage_popover_scrolls_all_provider_rows_in_short_window -- --nocapture` | 0 |
| `cargo test --locked -p runner-app --profile ci usage_pill_uses_weekly_windows_and_gemini_group` | 0 |
| `cargo test --locked -p runner-app --profile ci usage_pill_selects_recent_runtimes_but_displays_runtime_order` | 0 |
| `cargo test --locked -p runner-app --profile ci usage_pill_fits_three_full_weekly_values_at_minimum_sidebar_width -- --nocapture` | 0 |
| `cargo test --locked --workspace --no-fail-fast --profile ci --timings` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |

2026-09-29 weekly-pill visibility and fixed mission-permission follow-up, after correcting test assumptions about the now shorter Settings pane and preserving old role args on rename:

| Command | Exit |
| --- | ---: |
| `cargo test --locked -p runner-app --profile ci usage_pill_selects_recent_runtimes_but_displays_runtime_order` | 0 |
| `cargo test --locked -p runner-app --profile ci missions_pane_renders_default_crew_at_two_rem_sizes` | 0 |
| `cargo test --locked -p runner-app --profile ci content_scrollbar_starts_below_the_drag_strip_at_the_window_edge` | 0 |
| `cargo test --locked -p runner-app --profile ci surfaces::roles::tests::saving_a_rename_keeps_an_arg_that_holds_a_space` | 0 |
| `cargo test --locked -p runner-app --profile ci surfaces::roles::tests` | 0 |
| `cargo test --locked --workspace --no-fail-fast --profile ci --timings` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 |
| `cargo clippy --locked --workspace --all-targets --features updater --profile ci -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |
