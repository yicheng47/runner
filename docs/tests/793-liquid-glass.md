# #793 Liquid Glass — macOS QA record

Run date: 2026-10-09. Verdict: **Failed, with incomplete coverage**. The titlebar alignment and sidebar-collapse checks failed. Several required checks remain Blocked or Skipped. This is not a full feature pass.

## Candidate and authorization

Repository/worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-793-liquid-glass`; branch: `feat/793-liquid-glass`; base HEAD: `ac6a1fc65d284c66f95e4c948b632e754808098f`; SHA-256 of `git diff HEAD`: `ce59388dad36c21d71edf24eb2dad4306bf914d12c7c1c8c8a7d4f63725a8233`. QA verified this identity at handoff and again before stopping. This is the reviewed option B candidate with 88% dark / 90% light chrome and popup opacity. The clean working-tree review was reported by the coder through Runner at 06:58 UTC.

The 06:49 option E handoff (`a592d3e9224a39d3e56e405248d735374bd2b878b2e24d8f5ca822e11444f72d`) was withdrawn by the human at 06:52 UTC before live checks began. Its build and cleanup evidence is historical only, in `/private/tmp/runner-793-qa-nndrer7j`.

Authorization came from `docs/impls/briefs/793-liquid-glass.md` and the coder's frozen-candidate Runner handoff at 06:58:59 UTC. QA followed `docs/tests/full-smoke-test.md` and `docs/features/793-liquid-glass.md`; the current spec's option B and opacity values supersede the brief's older values. Scope was native development-app checks with shell panes only, no new agent chats or missions, no global authentication/configuration changes, no performance measurements, no source/test/expectation edits, and no commits or pushes. The installed Runner CLI was used only for crew coordination.

At 07:17 UTC the human reported the titlebar alignment/collapse defect and instructed QA to record it as Failed. At 07:19:26 UTC the coder relayed Jason's instruction to fix immediately, took back source ownership, and requested that QA stop remaining execution and release Cargo. QA stopped testing, restored the appearance selections and cleaned up. The matrix below therefore records the bounded observations on this candidate, rather than implying the remaining checks ran.

After this run, Jason narrowed QA to behavior related to the feature's code changes. General IME/runtime regression checks are therefore out of scope. Relevant coverage remains the work card/header/titlebar layout, Glass/Solid and transparency, new popup behavior, and terminal rendering where the new card padding affects it. The untouched IME implementation is not a required release blocker for this mission.

## Environment and build identity

macOS 26.6.2 (25G83), arm64, Apple M5 Pro. Two displays were available: DELL U2723QE, 5120×2880 physical / 2560×1440 logical, and a rotated DELL U2720QM, 2160×3840 physical / 1080×1920 logical. Native Windows was unavailable. App zoom was the existing 120%; original appearance was Dark / Glass, with the existing Runner dark palettes and Runner Light / Rosé Pine Dawn light palettes. Those palettes and zoom were not changed.

Tool versions: rustc 1.97.1 (`8bab26f4f`, 2026-07-14), Cargo 1.97.1 (`c980f4866`, 2026-06-30), installed and development Runner 0.13.3, codex-cli 0.160.1, Claude Code 2.1.295, Vim 9.1. Version/display evidence is `evidence/tools.txt` under the scratch root below. Native automation used the host `mcp__cua_repl` API; no version endpoint was exposed by that tool.

QA ran worktree-local `make run`, whose Cargo builds completed successfully and launched `target/debug/Runner` v0.13.3 (dev), then stopped that initial launch to use the documented native-control wrapper. The foreground make session ended with code 2 after the intentional app termination; this was not a compilation failure. Build log: `evidence/make-run.log`.

Native wrapper: `/private/tmp/runner-793-b-qa-nbpss6hn/Runner Smoke Dev.app`, bundle ID `com.wycstudios.runner.smoke-dev`, containing byte-identical copies of this worktree's `target/debug/Runner` and `target/debug/runner-agent-cli`. QA proved equality and hashes before trusting results and checked them again at cleanup:

| Binary | SHA-256 |
| --- | --- |
| Runner | `cebe8a1e2edd74d9bc881c359d1a54525873d1a700af6698ab00b4afdf15d9c3` |
| runner-agent-cli | `4793cd6435183f5fe8f4714f3db420ec1855dacc2eeba37c7d934a724466dadc` |

The running wrapper executable path and PID 31506 were verified with `ps`. The development daemon was PID 31950. The absolute development CLI `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/bin/runner` reported app/CLI 0.13.3 and the development endpoint `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, outside mission mode. Its CLI/daemon binaries matched the built sidecar. Evidence: `candidate.json`, `evidence/dev-status.json`, `evidence/final-binary-hashes.json`, `evidence/ps-before-stop.txt`.

