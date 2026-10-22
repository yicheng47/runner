# #836 m7a — Account validation

Scope: app-only relocation of Phase 1 enrollment to Settings → Account, the sidebar avatar menu, and Remote access's route choice and relay pages. No app is launched and no live service is contacted in this mission. New UI tests use local fixtures without network calls or sleeps. Local validation ran on macOS arm64 (Darwin 25.6.0). Live checks remain pending for Jason.

## Changed behaviors and selected checks

| Behavior | Check and reason | Result |
| --- | --- | --- |
| Account navigation and every enrollment/stop state | Settings route/nav tests plus UI fixtures for Off, Enrolling, Confirming, Connecting, Connected, Reconnecting, Paused and all five stop reasons in English/Chinese and dark/light themes; relocation must preserve every state. | Passed locally |
| Sign in, cancel, retry, sign out and Reset | Capture existing client requests in a fake transport; exercise sign-out confirmation/cancel and reset-to-route-choice without contacting a service. | Passed locally |
| Avatar and its menu | Signed-in initial, signed-out icon, both sets of items, navigation and sign-out confirmation; menu opens above the footer without moving usage. | Passed locally |
| Route choice and relay pages | State fixtures assert route choice only while signed out, waiting in the initiating pane, disabled Tailscale, and relay controls/devices only while signed in. | Passed locally |
| PIN and paired devices | Retain two-step PIN, lock warning, confirmation, Connected now and device geometry fixtures; these controls move into the relay card. | Passed locally |
| Sidebar footer | Existing layout matrix at min/default/max width, 60/100/150/200% zoom, dark/light; shared macOS/Windows footer must retain usage and avatar geometry. | Passed locally |
| Localization | Catalog key/placeholder parity plus English/Chinese render fixtures cover all new copy. | Passed locally |

The relevant maintained regression is SET-USAGE-02's footer placement/opening behavior, covered here by automated geometry/action fixtures; its live account and native-app steps do not run in this mission. Runtime, terminal input/resize and session lifecycle behavior are unchanged and are outside this feature's selected live scope.

## Required gates

Each command writes a log under this worktree's ignored `target/`; exit codes are captured directly by the invoking command.

