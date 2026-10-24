# #836 m9 front-page validation

Branch: `feat/836-m9-front-page`, based on `nightly-remote`. Scope and authorization: [mission brief](../impls/briefs/836-m9-front-page.md). PR 1 covers the shared target, chats, tabs, panes, drawer, sidebar, projects, host folder menu, computer-picker hint removal and accent attention dots. PR 2 follows only after PR 1 merges and covers Local only first, then the mission workspace and Start mission.

## PR 1 selected checks

Local parity is the first gate. Changed render inputs are the start form (`start_chat.rs`), chat header/tree/panes/drawer (`panes.rs`), Sidebar and its menus/projects, and the app shell's routing. Existing GPUI harnesses retain their controls, dimensions, focus/keyboard/confirmation assertions and local request ordering. Local defaults, project scope, remembered choices and persistence remain covered by existing start-chat, pane, chat-lifecycle, sidebar and window tests.

- Target dispatch: fake `protocol::Transport` request traces cover colliding public IDs on two computers, complete feature sets, dispatch refusal while disconnected, stale target/dialog generations and no replay. A held capability reply checks invalidation; stale bootstrap payloads are discarded after newer overview/status state, including when a second window opens the followed computer.
- Chat/layout/drawer: fake requests/events cover compound starts/rename/placement and partial failures, session lifecycle, split/fill/move/resize/close, stable pane IDs, concurrent host edits, serialized layout writes, activation in either result/push order, drawer membership and shell/chat close behavior. Existing geometry harnesses cover shared card bounds, gutters, focus dimming, headers and drawers across themes, locales and zoom.
- Sidebar/projects/routing: fake traces cover pin/move/reorder complete orders, rename/archive, mission lifecycle, project creation/deletion refusal/force, remote-only search/Active results, and routing/shortcut gates. Entry/return tests cover preservation of the local context and independent windows.
- Catalog/folder menu: fake responses cover fresh catalog per form, unavailable runtimes, native fork, host defaults/account display, projects/recent ordering and deduplication, home/child/parent/root, loading/empty/error/explicit Retry, typed validation, dismissal/focus, stale replies, missing features and POSIX/Windows/UNC strings. The approved Pencil frames are read through the connector without edits. No local filesystem or account lookup is permitted on remote flows.
- Picker: fake directory/follow/status tests cover 7 px attention dots appearing and clearing, unread output exclusion, online/paired eligibility, lock/offline/off release and one shared follow with visiting windows. Existing opaque-panel and dismissal harnesses retain their row/error assertions and require the removed hint to be absent.
- Avatar scope addition (human message `01M5NY7AVERWCA6N7MRNAX7N1A`): the 28 px footer avatar has a 2 px `theme::faint()` ring. Pencil `nFwHV/DcXaM` was read without edits. The existing avatar state/menu harness and footer geometry harness cover the unchanged 32 px hit area, initial/icon, hover/focus behavior, themes, widths and zoom; no existing test asserts the ring’s border width. The signed-out Settings gear branch is untouched. Rerun the app suite, both Clippy configurations, format and whitespace for this two-line addition.
- Required gates: workspace nextest with `CLAUDE_CONFIG_DIR` unset, workspace Clippy with and without updater, formatting and whitespace; direct exit codes and logs under this worktree's `target/` are recorded below when run.

New feature tests use fakes and deterministic channels/executor advancement, without sleeps or network. No crew slot launches real agents or the app, controls a development daemon, reads secrets or touches another worktree. The required pre-existing workspace suite includes its own isolated process fixtures.

## Results

The branch was rebased without conflicts onto rewritten `origin/nightly-remote` at `39d957cc` (main 0.17.2 included), using the temporary WIP commit authorized in the mission feed. That temporary commit was removed from branch history afterward; the implementation remains an uncommitted working-tree diff for review. The committed brief/plan will fold into the first approved implementation commit.

The initial automated gates passed. The first whole-diff review reported CHANGES REQUIRED (R1–R10); revised gates and re-review are recorded below. Jason’s live checks remain pending. All exit codes below come from the command itself, with matching `.exit` files beside the final logs.