Launches and external development CLI commands removed `NO_COLOR`, `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE` and `RUNNER_EVENT_LOG`. No Runner SQLite database was opened directly. Existing development sessions were automatically restored by the saved startup setting; QA did not intentionally start agents or send them input. The separate installed application and daemon were left running and untouched.

## Method and plan

A short plan was written before live testing in `plan.md` under the scratch root. Required coverage was: dark/light Glass against bright/dark content, Solid and live two-window switching; one card and option B headers across routes/panels/splits/collapse; native menus and usage behavior including all lifecycle paths, fullscreen and both displays; opaque terminal backgrounds, selection and CJK IME; live Reduce Transparency with explicit human permission. Each required behavior had to be visibly demonstrated to pass. The general agent-runtime suite was outside this mission's shell-only authorization.

Native control was verified by opening and dismissing the harmless New Chat dialog without creating an agent. UI actions then used native shortcuts, pointer actions and native paste. Product state came from the absolute development CLI, with test-session IDs matched to the ledger. No private PTY writes, database shortcuts or test-only rendering hooks were used.

Scratch/evidence root: `/private/tmp/runner-793-b-qa-nbpss6hn`. Test fixture files and a temporary `.app` wrapper are outside the repository. Native screenshots arrived as JPEG bytes; their filenames were corrected to `.jpg`, with the mapping preserved in `evidence/filename-corrections.json`. Screenshot paths below are relative to this scratch root's `evidence/` directory. Raw screenshots/state can include existing development sidebar labels and remain local; only this redacted record is intended for commit.

## Result matrix

