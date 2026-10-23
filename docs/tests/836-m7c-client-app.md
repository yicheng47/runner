# #836 m7c — client app validation

Candidate: `feat/836-m7c-client-app`, based on `nightly-remote` m7b `12b9b80a`. Coder and reviewer do not launch Runner or runnerd, contact a relay/service, or operate existing sessions. All automated remote responses and events are fake and use no network or sleeps. Native rendering is exercised only in GPUI’s test platform. This is feature validation, not a full regression pass.

## Selected behavior and checks

| Changed behavior | Check and reason | Result |
| --- | --- | --- |
| Account computers and incoming trust appear together | App test covers self first, all four access labels, per-host-key/name matching, UTF-8 truncation and an unmatched device; these establish the list without inferring identity from a device fingerprint. | Passed in the workspace gate |
| Pair dialog accepts exactly one submit at a time | Fake protocol counts requests while a second submit is attempted; wrong PIN, lock, no PIN and transport refusal render in English and Chinese. Existing PIN input tests cover masked reads, ASCII digits, six-digit paste and clipboard protection. | Passed in the workspace gate |
| Picker opens the chosen computer in this or another window | GPUI test opens another window and confirms the first remains local; picker rows include the local computer and remote directory entries. | Passed in the workspace gate |
| Remote overview and status pushes remain separate from local state | Fake events update the remote model; a shared follow survives one consumer leaving and stops after the last leaves. | Passed in the workspace gate |
| Remote terminal uses m7b’s terminal address | Fake attachment/input/resize/take-control checks assert the remote computer and session, minimum size, fresh-snapshot attach without taking again, cancellation and discarded disconnected input. | Passed in the workspace gate |
| Remote window retains its local layout and displays connection/control states | GPUI checks cover remote sidebar, disabled creation, taken-back terminal, reconnecting, unreachable and revoked; returning releases the follow and terminal and preserves local route/layout. Carbon/light themes and English/Chinese are covered. | Passed in the workspace gate |
| Suspended local missions resume on return | Fake-backend tests cover a loaded mission’s active state, terminal attachment/interactivity, selected tab, layout, closed tabs and draft; replayed missed events/metadata and later pushes establish recovery; same-mission notification navigation and an initial load completing while remote cover both early-return paths. | Passed in the workspace gate |
| Existing host controls and Settings remain intact | Workspace tests include the m6 control-card/PIN/connected-device tests and existing Settings render checks. | Passed in the workspace gate |

Related maintained live cases selected for Jason: `TERM-INPUT-01` for typing/text paste through the reused terminal surface, and `WS-CHROME-04` for sidebar/titlebar placement on both platforms. Their remote variants are part of the pending m7c live checks below. Unchanged IME/runtime/auth/session lifecycle cases are outside this feature’s live scope. The workspace test gate still runs every automated test as required by the mission.

## Commands and evidence

Each command writes its own log under this worktree’s `target/`. Exit codes are captured directly from the command, before any log inspection. Initial implementation attempts are retained, including test-compilation corrections.

