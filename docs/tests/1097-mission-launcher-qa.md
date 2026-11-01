# Mission launcher macOS QA

Current verdict: NO REMAINING MUST-FIX QA ISSUES on `a6b315b08a29026e0216172aa996374824d0ed69` for the selected feature matrix. Revised L1/L5/Cmd+N/split checks passed on `6b2f9221`, with the scope defect fixed and its affected rerun passed on `a6b315b0`. Original failure is retained below. The older dashed-New/tab behavior is superseded. L6 remains Blocked without a second host; Windows native remains Skipped, CI separate.

Candidate: `8e973b9e48b547631d1882ec477a77d995eabdd1`, clean `feat/1097-mission-launcher` at handoff on 2026-11-01. Evidence root: `/private/tmp/runner-1097-qa.45fr_was`. Automated checks belong to coder; no rerun by QA. Live checks target a copied development app built from this candidate, with development data only.

## Selected behavior and checks

| Case | Changed behavior and reason | Planned check | Result |
| --- | --- | --- | --- |
| L1 | Mission entry points now route through launcher | Empty/busy shortcut, Mission row, palette, crew list/editor | Passed |
| L2 | Crew selection and dismissal share launcher state | Pictures/roles, roleless crew, typed results, Esc/Backspace/× ownership, numbered chat picks | Passed |
| L3 | Goal input and mission start replace modal | Six-line growth/scroll, folder/project, effective slot setup, empty/one-slot starts and goal delivery | Passed |
| L4 | Missions require a whole tab | No split Mission row or mark, shortcut creates full tab | Passed |
| L5 | Launcher tab appears as New in current scope | Four steps' mark, dashed row, project A/B/none and folder-follow, collapsed reveals, direct +, numbering and replacement/removal | Passed |
| L6 | Remote capabilities and catalog govern mission UI | Second host required; remote crews, gates, computer-switch cleanup, no Empty mission or setup claims | Blocked: no enrolled second host |

Only owned scratch fixtures will be operated and archived; existing chats/missions remain untouched. Windows native validation is outside this macOS run; CI is separate coverage.

## Environment and prerequisites

macOS 26.6.2 (25G83). Design repo pulled unchanged at `05b4841`; all eight required nodes read via Pencil using the canonical `/Volumes/samsung/` file path, including nested sidebar frames. Development CLI reports `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`. No development GUI was running at initial native inventory; an existing development daemon was present. `remote computers --json` reports “Sign in to your account first”: L6 is Blocked, with no enrolled second host available.

Fixtures created through development CLI: role `qa1097` (`01M6BBWADDZ8S87RB4H4CSTNAR`), crew `QA1097 Crew` (`01M6BBWAEAD10BHV2KDCT661S3`, slot `lead`), roleless crew `QA1097 Roleless` (`01M6BBWAFCJNDQ7NSCWDG5F9VV`), project A `01M6BBWAGDNQRQBJ1CFTASNJDQ`, project B `01M6BBWAGY25QQJPJAE3GDQ9CR`. Codex role/slot uses `gpt-6-luna`, low effort, default workspace-write/on-request mode; config captured before agent startup.

## Live results

L1 Passed: ⇧⌘M reused an empty launcher and opened a full launcher from an occupied QA chat; Mission row and palette New mission reached crews; crew list play and crew page Start mission reached the selected goal step. Evidence: `palette-crews.png`, `busy-entry.png`, `crew-list-start.png`, `crew-page-start.png`.

L2 Passed: crew pictures/handles appeared; QA role preceded matching Crew-labelled results; roleless crew was dimmed with No roles yet and opened its page. Esc, empty-field Backspace and chip × removed entry-point tabs; reused launchers returned home, including Esc during search. Goal × returned to crews. Recent contained agents/roles; pressing 1 on the default empty search started the QA role chat. Evidence: `typed.png`, `roleless.png`, `entry-dismiss.png`, `entry-backspace.png`, `entry-x.png`, `reused-backspace.png`, `reused-search-escape.png`, `goal-x-crews.png`, `number-start-chat.png`.