| Check | Result | Exit | Log under `target/` |
| --- | --- | --- | --- |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 2,810 passed, 6 skipped | 0 | `836-m9-pr1-nextest-complete.log` |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked -p runner-app --no-fail-fast --cargo-profile ci` after the final app-only guards and scope fixes | 883 passed, 3 skipped | 0 | `836-m9-pr1-app-verified.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | 0 | `836-m9-pr1-clippy-verified.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed | 0 | `836-m9-pr1-clippy-updater-verified.log` |
| `cargo fmt --all --check` | Passed | 0 | `836-m9-pr1-fmt-verified.log` |
| `git diff --check` | Passed | 0 | `836-m9-pr1-diff-verified.log` |

The workspace run preceded the last app-only adjustments; the entire app suite was rerun afterward. Earlier focused common-surface checks passed 198 tests, exit **0** (`836-m9-focused-final-3.log`); the app plus the two model-cache diagnostic cases passed 885 tests, exit **0** (`836-m9-pr1-targeted-gate.log`). No live regression pass is claimed. The brief-listed redraw and Claude-account flakes did not fail in the final gates.

Preserved failures: the first workspace run exited **100**, with 2,807 passed and one failing folder blur harness (`target/836-m9-pr1-nextest.log`). GPUI focus events require an active fake window; activating it fixed the harness and the whole app suite passed afterward. Earlier focused runs also exposed the shared-entity read dependency, old remote-renderer assertions and the local role-speed fallback, which were fixed without weakening local geometry/control assertions.

The next workspace attempt was interrupted by SIGTERM before completion (`target/836-m9-pr1-nextest-final.log`; its direct shell exit was not captured). It had 2,718 passes, two failures in unchanged daemon model-cache process tests, two terminated CLI fixtures and 88 tests not run. An overlapping Clippy build was running; contention is a possible explanation, not established. Both model-cache cases passed in the subsequent diagnostic and complete workspace runs above. No daemon source changed. The first Clippy gate exited **101** on five app lint findings, subsequently corrected (`target/836-m9-pr1-clippy.log`). Original logs remain under `target/`.

Implementation choices: host results retain activation/reconciliation intent only; overview/status pushes own visible state. A per-tab unconfirmed layout hint serializes writes, and a differing host topology supersedes it. Successful starts retain their session ID through rename/placement errors. Retry resumes the remaining requests; launch choices freeze once started, and a saved title freezes once rename completes. Host paths remain strings, including drive roots and UNC paths. The host account is display-only. Attention follows share ownership with visiting windows, and unpaired/locked/forgotten windows release their own lease too. Sidebar state and width/collapse belong to the remote window, and old local callbacks cannot operate shared helpers after a target change. PR 1 retains only the explicitly allowed read-only mission bridge; Start mission is hidden until PR 2.

## Jason's pending live checks

On macOS and Windows with two Runners on a `remote-nightly` candidate: Direct/Role chat creation with host choices and all folder-menu states; chat stop/resume/fork/rename/archive; new/split/fill/resize/close panes and tabs; drawer shells; sidebar pin/unpin/move/reorder/rename and project create/delete; host-desktop convergence after each action; attention dots for remote questions/approval and their release on offline/lock/off; reconnect/return and older-host capability gates. Relevant changed behaviors come from `WS-CREATE-01/02/03`, `WS-PANE-01/02/03/04`, `WS-DRAWER-01/02`, `WS-SIDEBAR-01` and `WS-MENU-01`. Neither crew slot runs live checks.

Mission workspace/feed/answer/post, remote member arrangement and Start mission remain PR 2 work, with `MIS-LIFE-01/02/03` and `MIS-ASK-02` live checks pending after it lands.

## Avatar addition after the first review handoff

Jason’s final selection supersedes the earlier hold and color choice: 2 px `theme::faint()`, matching the design’s `$text-3`, on the inner 28 px circle only. The implementation is two lines in `app_shell/account_menu.rs`. The app suite passed **883 tests**, 3 skipped, exit **0** (`target/836-m9-avatar-app.log` and `.exit`), including the existing avatar state/menu and footer geometry harnesses. Formatting passed, exit **0** (`target/836-m9-avatar-fmt.log` and `.exit`). Workspace/all-targets Clippy passed both normally and with updater, each exit **0** (`target/836-m9-avatar-clippy.log` and `target/836-m9-avatar-clippy-updater.log`, with `.exit` files). Whitespace passed, exit **0** (`target/836-m9-avatar-diff.log` and `.exit`). This addition is included in the pending whole-diff review; no live check was run.

## PR 1 first-review fixes

The reviewer reported CHANGES REQUIRED with R1–R10. R8 was amended by human message `01M5P01409BR5K10CHNAPXEH1W`: omit empty Projects/Recent sections, allow Back and Escape during loading, and enable Use folder only after a successful current listing, including an empty folder. The revised `hCh3b`, `b9rzdK`, `J0QvG` and `kYEBx` frame contexts were read through Pencil without edits.

- R1: target identity survives a missing overview; the palette clears its remote items rather than enumerating local data. Stale local selections and CLI install/uninstall dispatch are refused remotely. The fake test withholds the overview and asserts no local request or installation.
- R2–R3: compound starts retain the expected destination tab and pane/drawer until pushed membership confirms it. An intermediate host-created tab does not consume activation, and removed active tabs are reconciled. Chat fill uses the same concurrent-host-edit completion logic as generic actions. Tests cover chat fill, split shell and drawer sequences, stale results, focus/drawer selection, and a subsequent permitted edit.
- R4–R5: resume and pane cycling read the target’s layout and lifecycle state. Fake tests preserve a local Start or non-chat route and local split while operating only remote sessions.
- R6: remote sidebars omit the local updater, avatar and account menu; stale account/settings/sign-out callbacks are refused. Fake render/dispatch tests cover a signed-in local account and an available local update. Existing local avatar tests retain the explicitly approved 2 px faint ring.
- R7–R8: New project uses a 320 px menu (scaled by zoom) anchored to its small plus control; working-directory menus retain field width. Pending/failed listing guards apply centrally to pointer and keyboard selection/navigation, while Back cancels the pending generation. Tests exercise real entry-point geometry at two zooms, disabled selection, explicit Retry, successful empty folders and stale replies.
- R9: shell splits capture their source’s host cwd before focusing the empty leaf; filling an empty shell pane uses the owning populated sibling. Fake traces cover Windows drive and UNC paths distinct from the project default.
- R10: single-pane status tooltips discard synthetic ordering timestamps for display, including Sidebar Active rows, while preserving attention/unread text and the local elapsed-time path.

Focused regression tests passed **11/11**, exit **0** (`target/836-m9-review-focused4.log` and `.exit`). Earlier focused attempts are preserved: compilation exits **101** in `836-m9-review-races` and `836-m9-review-focused2`, then exit **100** in `836-m9-review-focused3` from two harness sequencing mistakes (nested root borrowing and dispatching before the simulated host edit). The corrected harness dispatches outside a borrowed root and advances events in the intended order. No production failure was hidden or test assertion weakened.

The branch is now rebased onto `origin/nightly-remote` at `cbb877c4`, with the existing brief/plan commit replayed as `359843dd`. The uncommitted diff was saved under `target/`, reversed, and reapplied after rebase, without an implementation commit or stash. The sole three-way conflict was the spec’s Runner-as-client app/signed-out-picker bullets: the umbrella’s text was retained verbatim, including Local only and attention dots; the Remote actions edits remain. This resolution is part of the whole-diff re-review.

Local only is documented as PR 2’s first commit and has no implementation in PR 1. After reviewer approval, Claude Code must review the whole working tree and explicitly say “go” before any implementation commit, push or PR (human message `01M5P060G8EQ0NATADTJRECNF8`). Live checks remain Jason’s.

### Revised automated gates

All revised gates passed. The workspace run covers the final production code; afterward the app suite was rerun for the stronger saved-Start/non-chat navigation and field-width assertions, and again after the test-only Clippy fixes. The brief-listed flakes did not fail. The first revised Clippy attempt exited **101** for three redundant test closures (`target/836-m9-review-clippy.log` and `.exit`); those were replaced with the named helper, without lint allowances or skipped checks.

| Check | Result | Exit | Log under `target/` |
| --- | --- | --- | --- |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 2,820 passed, 6 skipped | 0 | `836-m9-review-nextest.log` |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked -p runner-app --no-fail-fast --cargo-profile ci` | 893 passed, 3 skipped | 0 | `836-m9-review-final-app.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | 0 | `836-m9-review-final-clippy.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed | 0 | `836-m9-review-final-clippy-updater.log` |
| `cargo fmt --all --check` | Passed | 0 | `836-m9-review-final-fmt.log` |
| `git diff --check` | Passed | 0 | `836-m9-review-final-diff.log` |