| Command | Log | Exit/result |
| --- | --- | --- |
| `cargo check --locked -p runner-app --all-targets --profile ci` | `target/m7c-check.log` | 101: initial UI integration compile errors; corrected |
| Same check after integration fixes | `target/m7c-check-2.log`, `target/m7c-check-3.log` | 0, 0 |
| `cargo nextest run --locked -p runner-app --cargo-profile ci -E 'test(surfaces::remote_client)'` | `target/m7c-targeted-tests.log`, `target/m7c-targeted-tests-2.log` | 101, 101: new test-context API corrections |
| First behavioral targeted run | `target/m7c-targeted-tests-3.log` | 100: missing test-only paths and a palette wake/redraw loop; the spinning test was stopped with SIGTERM, then corrected |
| Targeted tests after corrections | `target/m7c-targeted-tests-4.log` | 0: all 6 passed |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/m7c-workspace-tests.log`, `target/m7c-workspace-tests-final.log`, `target/m7c-workspace-tests-reviewed.log` | 0, 0, 0: 2,716 passed and 6 skipped in each run |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7c-clippy.log`, `target/m7c-clippy-handoff.log` | 101 initially (unused helper and collapsible conditional); corrected, then 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/m7c-clippy-updater-handoff.log` | 0 |
| `cargo fmt --all --check` | `target/m7c-fmt-handoff.log` | 0 |
| `git diff --check` | `target/m7c-diff-handoff.log` | 0 |
| `cargo nextest run --locked -p runner-app --no-fail-fast --cargo-profile ci` after the final app-only parity fixes | `target/m7c-app-handoff.log` | 0: 819 passed and 3 skipped |

The pairing tests also assert Forget-before-Pair ordering on an explicit pair-again submit and that canceling a submitted attempt prevents its completion from opening a window. Model tests keep revoked state through m7b’s final disconnected event and clear the directory on sign-out. A final app-only rerun covers Windows copy-propagation parity and keeping remote Take control off the UI thread; Windows native Ctrl+C remains part of Jason’s live terminal check.

## Working-tree review follow-ups

Reviewer message `01M5KCB7SR8YMKJSEE0TCBYX90` found three behavior gaps and an obsolete spec paragraph. The follow-up adds these checks before requesting a whole-diff re-review:

| Finding | Fix and check | Result |
| --- | --- | --- |
| IPC recovery could leave a frozen remote mirror | A daemon recovery generation invalidates affected window mirrors and in-flight attachments; replacements use the latest measured viewport with `take_control=false`. A fake frame stream closes, recovery events fire, a distinct snapshot arrives, and subsequent output is read from the new stream. A failed replacement renders an error and selecting the tab retries without taking control. | Passed in the targeted run |
| Pair again could display a stale fingerprint | Resolve the current directory row at dialog entry; replace the PIN field and dialog generation on an identity change; check the directory again before the one PIN request. The fake test covers active identity A → directory B → stale card callback displaying B, then entry clearing for C and aborting a queued attempt after D. | Passed in the targeted run |
| Hidden local routes affected notifications | Remote windows do not count as foreground local content; a local notification switches the selected window back to local mode before routing. The Unix GPUI fake test verifies local delivery/retention while remote, chat click routing and mission click routing; existing notification tests remain selected. Windows native delivery remains pending. | Passed in the targeted run |
| Old noncontroller spec allowed typing/scaling | Human clarification `01M5KCA07WHNCXFPY5GPJFFACZ` selects signed-off frame 3.5: keep the terminal visible with typing/size pushes disabled and Take control. The Terminal size paragraph and its checks now say that; host behavior is unchanged. | Corrected |

`target/m7c-review-targeted.log` exited 100 because the new notification fixture used a non-ULID node ID; corrected. `target/m7c-review-targeted-2.log` exited 0 with 26 selected tests passed. These tests use fake protocol responses, frame channels and events, with no network or sleeps. The required workspace rerun in `target/m7c-review-workspace.log` exited 0: 2,719 passed, 6 skipped. The final targeted run in `target/m7c-review-targeted-final.log` exited 0: 26 passed, covering notification routing after removing the intermediate activation of a hidden local tab. Both Clippy variants, format and diff checks also exited 0; their final logs are `target/m7c-review-{clippy-final,clippy-updater-final,fmt-final,diff-final}.log`.

Reviewer re-review `01M5KD8NR4KATYEVX8EJJTGH65` found one remaining issue: leaving remote mode did not reactivate the suspended local mission. Ownership synchronization now resumes its activity and terminal attachments even when secondary ownership is unchanged, refreshes metadata and replays missed events while retaining its layout, selected tab, closed tabs and draft. Same-mission navigation reports subjects and synchronizes ownership before its early return, without marking the previously hidden slot as viewed; notification routing then selects the requested slot. An initial mission load may finish while suspended, but attaches/focuses terminals only when local again. Three fake-backend regression tests cover the loaded round trip, same-mission notification, and load completion while remote. `target/m7c-mission-resume-targeted.log` exited 101 on initial fixture/API mistakes; corrected. `target/m7c-mission-resume-targeted-2.log` exited 0: 29 selected tests passed, with no network or sleeps. The required workspace gate in `target/m7c-mission-resume-workspace.log` exited 0: 2,722 passed and 6 skipped. Both Clippy variants, format and diff checks also exited 0 in `target/m7c-mission-resume-{clippy,clippy-updater,fmt,diff}.log`. `target/m7c-mission-resume-workspace-final.log` also exited 0: 2,722 passed and 6 skipped, covering the notification assertion that the old hidden slot is not marked viewed. The final targeted run in `target/m7c-mission-resume-targeted-final.log` exited 0 with 29 passed; its notification test also proves the clicked slot is marked viewed. The final Clippy variants, format and diff logs are `target/m7c-mission-resume-{clippy-final,clippy-updater-final,fmt-final,diff-final}.log`, all exit 0.

## Live checks pending for Jason

Run on a `remote-nightly` build on macOS and Windows, first with two Runners against a local `runner_server`, then the hosted service. No live checks ran in this mission.

1. Pair from the computers list and the picker; try a wrong PIN, locked entry and a host with no PIN. Verify checking prevents duplicate submits and a changed fingerprint is shown before pairing again.
2. Open the other computer in the same window, with Shift-click and with the window button. Check sidebar placement, dimmed creation actions, remote-only projects/tabs/missions and the local attention badge. Return and verify the original local tabs/panes and mission selection are intact, its terminals accept input, and its feed updates; restart and verify windows open on this computer.
3. Open a terminal, type/paste text and resize. Verify the host sees the controller’s size. Take it back on the host, then Take control on the client. Leave the remote computer and verify its attachment/follow is released while agents keep running and the size stays.
4. Drop the route while typing; observe reconnecting and the unreachable card, reconnect to fresh output and verify disconnected input is absent. Revoke on the host and verify the revoked card and explicit pairing again.
5. On the host verify m6’s control card, new-pair notification and Connected now behavior (rendered as Connected to this computer in the new list).

## Review and CI

Reviewer message `01M5KDTWSFSHKN49C4P1CDBGH8` reports **NO REMAINING MUST-FIX ISSUES** on the whole working-tree diff, after the IPC recovery, pairing identity, notification visibility and local mission restoration fixes. The final history is one integrated `feat(ui)` commit, folding in the branch’s initial brief, plan, README and log commit, so the combined UI and its tests build together; its parent is current `origin/nightly-remote` at `12b9b80a`. macOS/Windows CI is tracked on the PR after opening. Commit, push and open the PR against `nightly-remote` immediately after clean review, then drive CI green, per human message `01M5KC458S2K3FYEBERAY1HPK8`; there is no extra pre-push handoff. Jason owns the live checks and merge. No nightly/release or branch/worktree cleanup is authorized.


## Claude Code review follow-up — 2026-10-23

Scope: fixes 1–5 from the brief’s Review follow-up section, on `feat/836-m7c-client-app` at `114d5cec` plus the working-tree diff. App only; no daemon, protocol, CLI, dependency or migration changes. The coder does not launch the app. The reviewer decides whether a single-machine development-app check is warranted; two-computer and remote-service checks remain Jason’s. No slot starts or contacts `runner_server` or the hosted service.

Selected checks before execution:

| Changed behavior | Selected check and reason | Result |
| --- | --- | --- |
| Ambiguous incoming device labels | Fake trust/directory rows cover identical names, collisions after UTF-8 truncation, and a local/remote name collision; neither account row receives incoming access or a Revoke target, and each device retains a separate unmatched row. Both list and picker share this matching rule. | Passed in the workspace gate |
| Mission pane sizing and empty shells | Fake overview mission members cover 0, 1, 2, 3, 4, 7 and 24 members; widths preserve order and equal shares, and GPUI rendering omits an empty shell strip. | Passed in the workspace gate |
| Request independence | Channel gates hold a fake overview and then the directory request while another computer’s visit, retry and Forget complete; no sleeps, network or PIN retry. Existing follow-lease and single-PIN tests cover ownership and pairing regressions. | Passed in the workspace gate |
| Local Forget | Fake Forget followed by late connection/overview and daemon recovery events keeps an accurate forgotten state, releases the mirror, and opens plain pairing copy in both locales. | Passed in the workspace gate |
| Split divider orientation | A fake two-pane column overview renders headers at the same horizontal origin and width, proving the second pane has no spurious left border. | Passed in the workspace gate |
| Take control feedback and current-computer selection | A fake refusal renders a pane error, a successful retry clears it, and choosing the current healthy computer preserves its mirror, attachments and follow count, in both locales. | Passed in the workspace gate |

The existing `TERM-INPUT-01` and `WS-CHROME-04` remote variants stay with Jason; this round adds no local-runtime, IME, authentication or general lifecycle live scope. Run the targeted remote-client tests, the required workspace nextest gate, both workspace Clippy variants, format and diff checks. Each writes a separate `target/m7c-followup-*.log`, with the exit code captured directly before reading the log. This is feature validation, not a full regression pass.


Follow-up gate evidence:

| Command | Log | Exit/result |
| --- | --- | --- |
| Initial targeted invocation before this machine had a `target/` directory | No log could be opened | 1: shell redirection failed before Cargo started; created the worktree’s own `target/` |
| `cargo nextest run --locked -p runner-app --no-fail-fast --cargo-profile ci -E 'test(surfaces::remote_client)'` | `target/m7c-followup-targeted.log` | 101: a test selector returned SharedString instead of String; corrected |
| Same targeted command after the selector correction | `target/m7c-followup-targeted-2.log` | 0: 15 passed |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/m7c-followup-workspace.log` | 0: 2,728 passed, 6 skipped; includes the failed-connection reselection path |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7c-followup-clippy.log` | 101: needless borrow after sharing the picker’s matching rule; corrected |
| Final targeted remote-client tests after lint/copy corrections | `target/m7c-followup-targeted-final.log` | 0: 15 passed |
| Standard Clippy after the borrow correction | `target/m7c-followup-clippy-2.log` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/m7c-followup-clippy-updater.log` | 0 |
| `cargo fmt --all --check` | `target/m7c-followup-fmt.log` | 0 |
| `git diff --check` | `target/m7c-followup-diff.log` | 0 |