L3 Passed: Shift+Enter added lines; the goal grew to six visible lines and scrolled at eight without further growth. Folder Change used the native picker and canonical scratch paths; project picker worked. Slots showed the local effective QA role/model/effort and clicking them did not open an editor. Enter started both the goal-less Empty mission and bounded one-slot crew mission. Empty title was `Mission 2026-11-01`; the crew title was `QA1097 GOAL DELIVERY`, its first nonempty goal line. The member terminal displayed the supplied goal and `ACK QA1097 GOAL DELIVERY`, then became idle. Evidence: `six-lines.png`, `eight-lines.png`, `read-only-slots.png`, `empty-start.png`, `goal-ready.png`, `goal-delivery-terminal.png`, `empty-mission.json`, `crew-mission.json`.

L4 Passed: the QA chat's split launcher showed Terminal but no Mission row, Runner mark or tagline. ⇧⌘M opened a full-tab crew launcher; Esc returned to the split. Evidence: `split.png`, `split-mission-entry.png`.

L5 Passed: mark/tagline visible on default, typed, crew and goal steps; no mark in the split. Muted normal-weight app-window New row had selected fill and faint dashed outline. Project/Recents + opened directly, revealing previously collapsed destinations. New moved A → B → No project/Recents through the project picker and followed a folder change from A to B. Starting replaced New with the actual chat/mission row. Selecting another owned chat removed the launcher. With Projects collapsed to exclude pre-existing sessions, Cmd+1 skipped New and selected the first real QA chat, removing New. Evidence: `default.png`, `typed.png`, `crew-step.png`, `crew-page-start.png`, `project-plus.png`, `project-b.png`, `no-project-row.png`, `recents-collapsed.png`, `recents-revealed-row.png`, `project-plus-collapsed-reveal.png`, `folder-follow.png`, `shortcut-before.png`, `shortcut-after.png`, `number-start-chat.png`, `empty-sidebar-replaced.png`, `selection-removes-new.png`.

L6 Blocked: development remote computers command required sign-in; no enrolled second host. No remote native pass claimed. Windows native testing Skipped (macOS run); coder-owned CI remains separate.

## Candidate, artifacts and cleanup

Built with `env -u NO_COLOR CARGO_BUILD_JOBS=12 cargo build --locked -p runner-app -p runner-cli -p runner-daemon`; build passed. Copied Runner and runner-agent-cli into `Runner QA1097 Dev.app` under the evidence root; both copies were byte-identical. The CLI artifact also supplies runnerd. App SHA256 `4418d5feea908017e17a4d6a378e7950b5f1993975faece9b31b6ed3db63bf8c`; CLI SHA256 `6113bca1eb0b794767f0596c45d2fe4fd376306824f2c579a145879ae3867895`. Wrapper bundle ID `com.wycstudios.runner.qa1097-dev`, PID 54910. Development daemon was stopped as the normal launch procedure requires, then the copied wrapper launched with mission identity and NO_COLOR removed. No automated suite was rerun. Implementation tree remained at the candidate; only this QA record was untracked.

Owned Empty mission `01M6BC2FY8KHH08NTEBBZ4KSG2` and crew mission `01M6BC7VG5QNYJH53YN1J9YEFB` are completed and archived. Owned direct chats `01M6BC1H2H1HQPPS4BBSXQET2Y` and `01M6BCD3Y4Y0HJJ1TZSVVJH2PG` are stopped and archived. Cleanup snapshots and archive responses are under `evidence/cleanup-*.json` and `evidence/session-archive-*.json`. QA role, crews and projects remain as labelled fixture configurations; no destructive fixture deletion was performed. Scratch files and copied app are preserved for review.

Removed only the three run-owned Codex trust entries; config is byte-identical to its captured original and `config-after.diff` is empty. No authentication or other global configuration was changed.

Native control was interrupted by external app interactions several times, including window resize/navigation; state was refreshed before proceeding. One stale sidebar click after archiving selected a pre-existing pi tab; no input or session-control action was sent to it, and QA returned to its own chat. This selection is an execution-isolation incident, not a product failure, and is not counted as validation. No pre-existing session was stopped or archived.