The complete working-tree diff, including the rebase’s spec resolution and brief/plan changes, is awaiting reviewer re-review. No implementation commit, push or PR exists, and no live check ran.

## PR 1 second-review fixes selected checks

The second whole-diff review confirmed the original R1–R10 reproductions were addressed and reported CHANGES REQUIRED for R11–R12. R11 changes last-member chat/shell archival and batch cleanup: the host already removes the empty tab, so the client must not archive it a second time; an initially empty tab still needs explicit archival. R12 changes activation ownership across navigation: selecting another remote item advances a window presentation generation, cancels installed activation and stale start forms, and prevents older results from reinstalling activation. Pending request/layout state still settles, and stale errors or partial shell stages cannot move into the newly selected workspace. The unused empty-section placeholder keys are removed in both languages.

Selected fake checks: an opt-in host model removes a tab when its last pane/drawer member is archived/closed and rejects operations on the deleted ID. App tests exercise last chat, last shell, Archive All, batch archival with another surviving tab, and explicit initially empty-tab archival, asserting no redundant request/error and disappearance only after a push. Deterministic executor tests cover chat fill, shell placement/drawer, resume, fork and new-tab activation with navigation before the result and between result and confirming push, preserving the chosen tab or mission. A partial shell-placement failure after navigation must settle pending state without an error on the new tab or reuse of the old shell. Rerun the focused cases, the full workspace gate, both Clippy configurations, formatting and whitespace. Local rendering and all live checks remain unchanged; Jason owns the latter.