The first follow-up candidate added six tests. Request-isolation tests gate synchronous fake responses with channels and bounded receive assertions, with no sleeps or network. A local Forget remains distinct through late pushes and daemon recovery; the same-computer check covers both preserving a healthy mirror and reconnecting after failure. Both locales render the new pane-error and locally-forgotten states. Reviewer verdict and any selected live check remain pending until the explicit working-tree handoff.


### First follow-up review

Reviewer message `01M5KHRX5PH1KX9R1J0P83YGDC` found two must-fix gaps: a successful PIN submission in an existing remote window could hit the healthy-selection shortcut before restoring its shared follow, and a failed Take control message could outlive the mirror it belonged to. Before rerunning validation, the fixes add a pairing epoch per computer to replace that computer’s window mirrors and explicitly restore any active shared follow after pairing; the control error now belongs to its pane’s attachment lifetime. Fake checks extend through Forget and Revoke → successful PIN submit → new follow, Connected, fresh mirror and later overview/status delivery, plus failed Take control → leave tab → reopen with control → no stale error.

The reviewer independently ran the first candidate’s 15 remote-client tests (exit 0; `target/m7c-reviewer-targeted.log`) and inspected the gate logs. Live verdict: no single-machine development-app check is warranted because the changed behavior depends on paired-host/account state and local screens/input/lifecycle are unchanged. Native remote checks for ambiguous rows/Revoke, mission widths/dividers/shell strip, slow-host independence, Forget → pair and Take control/reselection are **Blocked here and left to Jason on macOS and Windows**, under the second-computer/service restriction. No app, daemon, real agent or service was launched/contacted; GPUI in-process rendering is not a native pass.