| Check | Status | Observation / evidence |
| --- | --- | --- |
| Candidate identity, development endpoint and native control | Passed | Exact source/wrapper hashes, process paths, status endpoint; New Chat opened and Esc dismissed, `01-new-chat.jpg`. |
| Development build | Passed | Worktree-local builds completed and candidate launched; `make-run.log`. Intentional termination is described above. |
| runner-app tests, CI profile | Passed | Coder reported exit 0 on this exact revision; copied log confirms 658 passed, 3 existing manual cases ignored; `793-test.log`. QA did not rerun Cargo tests. |
| Workspace Clippy, CI profile, all targets | Passed | Coder reported exit 0; copied `793-clippy-workspace.log`. |
| runner-app updater Clippy | Passed | Coder reported exit 0; copied `793-clippy-updater.log`. |
| Cargo format check | Passed | Coder reported exit 0; copied `793-fmt.log`. |
| git diff check | Passed | Coder reported exit 0; copied `793-diff-check.log`. |
| Dark/light appearance control and Glass/Solid selection | Passed | Native Settings changes visibly applied, `03-appearance-dark-glass.jpg`, `11-light-glass-bright-backdrop.jpg`, `12-light-solid-main.jpg`; original Dark/Glass restored in `36-restored-dark-glass.jpg`. |
| Live appearance/material updates in two windows | Passed | Changed Settings in the original window; the second window rendered Light/Solid without restart, `12-light-solid-main.jpg`, `14-current.jpg`, `16-after-minimize.jpg`. |
| Dark/light Glass against controlled bright/dark backdrop | Blocked | A scratch bright image was opened in Preview, but the bound Runner capture did not establish the correct foreground/backdrop stack. `09-bright-backdrop.jpg`, `10-dark-glass-bright-backdrop.jpg`, `11-light-glass-bright-backdrop.jpg` prove only their visible surfaces, not the full compositor contrast check. The dark fixture was not opened. The Preview test window was closed after Jason objected to the exposed backdrop. No wallpaper change. |
| Single-shell card and primary header | Passed | A real QA shell accepted pasted input and rendered inside the opaque card, `05-new-shell.jpg`, `07-shell-paste-input.jpg`; titlebar defect is a separate Failed row. |
| Split-shell card, option B headers and vertical divider | Passed | Two QA shell panes in one card with transparent headers over the card fill and a continuous divider, `18-light-solid-split-actual.jpg`, `31-own-shell.jpg`, `37-own-tab-before-close.jpg`. Full route/theme cross-product remains incomplete. |
| Roles card | Passed | Second native window displayed Roles in Dark/Glass and later Light/Solid, `04-second-window.jpg`, `14-current.jpg`. |
| Settings card/nav | Passed | General and Appearance in Dark/Glass and Light/Solid, `02-settings.jpg`, `03-appearance-dark-glass.jpg`, `34-settings-current.jpg`, `36-restored-dark-glass.jpg`. |
| Crews, profile and archived-chat card routes | Skipped | Candidate withdrawn before these routes were exercised. No pass is inferred from other routes. |
| Side panel and terminal drawer card/headers | Blocked | Authorized shell-only tabs do not expose these agent-chat controls. QA did not start an agent or operate an existing agent chat to obtain a fixture. `20-side-panel-attempt.jpg` is an unsuccessful attempt, not panel evidence. |
| Existing development mission card/rail | Skipped | An existing mission was visible in navigation, but QA had not opened it before the stop instruction. No new mission was created. |
| Titlebar controls align with card header | Failed | Jason's 07:17 Runner finding: traffic lights, sidebar toggle and back/forward sit 8 px above the card header. Expanded screenshots show the offset; see finding F1. |
| Sidebar collapse retains titlebar/control position | Failed | Native Cmd-S collapsed the sidebar. The toolbar position changes between expanded and collapsed states; `18-light-solid-split-actual.jpg`, `25-collapsed-sidebar-split.jpg`. Jason explicitly instructed this finding be recorded as Failed. |
| Collapsed card outer frame and split divider | Passed | Collapsed sidebar shows one card with outer padding and aligned split body/header divider, `25-collapsed-sidebar-split.jpg`. This does not clear the collapse alignment failure. |
| Solid pane-menu arrow/Enter action | Passed | Opened left pane menu, Down + Enter entered Rename, native text changed QA-owned title to `QA793-left`; `21-solid-pane-menu.jpg`, `22-menu-keyboard-rename.jpg`, `31-own-shell.jpg`. |
| Solid layout-menu Esc and terminal focus return | Passed | Esc dismissed the layout menu; without refocusing the terminal, native paste/Return printed `QA793 focus return` in the previously focused right QA pane, `23-split-side-panel.jpg` (actually layout menu), `24-solid-menu-focus-return.jpg`. |
| Other Solid menus, usage gear, same-trigger/outside click/typeahead | Skipped | Not completed before candidate withdrawal. Only the specific Solid menu behaviors above passed. |
| Glass usage popover opening and capture | Blocked | Trigger attempt produced no popup in the bound-parent screenshot/AX tree, `08-glass-usage.jpg`. The native API exposed only generic window controls. QA could not establish whether an independent popup was open, so this is an evidence/tool limitation, not an attributed product failure. |
| All Glass menus and usage placement, arrows/Enter/Esc/typeahead, outside click, same-trigger toggle, focus return and settings gear | Skipped | No complete visible Glass-popup check was completed before withdrawal. These remain required. |
| Popup dismissal on parent move/resize/minimize/close, app switch and Space change | Skipped | Not demonstrated before withdrawal. Native parent minimization used to inspect the second window does not prove popup dismissal. |
| Popup fullscreen behavior | Skipped | Not completed before withdrawal. |
| Popup second-display behavior | Skipped | Two displays exist; this is an unexecuted required check, not a no-display exemption. |
| Explicit TUI backgrounds and CJK glyph rendering | Passed | Native paste launched Vim on a scratch file with explicit blue background/white text. Output remained opaque and clear of the card edge; `32-vim-solid.jpg` (Light/Solid), `37-own-tab-before-close.jpg` (Dark/Glass). This verifies rendering, not IME composition. |
| Terminal selection against explicit background | Passed | Native drag selected the first Vim line with readable highlighting, `33-terminal-selection-solid.jpg`; still visible after restoring Dark/Glass in `37-own-tab-before-close.jpg`. |
| CJK IME composition at edges/corners | Skipped | Jason narrowed scope to behavior changed by this feature; untouched IME behavior is out of scope. Pasted CJK output is not accepted as IME evidence. No system input-source changes were made. |
| Live Reduce Transparency on/off | Blocked | QA asked through Runner for permission; the human confirmed at 06:52 it remained pending, and no approval arrived during this run. Setting was not toggled. |
| Native Windows/Linux appearance and menus | Blocked | No native Windows/Linux test machine was available. Local macOS tests are not native cross-platform evidence. |
| Frame rate, memory and CPU | Skipped | Explicitly outside this mission's scope. |
| Cleanup | Passed | Own remaining shells stopped, own app/daemon stopped, 14-process tracked tree has no survivors, original Dark/Glass restored and own Preview image window closed; details below. |