The focused R11/R12 run passed **5/5**, exit **0** (`target/836-m9-r11-r12-focused3.log` and `.exit`), including the existing placement activation and partial-retry tests. Earlier focused results are preserved: the first run exited **101** because the fake model lacked an explicit unknown-pane arm; the second exited **100** after detecting that a cancelled draft hint could still reopen its drawer. The fix retains the prior focused pane, drawer selection/height/open state in cancelled or stale hints, preserving reconciliation IDs and pending-write serialization. Both before-result and before-push navigation cases now pass for chat fill, shell placement, drawer shells, resume, fork and new tabs. Stale partial shell failures settle without showing an error on the newly selected item or reusing the successful shell there.

R11’s fake host deliberately rejects tab/layout operations after the last member removes the tab, so it cannot mask the original redundant archive. The independent reviewer’s existing in-process daemon contract checks also passed **3/3**, exit **0** (`target/836-m9-reviewer-close-contract.log` and `.exit`). No daemon source changed. Initially empty tabs retain explicit tab archive; affected nonempty tabs receive only the remaining layout cleanup, even in a batch that deletes another tab.

The navigation fix also invalidates a start form whose catalog is still pending, removes its pending request when the stale reply settles, and permits a fresh form for the newly selected item. This follows the same route-generation contract; its deterministic fake test selects a mission before catalog completion, verifies no stale form appears, then opens a fresh form successfully. Final gates run after this addition.

An intermediate workspace run passed **2,823 tests**, 6 skipped, exit **0** (`target/836-m9-r11-r12-nextest.log` and `.exit`), before the catalog-navigation follow-up. The next run exited **100**, with 2,822 passes, two failures and 6 skips (`target/836-m9-r11-r12-final-nextest.log` and `.exit`). The new catalog test reached its final request-count assertion but counted the entry-time catalog read; it now asserts exactly two additional form requests relative to the captured baseline. The other failure was the unchanged daemon test `spawn_failure_timeout_and_cache_write_failure_are_contained`, whose in-memory catalog assertion returned `None` instead of `Some(memory)` at `models/tests.rs:578`; it passed in the preceding workspace run, and no daemon files changed. These results are retained rather than attributed to a proven root cause.

The corrected focused run passed **6/6**, exit **0** (`target/836-m9-r11-r12-verified-focused.log` and `.exit`), including the catalog-navigation case. The final full gates follow this exact source, without further implementation changes.

### R11/R12 final gates

All gates passed on the final implementation and test source. The daemon model-cache case passed unchanged in this run; its earlier failure remains recorded above without a root-cause claim. The brief-listed redraw and Claude-account flakes did not fail.