| Re-review command | Log | Exit/result |
| --- | --- | --- |
| Targeted remote-client nextest command | `target/m7c-followup-rereview-targeted.log` | 0: 16 passed |
| Required full workspace nextest command | `target/m7c-followup-rereview-workspace.log` | 0: 2,729 passed, 6 skipped |
| Standard workspace all-target Clippy | `target/m7c-followup-rereview-clippy.log` | 0 |
| Workspace all-target Clippy with updater | `target/m7c-followup-rereview-clippy-updater.log` | 0 |
| `cargo fmt --all --check` | `target/m7c-followup-rereview-fmt.log` | 0 |
| `git diff --check` | `target/m7c-followup-rereview-diff.log` | 0 |


### Clean re-review and publication

Reviewer message `01M5KJ8FZCJBWNZS7YQ1CRZT0Q` reports **NO REMAINING MUST-FIX ISSUES** on the whole follow-up working-tree diff after both fixes. Independent final targeted nextest exited 0 with 16 passed (`target/m7c-reviewer-rereview-targeted.log`); independent `git diff --check` exited 0. The reviewer inspected the 2,729-pass workspace and both Clippy logs. The reviewed code/docs diff before this verdict was added has binary-diff SHA-256 `c19f186a8bbb74dae43f5bcde16f18c7e2e22e60da38aa943de95cd399062fc5` on Darwin arm64.

The live verdict above is unchanged: no single-machine app check is warranted, and native remote checks stay Blocked here / with Jason on macOS and Windows. No app, daemon, real agent or service was launched/contacted. Both code review findings are resolved; the seven added tests and the extended existing tests use fake protocol responses/events only. The follow-up is folded into the branch’s single integrated commit on unchanged `origin/nightly-remote` at `12b9b80a`. Its post-amend build recheck and macOS/Windows CI are recorded on PR #1005. Jason retains the two-computer checks and merge; no merge, cleanup, nightly or release is authorized.
