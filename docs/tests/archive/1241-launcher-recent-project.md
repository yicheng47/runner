# #1241 — Launcher recent-project live QA

**NO REMAINING MUST-FIX QA ISSUES.** All selected macOS checks L1–L5 passed on 2026-11-09, 17:23–17:37 CST. No required selected check is unresolved.

## Candidate and environment

Candidate: `d1713f3801942645ff53aad76839bdcf4027427e`, reviewed local candidate on `feat/1241-launcher-recent-project` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1241-launcher-recent-project`. The worktree was clean before QA; the only QA change is this record. macOS 26.6.2 (25G83), arm64; Runner app and CLI 0.19.6 development builds. The development CLI confirmed `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, with mode `outside a mission`.

Authorization: the codex trio’s reviewed-candidate handoff and the project’s standing development-app authorization. Scope is the brief’s L1–L5, including keyboard project selection in L5. Only owned Terminal sessions and two disposable projects were created. No agent prompts were sent. Automated gates were already passed by the coder and were not repeated. Remote native behavior and Windows/Linux native checks were not selected by the handoff; this record makes no claim about them. CI remains separate validation.

The shared QA lock was acquired before `make run`. The build ran in a child environment without the calling mission’s `RUNNER_*`, agent-specific variables, or `NO_COLOR`. After the build, its first app process was stopped and `script/screenshot-app bundle` / `launch` wrapped byte-identical app and CLI copies at `/private/tmp/runner-1241-qa-d1713f38/Runner Screenshot Dev.app`. Native computer use was bound to that exact wrapper. The coder was notified when the build finished; QA did not rebuild after the handoff.

App SHA-256: `65ec2fd1638d05f90e04782ba898cdf8335a616a2150a3340deac303b017b751`. CLI SHA-256: `2bc483f18cf27fde05c292aadfec4bbbc18c27edb30a09e8e3f0cbf84130746c`. Both original/copy pairs matched. Development settings and saved window state were backed up before launch. Evidence root: `/private/tmp/runner-1241-qa-d1713f38/evidence`; its parent retains the manifest, backups and wrapper until the PR merges.

## Results

Evidence names below are relative to the evidence root. Screenshots retain their capture dimensions; structured CLI evidence independently confirms the started sessions’ project and working directory.

| Check | Result | Observation and evidence |
|---|---|---|
| L1 — starting project | Passed | From A’s Terminal, ⌘N and top sidebar New tab show editable A with its folder; the dashed row is in Recents and A’s row is unhighlighted. Starting Terminal files it under A at `/private/tmp/runner-1241-qa-d1713f38/project-a`. Screenshots: `L1-shortcut-A.png`, `L1-sidebar-A.png`, `L1-started-in-A.png`; CLI: `L1-sessions.json`. |
| L2 — fallbacks | Passed | An owned Recents Terminal has no project and ⌘N shows No project. From A’s tab, changing the page’s picker to B and pressing ⌘N restores editable A and A’s folder. Screenshots: `L2-recents-no-project.png`, `L2-picker-changed-B.png`, `L2-origin-restored-A.png`; CLI: `L2-sessions.json`. |
| L3 — sidebar placement | Passed | With Recents collapsed, or in Active view, no dashed row shows and the top New tab entry is selected while the picker remains A. With only A collapsed, A stays collapsed and unhighlighted; the dashed row remains in expanded Recents, as the signed spec requires. Screenshots: `L3-recents-collapsed.png`, `L3-A-collapsed.png`, `L3-active-view.png`. |
| L4 — WS-CREATE-17 | Passed | A + locks the picker and places the dashed row under A. B + expands collapsed empty B and moves the locked picker and row there. Clicking either locked picker does not open a menu. Recents + shows editable No project with the row in Recents. Changing its picker to A and choosing A/inside with Change keeps the row in Recents; Terminal starts under A at that nested folder. L1/L2 cover shortcut/top-row origin behavior and L3 covers the collapsed Recents/Active variants. Screenshots: `L4-A-plus-locked.png`, `L4-B-plus-locked.png`, `L4-recents-plus.png`, `L4-recents-picker-editable.png`, `L4-folder-change-row-stable.png`, `L4-inside-terminal.png`; CLI: `L4-sessions.json`. |
| L5 — picker focus | Passed | Mouse selection of B returns navigation to the launcher: Down visibly moves the row selection, four Down presses select Terminal, and Enter starts it under B at B’s folder. Keyboard project selection (End, Up, Enter to A) also restores Down navigation. Esc dismisses an open picker; the next Down moves to the next launcher row without reopening the menu. No search-field click is used after any picker close. Screenshots: `L5-keyboard-picked-A.png`, `L5-keyboard-choice-arrows.png`, `L5-mouse-picked-B.png`, `L5-mouse-choice-arrows.png`, `L5-Terminal-selected.png`, `L5-B-terminal-started.png`, `L5-open-menu-before-Esc.png`, `L5-Esc-closed.png`, `L5-Esc-arrows.png`; CLI: `L5-sessions.json`. |

The brief’s L3 sentence saying no dashed row for all three variants conflicts with its signed spec. QA followed the spec and the coder’s clarified handoff: collapsing A alone keeps the row in expanded Recents. This is a clarified expected result, not a failed check.

## Findings

No failed or blocked selected checks. No implementation changes were made during QA.

## Owned objects and cleanup

Projects: A `01M6ZT33637QMDTQXCESTT5Y06`, B `01M6ZT336ZA0S5W3PD28AV24HY`. Terminal sessions: A fixture `01M6ZT5DMXBV83Y1ERJE85FQXG`, L1 A `01M6ZT6FZJH8RF0V6BFPP89TEP`, Recents fixture `01M6ZT83J4WMRSWNKYS35WWG60`, A/inside `01M6ZTF9K7A8BV90TSKF0KX3R3`, L5 B `01M6ZTH3VZ4TGHW6PV94HSSA20`.

Terminal sessions close rather than archive. After checking every project member against the owned ledger and scratch path, the development CLI deleted A and B with `project delete <id> --force`, closing their four Terminals. The remaining owned Recents Terminal was selected in the native app and closed with ⌘W. CLI session/project lists confirmed zero remaining owned objects and no missing pre-existing session or project IDs. Empty fixture directories (`project-a/inside`, `project-a`, `project-b`, `recents`) were removed; evidence, history and wrapper files remain. No pre-existing chat or mission was typed into, archived or directly operated.

The owned wrapper and development daemon exited, the wrapper was unregistered from LaunchServices, and process inventory confirmed neither remains running. The only UI-setting difference was `sidebarActiveOnly`; settings, launcher recents and saved window state were restored byte-for-byte from the pre-launch backups. The QA lock was released and reports free. Evidence: `cleanup-projects.json`, `cleanup-sessions.json`, `cleanup-project-list.json`, `cleanup-verification.json`, `settings-diff-before-restore.json`, `settings-restoration.json`, `cleanup-projects-deleted.png`, `cleanup-owned-recents-terminal.png`, `cleanup-terminals-closed.png`.

No PR existed during this run. The coder owns the PR description and labels; these results support removing `needs-live-test` for the selected macOS matrix once recorded there.