| Command | Log | Exit code |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/m7a-review-fix-nextest.log` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7a-review-fix-clippy.log` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/m7a-review-fix-clippy-updater.log` | 0 |
| `cargo fmt --all --check` | `target/m7a-review-fix-fmt.log` | 0 |
| `git diff --check` | `target/m7a-review-fix-diff-check.log` | 0 |

The final workspace gate after review fixes passed 2655/2655 tests (six configured skips). The original selected app checks passed 31/31 with exit 0 in `target/m7a-focused-confirmed.log`. The full workspace run is the required automated gate, not a full live regression pass.

## Corrections during validation

- The initial compile check exited 101 for three compile errors in the new UI; they were corrected and the subsequent check exited 0.
- The first app run (`target/m7a-app-tests.log`, exit 100; 807/808 passed) and initial focused run (`target/m7a-focused-tests.log`, exit 100; 30/31 passed) exposed a fixture ordering issue: the startup status snapshot overwrote the trust-layout fixture's seeded signed-in state. The fixture now drains startup work before seeding the status. The trust-layout test then passed in both languages and themes.
- The first Clippy run (`target/m7a-clippy-initial.log`, exit 101) required `then_some` instead of a lazy closure; both final Clippy gates passed.
- Tightening the actual Sign in button checks initially kept a direct sign-in call in the test, producing two captured calls (`target/m7a-nextest-action-fixture-failure.log`, exit 100; 2653/2654 passed). After removing it, the route-button test clicked the center of a full-width wrapper rather than its left-aligned button (`target/m7a-focused-final.log`, exit 100). The wrapper now fits the button; the confirmed focused run passed 31/31 with exit 0. The earlier workspace run, before those stronger assertions, passed 2654/2654 with exit 0 in `target/m7a-nextest-before-action-assertions.log`.

## Spec details chosen

The spec records the shared signed-in predicate and waiting-card routing; Sign in… in the avatar menu opens Account; Account omits a sign-in date because the protocol only exposes the current relay connection time; and m7a uses the account heading over the existing paired-device table until m7c. None changes the enrollment, pause, retry, sign-out or reset protocol.

## Live checks pending for Jason

Run on a `remote-nightly` candidate on macOS and Windows: sign in against the hosted service from Account and from Remote access's relay choice; inspect the avatar and menu signed in and out; confirm/cancel sign out; pause/resume; confirm Reset remote access returns to route choice. Neither coder nor reviewer runs the app.

## Review and follow-ups

The first working-tree review found three P2 issues, all addressed below. On 2026-10-22 the reviewer re-reviewed the whole diff and reported `NO REMAINING MUST-FIX ISSUES`. Its 14/14 selected tests and `git diff --check` passed with direct exit 0 (`target/m7a-reviewer-fixed-focused.log`). The existing brief/plan/spec commit was folded into the first implementation commit after this clean review.

## Review follow-ups

| Finding | Fix and selected regression | Result |
| --- | --- | --- |
| Avatar trigger rectangle was replaced for popup placement, so a second click reopened the menu. | Keep the actual trigger bounds for outside-click handling and pass the left position separately through the existing popup positioning function. The avatar-menu fixture now checks open → same-avatar click → closed in both sign-in states. | Regression reproduced before the fix, exit 100 in `target/m7a-review-toggle-before-fix.log`; final workspace gate passed, exit 0. |
| Remote access's enrollment page omitted the fingerprint-comparison reminder. | Share the reminder between Account and Remote access; the status matrix asserts it in each pane during enrollment in both languages/themes. | Final workspace gate passed, exit 0. |
| Account claimed the relay was on while paused or reconnecting. | Use neutral configured-route copy for Paused, Connecting and Reconnecting. A bilingual text/state test keeps the connected-state availability claim exclusive to Connected. | Final workspace gate passed, exit 0. |
| Live QA F1: signed-out avatars were blank circles; GPUI SVGs require their own text color instead of inheriting the wrapper's. The moved footer lock and approval reminder had the same omission. | Set the avatar SVG to `theme::muted()` and the lock/reminder SVGs to `theme::faint()`; the other SVGs added by this PR already set colors. Assert the actual avatar SVG builder's style at all three sizes in light/dark, since the existing geometry fixtures cannot observe SVG paint color. | Regression failed before the fix, exit 100 (`target/m7a-f1-regression-before.log`); final workspace run passed 2656/2656, six skips. All five required gates exited 0 (`target/m7a-f1-{nextest,clippy,clippy-updater,fmt,diff-check}.{log,exit}`). Initial macOS live QA passed 9/13 rows, with four F1 failures; targeted icon live re-check is pending and on hold per Jason. Windows live QA was not run. |

All five required gates after these fixes passed with exit 0, logged as `target/m7a-review-fix-{nextest,clippy,clippy-updater,fmt,diff-check}.log`. The workspace run passed 2655/2655 tests with six configured skips. A process-list check found no executable left running from this worktree’s `target/`. No app launch or live service calls occurred. The reviewer cleared all three fixes before commits began.

## Intermediate commit validation

The planned Account, avatar-menu and Remote access commits were each built and tested in their actual intermediate source state before committing. The final source was checked against the reviewed source after assembling the commits.

| Step | Commit | App tests | Log / exit |
| --- | --- | --- | --- |
| 1 | `dbcc53ba feat(ui): add Account settings for relay sign-in` | 809/809 passed; three configured skips | `target/m7a-commit-1-tests.log` / 0 |
| 2 | `31c23c31 feat(ui): add the sidebar account avatar menu` | 811/811 passed; three configured skips | `target/m7a-commit-2-tests.log` / 0 |
| 3 | `3990e625 feat(ui): add relay route selection and access settings` | 812/812 passed; three configured skips | `target/m7a-commit-3-tests.log` / 0 |

The fourth commit contains only the guide and this validation record; formatting and whitespace gates passed with exit 0. CI on macOS and Windows is checked after the PR opens. The live checks listed above remain pending for Jason.