The wrapper did not exit after a final Cmd+Q attempt. Jason then requested to perform his own final test, so QA stopped native actions and left the still-running copied candidate for him. Its temporary LaunchServices registration had already been unregistered; files remain available at the wrapper path.

QA verdict: NO REMAINING MUST-FIX QA ISSUES in the selected local feature matrix. L6 remains disclosed Blocked. Jason reported that his final manual live test passed on 2026-11-01. The human final-test hold is cleared; coder may proceed with the authorized history shaping, push, PR and CI.


## Revised no-tab launcher rerun (2026-11-01)

Candidate `6b2f92212f657770d17ae4ea2fd69215cd0310b6`, clean feature branch at handoff; full delta code-review clean. This section supersedes old empty-tab/dashed-New behavior and affected L1/L2/L4/L5 assertions above. Unchanged goal/crew contents and start delivery retain prior evidence. Remote L6 remains Blocked; Windows native remains Skipped.

Selected before testing: L1 every mission entry point and Cmd+N route to workspace page without creating a tab; L5 mark on default/search/crew/goal, selected New chat navigation without tree row or pane header/×/split menu, Esc restores prior tab, sidebar navigation leaves page; successful starts reveal selected real rows in initially collapsed project and Recents; split-pane retained header/×/no Mission/mark plus Cmd+N/Shift+Cmd+M page routing; crew/goal backward navigation. These cover the revised routing and the review's collapsed-destination defect; no broader rerun. Results pending.

Build passed: `env -u NO_COLOR CARGO_BUILD_JOBS=12 cargo build --locked -p runner-app -p runner-cli -p runner-daemon`. Copied wrapper `Runner QA1097 Revised Dev.app`; evidence under `/private/tmp/runner-1097-qa.45fr_was/evidence-revised`. Source build/copy completed and coder notified before native checks. App SHA256 `0c283bc164e6c857d44e2269a91b76b7117f6867e96742369d731139f9c8df24`; CLI SHA256 `6113bca1eb0b794767f0596c45d2fe4fd376306824f2c579a145879ae3867895`.

Startup prerequisite Blocked at 12:12 UTC: initial copied app PID 46911 aborted after runnerd connection closed, with startup.rs:315 panic in `evidence-revised/app.log`; subsequent LaunchServices attempt PID 47448 has no native window (`cgWindowNotFound`). Inventory shows concurrent `Runner Smoke Dev` from `/private/tmp/runner-1102-qa-xiyiuzd_` using development data. Cause is not established against a baseline and no launcher result is claimed. Asked Jason through Runner to have the other test owner yield development-app use; did not stop their app or operate their sessions.

Jason confirmed at 12:13 UTC that the startup panic was caused by concurrent #1102/#1103 QA using the same development daemon/data. Classified as an environment conflict, not a product failure. Native rerun paused at his direction; no further launch or interaction until he reports that the other app exited. A fresh launch is required when released.

At 13:58 UTC, yielded development namespace to #1102/#1103 QA per Jason. Verified and terminated only own leftover wrapper PID 47448; `ps` confirmed exit. Cross-mission reply sent to their qa handle before this follow-up. Development daemon was left untouched after yielding because the other QA owner may now be using it. No session operations. #1097 remains paused until Jason reports #1102 QA passed; then fresh-launch selected rerun on frozen 6b2f9221.

Fresh launch resumed after the other QA owner reported a passed run and closed wrapper at 14:07 UTC. Own copied app PID 78812 started normally after development-daemon restart; exact dev socket verified. Revised design read through Pencil at canonical path (published a6e8e7a); no design edits. Native display size changed externally during checks; refreshed state and fitted the owned window before continuing.

### Revised candidate results

