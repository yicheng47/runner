# #1309 / #1310 — macOS live QA

## Candidate and scope

Tested `b3b292c3bed6bb69a1440eb66b924af0f710c240` on `fix/1309-usage-badge-polish` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1309-usage-badge-polish` on 2026-11-13. The worktree was clean before the build. Reviewer handed this exact combined candidate to QA after a clean review. No implementation or test expectations were changed by QA.

Selected the brief's Codex alignment, Antigravity Gemini value/subtitle, footer and preview spacing/overflow checks, and #1310's available update states and retained pill output action. The review added padding near the badge fit threshold and the shared Agents detail header. This is a scoped feature run, including the relevant parts of `SET-USAGE-04`, rather than a full regression pass. Other platforms and unrelated settings/session behavior were outside this run.

Environment: macOS 26.6.2 (25G83), arm64; Runner 0.19.8 development data. Native screenshots were 5120×2820 Retina pixels. Initial development settings were English, 140% app zoom, 252.62556-point sidebar, and Antigravity disabled. Backed up `ui-settings.json`, `window-state.json`, and `window-layout.json` before changes. Tested English at 100% zoom, temporarily enabling installed Antigravity 1.5.8. A real Copilot update was explicitly authorized only when already available; 1.0.110 → 1.0.111 was available and was used. No failure or reinstall was forced.

## Method and evidence

Evidence root: `/private/tmp/runner-1309-qa/`. Raw screenshots and logs remain local; cropped screenshots below contain the relevant controls. They were captured from the candidate with `@oai/sky` through `node_repl`; crops use Pillow and preserve the visible content.

Acquired `script/qa-lock acquire --timeout 540` before building. The first request timed out behind iOS mission `01M7AFV1RG8Q640SSDGXE886HE`; the next bounded request acquired the lock. Launched with `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`, using this worktree's own `target/`. No automated suite was rerun by QA. Coder/reviewer separately reported 1487 tests passed, 4 skipped, formatting and workspace Clippy with and without updater passed.

Built a temporary native-control wrapper at `/private/tmp/runner-1309-qa/Runner 1309 QA.app`, bundle ID `com.wycstudios.runner.qa1309`, copying this worktree's `target/debug/Runner` and `runner-agent-cli`. Both copies passed `cmp`. App SHA-256: `7fdecbcc9c26b3c2784051c13e1bc202a91615c10f7ee36b5703eeaf162d5aa8`. The absolute development CLI's `status --json` confirmed `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`; `status-before.json`, `status-candidate.json`, `candidate-hashes.txt`, and `make-run.log` retain the setup evidence.

## Results

All rows are macOS/native at 100% zoom unless stated otherwise.

| Check | Result | Observation and evidence under the evidence root |
| --- | --- | --- |
| #1309 Codex row alignment | Passed | Codex has a single title line vertically centered against its toggle, with no blank subtitle line. `usage-settings-evidence.png` / full `usage-agy-final.png`. |
| #1309 Gemini weekly value and both pools | Passed | Antigravity footer and preview show 10%, matching Gemini Models Week 10%; Claude and GPT models Week is 47%. Both pools remain visible in Settings and the popover. `usage-settings-evidence.png`, `popover-evidence.png`, `footer-wide-evidence.png`; full `usage-agy-final.png` and `usage-popover.png`. |
| #1309 English subtitle | Passed | Settings shows “Shows the Gemini Models weekly pool.” `usage-settings-evidence.png`. |
| #1309 footer and Sidebar preview separation | Passed | Three entries are clearly separated in both surfaces while each icon remains close to its value. The natural-size preview's 144-point usage budget fits the three 2-digit values with padding and clear Host/Settings controls. `usage-settings-evidence.png`, `footer-wide-evidence.png`. |
| #1309 narrow overflow and threshold padding | Passed | Native drag reached the 200-point minimum: 94% plus `+2`, no clipping/wrapping or control overlap. Additional controlled development-settings width fixtures, rendered by the same frozen binary, showed all three values at 238 points and 94%, 26%, `+1` at 236 points. Last value/overflow stays inside the usage area with clear padding before Host and Settings. `footer-overflow-evidence.png`, `footer-238-evidence.png`, `footer-236-evidence.png`; corresponding full screenshots retained. Width fixtures establish rendered fit, not successful native dragging at those exact thresholds. |
| #1310 running update row | Passed | Copilot's naturally available update displayed only the centered “Updating to 1.0.111…” caption alongside the toggle/chevron, with no Show output or Answer button. `copilot-running-evidence.png` / full `copilot-update-running.png`. |
| #1310 shared detail header | Passed | During update, the detail header displayed a centered caption with no output/answer button. After success it displayed “Updated to 1.0.111” without those buttons. `copilot-detail-evidence.png`, `copilot-success-evidence.png`; full `copilot-update-detail.png`, `copilot-pill-menu.png`. |
| #1310 successful update | Passed | Copilot reached 1.0.111. The success caption contained no output/answer button and was centered against its toggle; the task list independently showed successful completion. The completion task subsequently cleared. `copilot-success-evidence.png`, `pill-action-evidence.png`, full `copilot-pill-menu.png`; final Agents catalog `copilot-update-success.png`. |
| #1310 pill opens output | Blocked | The pill opened its task list and retained Show output (`pill-action-evidence.png`). The successful task cleared before the subsequent click could open the output dialog; `copilot-output-dialog.png` shows the detail page without a dialog. The daemon's existing successful-task retention is 5 seconds (`crates/runner-daemon/src/ops/runtime_tasks.rs`). No additional update remained available, so no repeat update/reinstall was forced. Rendering the button is not counted as opening output. |
| #1310 failed/broken and may-need-input states | Skipped | No natural failure or waiting-input state occurred. Per the human's scope instruction, these are covered by coder's passing `update_task_row_shows_only_caption_unless_failed_and_omits_output_buttons` test, including retained Retry/Reinstall and both list/detail caption layouts; no live pass claimed. |

## Limits and recovery

No product failure was established. The pill-opening check was not completed, and the initial handoff kept `needs-live-test` pending the operator's coverage decision. PR #1311 subsequently recorded this matrix and limit.

On 2026-11-13 at 15:02:42 UTC, Jason explicitly accepted the disclosed coverage limit through Runner message `01M7APXBFFJY05DFC84Q4AK597`: `background_tasks.rs` is unchanged by this diff and Show output was observed on screen. This acceptance waives the remaining pill-opening requirement for this PR; it does not turn the Blocked observation above into a live pass. The previously approved unit-test fallback still covers failure/broken/may-need-input states. With this accepted scope, no required live check remains outstanding and `needs-live-test` can be removed. NO REMAINING MUST-FIX QA ISSUES.

Sky background dragging was inconsistent: one foreground attempt reached minimum width, but later precise drags did not change the saved width. The documented `osascript -l JavaScript script/drag.js ... --dry-run` fallback was rejected by the execution policy (`policy forbids commands starting with osascript`), and no bypass was attempted. Exact 238/236-point rendering was therefore checked using backed-up development settings between launches, separately disclosed from native drag coverage.

A native quit attempt did not finish closing the first QA wrapper before the initial width-fixture launch. Process inventory caught two QA-owned wrapper processes; the first was terminated before accepting the 238-point evidence. Later fixture launches verified the previous QA wrapper had exited first. No assertion was accepted from the duplicated-wrapper setup. The frozen binary did not change.

## Cleanup

No test chats, missions, roles, or crews were created. Pre-existing chats/missions received no input, direct stop, archive, or configuration actions. The normal development daemon replacement/startup and final daemon shutdown preserve their saved records for normal resume. The Copilot binary remains at the authorized updated version 1.0.111; it was not rolled back.

Stopped all QA wrapper processes and the candidate development daemon (`daemon-stop.json`: `{"stopped":true}`), unregistered the wrapper with LaunchServices, and confirmed no QA wrapper or development daemon remained before releasing the lock. Restored exactly the four changed settings keys: `appZoom` 1.4, `sidebarWidth` 252.62556, `disabledAgents` containing Antigravity/Trae, and `enabledAgents` containing Claude Code. `restoration.json` confirms no remaining semantic difference from the original `ui-settings.json`. No authentication, agent defaults, global agent configuration, or installed Runner state changed.

Released our QA lock; the waiting iOS mission immediately acquired it again. Retained the wrapper, screenshots, settings backups and logs under the evidence root. This report is QA's only repository change and is left uncommitted for the coder.