## Findings and retained first attempts

**F1 — titlebar offset and collapse jump (Failed).** Expected: traffic lights/sidebar toggle/back-forward align vertically with the card header and do not move when the sidebar collapses. Steps: display the QA split-shell tab with sidebar expanded, then press Cmd-S to collapse. Actual: the expanded sidebar controls sit above the inset card header; collapsed toolbar controls shift into the card header. Jason identified the offset as 8 px through Runner at 07:17:02 UTC and instructed that both checks be Failed. Evidence: `18-light-solid-split-actual.jpg` and `25-collapsed-sidebar-split.jpg`. No baseline was established to attribute this beyond the tested candidate. The coder took ownership of the fix at 07:19:26 UTC. A new reviewed candidate must rerun titlebar/collapse and affected layout checks; this record does not clear the fix.

**Native input limitation (not attributed to product).** The first `typeText` attempt to set a scratch working directory/prompt and print CJK text dropped punctuation/characters, producing a shell `invalid parameter name` error, retained in `06-shell-input.jpg`. A retry through native paste produced the exact command and visible CJK output in `07-shell-paste-input.jpg`. The first error remains recorded; pasted text does not prove keyboard IME. Later mis-targeted pointer attempts are preserved as `19-solid-tab-menu.jpg`, `20-side-panel-attempt.jpg`, `26-terminal-selection.jpg` and `27-vim-opaque-background.jpg`; those filenames do not establish the named behavior. QA used fresh native state and the unique QA title to reacquire the owned shell before the successful Vim/selection checks.

**Glass popup evidence gap (Blocked).** `08-glass-usage.jpg` and the generic native AX tree could not show a separate usage panel. QA did not substitute source inspection for native popup evidence and did not select the authorized fallback. The coder needs either a native observation/control path for the popup or human execution of the remaining popup checks. No popup failure has been established that would by itself justify the fallback.

## Cleanup and evidence