| Check | Result | Exit | Log under `target/` |
| --- | --- | --- | --- |
| Focused R11/R12, catalog cancellation, existing placement/retry cases | 6 passed | 0 | `836-m9-r11-r12-verified-focused.log` |
| `env -u CLAUDE_CONFIG_DIR cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 2,824 passed, 6 skipped | 0 | `836-m9-r11-r12-verified-nextest.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | 0 | `836-m9-r11-r12-verified-clippy.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed | 0 | `836-m9-r11-r12-verified-clippy-updater.log` |
| `cargo fmt --all --check` | Passed | 0 | `836-m9-r11-r12-verified-fmt.log` |
| `git diff --check` | Passed | 0 | `836-m9-r11-r12-verified-diff.log` |

Each exit code has a matching `.exit` file written from the command’s result. The entire working-tree diff is awaiting the reviewer’s third pass. No implementation commit, push, PR or live check has occurred. Claude Code’s separate whole-tree review and explicit “go” still follow a clean reviewer verdict.

## PR 1 clean reviewer verdict and Claude Code gate

The reviewer’s third whole-diff verdict is **NO REMAINING MUST-FIX ISSUES** (Runner message `01M5P2MX5MZ32ZW5PYY31XBH1F`, 2026-10-24). R1–R12 are resolved; no PR 2 implementation or daemon/protocol changes were added. The reviewer independently passed **58/58** focused app tests, exit **0**, and whitespace, exit **0**; evidence is `target/836-m9-reviewer-third-focused.log` and `.exit`, plus `target/836-m9-reviewer-third-full.patch/.json` and the incremental delta snapshot. The final coder gates above remain exit **0**. The reviewer verified all 60 files, including the retained umbrella spec text.

The verdict covers working-tree diff `b11029420a230d956a0b61b4e7e407c74e29b4e4d6a96325346c27aae37e1526` against HEAD `359843dd2cc5c424486ca7e919f55e68b6a17d68`; the complete feature diff against `origin/nightly-remote` at `cbb877c4b992b1e600f186ba21c7f7c9df40e346`, including the existing brief/plan commit, is `c2b255a8ac71fa15aa41d971bf1c96d79ab43abb3b0a623f2b5cc53a771a5b4b`. This verdict-record addition is the sole subsequent change. Claude Code must review the complete feature diff and explicitly say “go” before any implementation commit, push or PR. His findings return through reviewer review. The final commit history’s pre-push checks follow that approval. Live checks remain Jason’s; no live pass is claimed.

## Claude Code whole-diff review follow-up C1–C3

Claude Code’s review (Runner message `01M5P38ZZ3V0FQ4ECEJB4CKYBF`, 2026-10-24) confirmed local parity and requested three small fixes before commit: C1 removes whole-slice cloning from tab label helpers and projects only the local layout’s member sessions; C2 resolves remote rename placeholders from the window’s remote layout; C3 sizes the inline rename field from the sidebar’s target-scoped Active setting. Existing local label/title assertions are retained with explicit display projections. A focused fake test checks the actual rename field’s derived host placeholder both without a local tab and with colliding local tab/session IDs and a different local label.

As instructed, validate the focused test, complete app suite, both workspace/all-target Clippy configurations, formatting and whitespace, then hand the delta to the reviewer. Claude Code’s message is the conditional “go” once the reviewer confirms this delta has no must-fix; no further pre-PR human review round is required. Commit/push/PR remain gated on that delta verdict. No live checks run.

### C1–C3 validation results

The requested app suite passed **898 tests**, 3 skipped, exit **0** (`target/836-m9-c1-c3-verified-app.log` and `.exit`). The focused local-title and remote-placeholder cases passed **3/3**, exit **0** (`target/836-m9-c1-c3-verified-focused.log` and `.exit`). Workspace/all-target Clippy normally and with updater each passed, exit **0** (`target/836-m9-c1-c3-verified-clippy.log`, `target/836-m9-c1-c3-verified-clippy-updater.log` and their `.exit` files). Formatting and whitespace each passed, exit **0** (`target/836-m9-c1-c3-verified-fmt.log`, `target/836-m9-c1-c3-verified-diff.log` and `.exit` files). Validation scope follows Claude Code’s explicit follow-up instruction; the earlier full workspace result remains recorded above.

The first focused run exited **100** (`target/836-m9-c1-c3-focused.log` and `.exit`) because its local collision fixture used a single-pane tab, whose manual tab name is intentionally discarded by `PaneLayout::from_node_row`. The fixture now uses grouped local and remote tabs so the conflicting manual local label is retained and the derived remote placeholder is asserted through the real sidebar menu handler. Existing label expectations remain unchanged. The C1–C3 six-file delta is awaiting the reviewer; no implementation commit, push, PR or live check has occurred.

### C1–C3 clean verdict and final-history gate

The reviewer confirmed **NO REMAINING MUST-FIX ISSUES** for the entire six-file C1–C3 delta (Runner message `01M5P3NEV4CP8HJV9TYNW4WPYD`, 2026-10-24), retaining the previous clean whole-diff verdict. Independent focused checks passed **5/5**, exit **0**, and whitespace passed, exit **0** (`target/836-m9-reviewer-c1-c3-{focused,diff}.log` and `.exit`). The reviewer verified the coder’s app suite and both Clippy/fmt/diff exits were zero and the rest of the implementation was unchanged.

This satisfies Claude Code’s conditional “go” in `01M5P38ZZ3V0FQ4ECEJB4CKYBF`. PR 1’s integrated target/read-model and consumer changes are prepared as one atomic implementation commit containing the brief, plan, guides and fake tests; there is no standalone brief commit. The final commit’s content/build checks and reviewer history check precede the authorized push. Claude Code reviews the PR before merging; the crew does not merge or begin PR 2 before that merge. Live checks remain Jason’s.