| Check | Result | Evidence / limits |
| --- | --- | --- |
| L1 shortcut from launcher and occupied tab/split; Mission row; palette New mission | Passed | `shortcut-crews.png`, `mission-row.png`, `palette-crews.png`, `occupied-mission-page.png`, `split-mission-page.png` |
| L1 crew list and crew page Start mission reach selected goal | Passed | `crew-list-goal.png`, `crew-editor-goal.png` |
| L1 direct project + scope | Failed | Project + changed cwd to QA1097 A but kept No project selected, including picker checkmark; `project-plus-no-project.png`, `project-plus-picker.png`. CLI project list confirmed canonical matching cwd. No baseline comparison; reported immediately to coder/reviewer. |
| L1 Recents + and sidebar New chat | Passed | Direct workspace page, no menu or tree row; `recents-plus-collapsed.png`, `cmdn-page.png` |
| L5 mark on all four steps; New chat navigation selected; no pane header/split-menu/× or New tree row | Passed | `cmdn-page.png`, `search-page.png`, `shortcut-crews.png`, `goal-page.png`; existing real tabs stay in tree |
| Crew/goal backward navigation | Passed | Goal Esc/Backspace/× -> crews; crew Esc/Backspace/× -> default; `goal-x-back.png`, `crew-x-default.png` |
| L5 Esc restores previous tab; sidebar row leaves page | Passed | Owned terminal and empty split restored; owned mission selection leaves page; `escape-restored-tab.png`, `split-restored.png`, `sidebar-row-leaves-page.png` |
| L5 actual start reveals collapsed project destination | Passed with explicit picker selection | Selected QA1097 A through picker after reported + defect; terminal start expanded A and showed selected real row. `collapsed-project-before-start.png`, `project-start-revealed.png`; this does not pass the + scope defect. |
| L5 actual chat and mission starts reveal collapsed Recents | Passed | Own chat from Cmd+N expanded Recents and selected row; own Empty mission did likewise. Scrolled sidebar to view row below viewport. `cmdn-chat-before-start.png`, `cmdn-chat-recents-revealed.png`, `mission-collapsed-before-start.png`, `mission-start-revealed.png` |
| Real empty split pane retained; Cmd+N/Shift+Cmd+M open workspace page | Passed | Empty split had header/split menu/×, no mark or Mission row; workspace page had mark/no header. `empty-split.png`, `empty-split-mission-page.png`, `split-cmdn-page.png`, `split-restored.png` |
| L6 / Windows native | Blocked / Skipped | Prior disclosed limits unchanged; no broader rerun. |

Owned rerun objects: terminal sessions `01M6BQ8KQVMMSAWNP975DDTKQA` and `01M6BQ9Q9K1194ZA7A6RSTQXVQ` (second spawned by normal terminal Split Right); bounded Codex chat `01M6BQDFESYGW0S1KZRJ4FPXV3` (gpt-6-luna low, scratch revised-chat); Empty mission `01M6BQJF05XSTE5K668BF66WQG` (scratch revised-empty, title Mission 2026-11-01). Chat reached idle after its bounded role prompt; no new delivery claim needed for this rerun. All stopped. Chat and mission archived with snapshots; shell stop removed shell sessions, so later archive/show returned session not found. Final session list contains none of these owned sessions. Removed only new own revised-chat trust section; config matches pre-chat baseline byte-for-byte (`config-after.diff` empty). Closed own copied wrapper PID 78812 after checking executable identity; left shared daemon and unrelated sessions alone.

Revised verdict on 6b2f9221: one remaining must-fix QA issue, project-plus scope label/picker state. Other selected checks passed as listed. Await reviewed fix; rerun only project-plus scope and its actual-start placement, with any justified affected navigation. Prior old-design local pass is not a pass for this revised candidate.

## Project scope fix rerun

Frozen reviewed candidate `a6b315b08a29026e0216172aa996374824d0ed69`; only QA record dirty at handoff. Same development build command passed, then copied into `Runner QA1097 Fixed Dev.app` (bundle `com.wycstudios.runner.qa1097-fixed-dev`, PID 90021). App SHA256 `df8571ef213c08845daaa5b3a952cbe227e987533905263dc339d7a32b34b439`; CLI unchanged at `6113bca1eb0b794767f0596c45d2fe4fd376306824f2c579a145879ae3867895`. No other QA wrapper running before launch. Development daemon restarted for the fresh copied candidate; source-copy completion notified to coder. Evidence directory `evidence-fixed` contains artifact hashes, app log, native screenshots and CLI session snapshots. No automated suite rerun.