Ledger: `ledger.json` under the scratch root. QA created three shell sessions, no agents/missions/roles/crews. `01M4FQWB01YV4M27Z3E0Z65P7N` was closed during the extra-tab cleanup and was absent from the later CLI list. QA closed the owned split tab and explicitly stopped `01M4FQJAC508FC2Y8BVQXXZ3RW` and `01M4FQWAM4FZK4AAG8XR1P2JWW` with the absolute development CLI; both stops returned exit 0. Attempting the normal archive command returned `terminals close rather than archive`; terminal archive is not supported. That failed attempt is retained in `evidence/archive-sessions.json`; stops are in `evidence/stop-sessions.json`. No pre-existing session was individually stopped or archived.

The initial make-run app/daemon (25503/25993) were stopped before wrapper testing. QA stopped wrapper PID 31506 after verifying its exact executable, then stopped its development daemon PID 31950 through the absolute development CLI (`{"stopped":true}`, exit 0). The foreground wrapper command ended with 143 from the intentional SIGTERM. `evidence/cleanup-process-tree.json` traces 14 processes descending from the QA app/daemon and confirms zero survivors against `evidence/ps-after-stop.txt`. No Vim, QA helper, load generator or Cargo process remains. The unrelated installed application PID 43899 and daemon PID 44311 remain running.

Native Settings restored Dark and Glass (`36-restored-dark-glass.jpg`); original palette/zoom/startup behavior was retained. Preview's scratch image window was closed with native Cmd-W, retaining the pre-existing Preview app. No system setting, wallpaper, authentication or permission configuration was changed. Scratch fixtures, wrapper, logs, screenshots and conversation history are preserved. No branches, worktrees or files were deleted.

Required follow-up: build/review the titlebar fix, verify its new candidate identity, rerun the failed titlebar/collapse checks and all layout behavior touched by the fix, and complete the remaining feature-related matrix under Jason's narrowed scope. Earlier observations describe only the binary and hash recorded here. Do not label the feature fully passed while required feature-related Blocked/Skipped rows remain.

## Follow-up — reviewed PR candidate, 2026-10-09

Verdict: **Failed, with incomplete coverage**. Jason reported a doubled/overlapping Glass popup boundary and supplied a screenshot. The coder withdrew this candidate at 08:09:01 UTC, so QA stopped remaining execution. This follow-up preserves the earlier Failed run above; neither run is a full feature pass.

### Candidate, authorization and environment

Repository/worktree and branch are unchanged. Candidate commit: `bfd360e1e655d1936df86ee23f4e15cf6fe4d0ff`; Git tree: `c23a02c4d3117d317848c00b35e3773c6e9b7b4e`; initial SHA-256 of `git diff HEAD`: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (clean). The coder confirmed this is the exact tree reviewed on rebased HEAD `2553d7238e5d242bb5dfc43b60cb637a0868126c` / diff `6048e4013a3d6cdcb71fb463290028b857dd18fe3851064d996e51e2261db5f1`, with `NO REMAINING MUST-FIX ISSUES` at 07:50:44 UTC. PR: [#852](https://github.com/yicheng47/runner/pull/852).

Jason explicitly instructed QA to start immediately without waiting for CI. The coder supplemented that authorization with the frozen identity at 08:00:16 UTC. Live scope was the remaining changed card/material/popup behavior, with no untouched IME or general agent-runtime matrix. A plan and candidate/environment record were written before checks at 07:58 UTC. Reduce Transparency permission remained pending; no system setting was changed. OS and CLI versions match the earlier run; evidence is `tools.txt` in the new scratch root. Native Windows remained unavailable.

New scratch root: `/private/tmp/runner-793-followup-qa-91prxcdu`; evidence below is relative to its `evidence/` directory. QA ran worktree-local `make run`, verified successful compilation/launch, stopped that initial launch, then made byte-identical executable/sidecar copies in `Runner QA Followup.app` with bundle ID `com.wycstudios.runner.qa793-followup`. Runner SHA-256: `0d060b2b426d20ed3bb0f5a9081e5113eae19bfb07b5b57ef14370ccd0302c1b`, matching the coder's identified build; CLI SHA-256: `4793cd6435183f5fe8f4714f3db420ec1855dacc2eeba37c7d934a724466dadc`. `candidate.json`, `make-run.log`, `ps-wrapper-start.txt` and `dev-status.json` establish the copied binary, actual running path, version 0.13.3 (dev), absolute development CLI and development socket. Crew variables and `NO_COLOR` were removed from launches and CLI children as in the first run. No SQLite file was opened directly.

### Selected checks and results

| Changed behavior / selected check | Status | Evidence and reason |
| --- | --- | --- |
| Candidate build, namespace and native control | Passed | Identified build/hashes above; native New Chat dialog opened and Esc dismissed without creating an agent, `01-harmless-dialog.jpg`. |
| Titlebar alignment fix | Passed | Jason reported his alignment smoke passed before this handoff; this is human evidence reported through Runner, not a relabeling of the initial failure or a new exhaustive QA layout pass. |
| Single-shell work card/input | Passed | One owned shell accepted native paste and printed its unique marker inside the card, `03-new-shell.jpg`, `04-shell-input.jpg`. |
| Popup menu keyboard action | Passed | Opened the owned shell's menu, Down + Enter reached Rename; native paste/Enter saved `QA793-followup`, `07-glass-menu-enter.jpg`, `08-renamed-shell.jpg`. This proves the action path, not popup placement or appearance. |
| Floating Glass boundary has one aligned rim | Failed | Jason's screenshot shows doubled/overlapping lower/left outlines and corner edges. See F2 below and `human-doubled-popup-border.png`. |
| Remaining popup placement/flip, Esc, same-trigger/outside dismissal, focus return, usage gear and parent move/resize/minimize/close/app-switch/Space behavior | Skipped | Candidate withdrawn before these checks completed. The parent capture omitted the independent panel; successful keyboard action alone does not establish these behaviors. No full menu matrix pass is inferred from Jason's general visual assessment. |
| Popup fullscreen and second display | Skipped | Candidate withdrawn before execution. Two displays are available; this is not a no-display exemption. |
| Dark/light Glass/Solid, controlled backdrops and two-window live changes | Skipped | Candidate withdrawn before new-candidate execution. Earlier-candidate observations remain historical. No new Preview window or backdrop fixture was opened in this follow-up. |
| Remaining card routes/panel/drawer/split/mission geometry | Skipped | Candidate withdrawn before the route matrix was completed. The startup view contained a pre-existing stopped chat and side panel (`02-initial-window.jpg`); observing it does not imply all routes passed. No existing agent chat or mission received QA input. |
| Reduce Transparency live override | Blocked | Explicit permission to toggle the macOS accessibility setting never arrived. Setting was left unchanged. |
| Native Windows GUI | Blocked | No live Windows machine was available. Automated Windows CI is separate evidence below. |
| Cleanup | Passed | Owned shell stopped, every identified QA wrapper/daemon stopped, all tracked process snapshots reconciled with zero survivors; details below. |

The coder reported all required local gates exit 0 on this reviewed source: runner-app tests 659 passed / 3 existing manual cases ignored, workspace Clippy, updater Clippy, format and diff checks, plus the development workspace build. Logs were copied from `target/793-*.log` to this scratch evidence directory. At 08:07:51 UTC the coder reported `gh pr checks --watch` exited 0 on this commit: macOS passed in 7m5s and Windows passed in 11m58s, including Windows Clippy, workspace nextest, app build and installer/manifest/icon checks. These are automated results, not native Windows GUI or a completed live feature pass.

### F2 — doubled Glass popup boundary

Expected: the floating menu/usage surface has one aligned rim and rounded outline. Actual: Jason observed overlapping/doubled lower and left outlines and mismatched corner edges on the Glass usage popup. His screenshot was captured at 16:06:40 local time and the coder relayed the finding/withdrawal at 08:09:01 UTC through Runner message `01M4FVAPW7H3WR0YDCP6F136KW`. QA visually inspected the supplied screenshot and retained a byte-identical copy outside Git.

Evidence: `/Users/jason/Desktop/Screenshot 2026-10-09 at 4.06.40 PM.png`, copied to `/private/tmp/runner-793-followup-qa-91prxcdu/evidence/human-doubled-popup-border.png`; hash/source metadata is `human-finding.json`. Minimal reproduction reported by the human: display the Glass usage popup and inspect its lower/left rim and corners. QA did not independently reproduce the appearance because the native API's parent screenshot omits these non-activating panels. The finding is recorded as human-observed on the candidate under test; no prior-version attribution was established. The earlier general observation that things looked okay is retained as context and does not override this specific failure. The coder owns the fix and must route it through review and affected popup QA.

### Follow-up cleanup

QA created only shell session `01M4FTT7RVJE6FS3NQBGA5ZEHM`; the absolute development CLI stopped it successfully (exit 0, `stop-sessions.json`). No agent chats, missions, roles or crews were created, and terminal archive remains unsupported as recorded above. No appearance, input-source, wallpaper, transparency, authentication or permission setting was changed in this follow-up; later human UI changes were retained.

Initial make-run app/daemon PIDs 74227/74394 were stopped before wrapper testing. The original foreground wrapper/daemon were 81660/82116; the foreground session had exited 0 before teardown. Inventory found the same scratch wrapper restarted as 58537/58538. The first teardown's stale-PID lookup safely stopped without signaling an unknown process; QA then verified the current exact wrapper executable path, stopped 58537, and stopped the development daemon through the absolute CLI (`{"stopped":true}`, exit 0). QA did not query the closed app again. This process change is a cleanup observation, not an attributed product failure.

`cleanup-all-snapshots.json` reconciles all captured build/app/daemon descendant inventories against `ps-final.txt` and reports zero survivors. `cleanup-process-tree.json`, `daemon-stop.json`, `ledger.json` and the preserved process snapshots provide the audit trail. The installed app PID 43899 and installed daemon PID 44311 remained running and untouched. Scratch artifacts, screenshots, logs, test history and both failure records were retained; no branches, worktrees or files were deleted. Source/Cargo is released back to the coder after this record and verdict notification.

Subsequent authorization: at 08:11:13 UTC the coder relayed Jason's explicit instruction to stop QA and let him check the fix visually. At 08:12:31 UTC Jason reduced the product scope to retain sidebar Glass and restore all menus/usage popovers to Solid in-window presentation. QA had completed cleanup and released source/Cargo; it remains stopped, with no restart on an earlier handoff. The failed native-popup observations above describe the withdrawn candidate, not the unbuilt fallback, and do not prescribe native Glass-popup tests for the reduced scope. Jason will visually test the next fix himself.

## Human confirmation of the Solid-menu fallback

Jason confirmed the revised appearance and explicitly requested commit and push on 2026-10-09 (recorded by coder at 08:31:53 UTC). The confirmed build restores all menus and usage to Solid in-window presentation and retains sidebar/persistent chrome Glass, the opaque work card and fixed titlebar alignment. Its working-tree candidate was HEAD `bfd360e1e655d1936df86ee23f4e15cf6fe4d0ff` / diff SHA-256 `2284c1b03cae8fac1547e365eae8f51b1c199b252c35e2f015e49fc5ee882519`; `target/debug/Runner` SHA-256 was `33003bc1765894a0cdb5e105cb058300c38c28e5903fbcc467887e0a3cffe7a5`. This is human visual acceptance, not a new QA pass; QA remains stopped and both historical Failed/incomplete verdicts above remain unchanged. Reduce Transparency and native Windows GUI remain unverified.

The fallback's required local gates and build all exited 0: `cargo test --locked -p runner-app --profile ci --no-fail-fast` (656 passed, 3 existing manual ignored), workspace Clippy, updater Clippy, formatting, diff check, and `cargo build --locked --workspace -j12`. Logs and exact commands are in `target/793-solid-fallback-*.log` and `target/793-solid-fallback-gates.json`; these automated results do not establish the unexecuted live matrix.