Selected affected checks before testing: programmatic scope changes through project A/B + and Recents + while reusing default, crews and goal page; displayed label/checkmark agreement; real start scope/cwd and selected row placement into collapsed project and Recents. Native representatives were project A + from default, project B + from crews, and Recents + from goal.

| Affected case | Result | Evidence |
| --- | --- | --- |
| Default page -> project A + | Passed | Label QA1097 A, picker checkmark A and canonical project-a path. `project-a-start-label.png`, `project-a-checked.png` |
| Crew page -> project B + | Passed | Returned default page with label QA1097 B, checkmark B and canonical project-b path. `project-b-from-crews-checked.png` |
| Goal page -> Recents + | Passed | Returned default page with No project checked; retained scratch folder is independent of project membership. `recents-from-goal-checked.png` |
| Actual start after project A +, destination collapsed | Passed | Created shell `01M6BQY7DDJPAN6KD9V5CSMMDE`, project id A and cwd project-a confirmed by CLI; A expanded and actual row selected. `project-a-collapsed-before-start.png`, `project-a-start-revealed.png`, `sessions-before-cleanup.json` |
| Actual start after Recents +, destination collapsed | Passed | Created shell `01M6BR1BZKDSNK051ZB6TTKWE6`, project id null and scratch cwd project-b confirmed by CLI; Recents expanded and actual row selected. `recents-collapsed-before-start.png`, `recents-start-revealed.png`, `sessions-before-cleanup.json` |

Both owned shell sessions stopped successfully and disappeared from the final CLI list, as shell stop removes them. No new agent/chat/mission or trust entry in this affected rerun. Closed own copied wrapper PID 90021 after executable identity check; shared daemon and unrelated sessions left alone. One sidebar-position shift caused a transient selection of the pre-existing shell tab while preparing Recents collapse; no terminal input or session-control action was sent to it, and QA returned to the launcher. Disclosed as execution isolation, not product evidence.

Final verdict: NO REMAINING MUST-FIX QA ISSUES. Revised selected L1/L5/Cmd+N/split pass carries forward from 6b2f9221 with the original project-plus Failed observation preserved and the affected fix rerun Passed on a6b315b0. Unchanged prior L2-L4 content/input/delivery checks remain applicable, with their old tab/dismissal/routing assertions superseded by the revised results. L6 Blocked and Windows-native Skipped remain disclosed. This is a selected feature pass, not a full regression pass; Jason's earlier manual pass preceded the revised design and is not claimed as a manual pass of this revision.

## Rounded launcher corner follow-up

Original visual failure: Jason reported a square dark background spill outside the rounded default launcher page while final CI ran; prior `evidence-revised/cmdn-page.png` also shows the corner spill. This remains a recorded Failed observation on the preceding source, separate from the selected behavior pass. Coder traced it to the launcher child's rectangular background over the rounded parent.

Local delta `554fb39f97e92b6f9c3594aea0e06a054e623440` (parent `5a6ba8dd`) removes that redundant child fill. Reviewer reported NO REMAINING MUST-FIX CODE REVIEW ISSUES at 14:32 UTC after checking both page and real-pane background owners. Proposed targeted live coverage was all four outer corners on default, typed search, crews and goal, plus the background/header of a real empty split pane, with no agent or mission start.

Jason took ownership of the live check, superseding the proposed QA build/native handoff. Coder relayed Jason's live test Passed at 14:33:55 UTC and his authorization to merge once final CI passes. This is a human-reported pass; QA did not independently rerun the corner check and has no new human screenshot evidence path. No QA build, native launch, agent usage or configuration change occurred for this delta.

Final validation disposition: prior selected QA passes and disclosed L6/Windows-native limits stand; the original corner failure is preserved, its minimal fix is review-clean, and its live validation Passed by Jason's report. No remaining must-fix QA issue is known. Final history folding, CI and the now-authorized merge belong to coder.
