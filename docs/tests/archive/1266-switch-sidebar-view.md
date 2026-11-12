# #1266 — Switch sidebar view QA

## Candidate and method

QA candidate: `4bb3d39253d3d77ab35e364031898c2ddf73421c`, clean-reviewed by `reviewer-2`, branch `feat/1266-switch-sidebar-view`, base `b841b51a63fae25426fd9a36b4da23b927213efa`. Repository: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1266-switch-sidebar-view`. Before this QA record was created, the working tree was clean; the empty binary diff SHA-256 was `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

Authorized by the #1266 codex trio mission. Follow the feature spec and the candidate identity, development-data isolation, native control, evidence and cleanup rules in `full-smoke-test.md`; this is feature QA, not a full regression. Product actions use native computer use and the absolute development CLI with implementation mission identity removed. No SQLite access. QA owns this record and does not commit it.

Environment: macOS 26.6.2 (25G83), Rust 1.97.1, Cargo 1.97.1, installed coordination CLI Runner 0.19.7. macOS CUA preflight succeeded using a harmless Finder Escape. No Windows native computer-use target is exposed; Jason explicitly chose to record Windows as Blocked and retain `needs-live-test` on 2026-11-12. This does not waive that check.

Execution date: 2026-11-12. Product runtime fixtures were two `/bin/zsh` 5.9 terminals; no agent account was needed for the selected checks. Pre-existing development sessions resumed automatically under the standing development-app authorization and received no QA input or individual lifecycle operations.

Built with `make run` after acquiring `script/qa-lock`; stopped the bare app process created by that command, then copied this worktree's `target/debug/Runner` and `target/debug/runner-agent-cli` into `/private/tmp/runner-1266-qa-gtysecjn/Runner 1266 QA.app/Contents/MacOS/`. Byte comparisons passed. Launched that executable directly with `NO_COLOR` and the implementation mission's four identity variables removed and `RUNNER_SESSION_CLI` empty. CUA selected bundle `com.wycstudios.runner.qa1266`; opening the launcher proved native control. The absolute development CLI reported `0.19.7 (dev)`, `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`, and mode `outside a mission`. Its installed sidecar hash matched the built CLI. The final HEAD remained the reviewed SHA and there was no product-source diff.

| Binary | SHA-256 |
|---|---|
| Built app and wrapper copy | `510994fee1e7c7e411df7e5c2245af7f3ab9fdb37f0095cfbdd0a5208edd38ed` |
| Built CLI, wrapper sidecar and installed development CLI | `e337d2a799ac0276c08e2e0be2b9f3f5faab3792044044d7f524737813e8d105` |

Evidence root: `/private/tmp/runner-1266-qa-gtysecjn`. Screenshots are under `evidence/`; original captures are retained, alongside smaller `*-preview.png` copies. `ledger.json`, `versions.txt`, `status.json`, `rebound-settings.json`, `before-reopen-state.json`, `settings-restoration.json`, process inventories and app logs provide supporting identity and cleanup evidence. Large evidence stays outside the repository.

## Selected checks and pass criteria

| Check | Platform and runtime | Why selected and required result |
|---|---|---|
| View switching, collapsed expansion, button parity | macOS, development app | The new action shares the filter path. Cmd+. must switch Projects → Active → Projects when open and expand into the newly selected view when collapsed; the existing button must agree. WS-ACTIVE-04 switching slice. |
| Saved view | macOS, development app | Local preference is part of the feature. The selected view must survive app quit/reopen. WS-ACTIVE-04 persistence slice. |
| Number selection | macOS, owned test rows | Shortcut rows are synchronized by the changed path. With visibly different row orders, Cmd+number must select the row currently shown after each view switch. WS-ACTIVE-02 selection slice. |
| Settings and rebinding | macOS, development app | The action must be discoverable and configurable. Find its row, save another binding, show that the new key switches and Cmd+. does not, then restore the original binding. |
| Command palette | macOS, development app | The action is newly exposed in the palette. Find and execute it, including from a collapsed sidebar, and observe the changed expanded view. |
| Ctrl+. routing and Outside terminals | Windows, owned raw-input terminal | The new Ctrl binding must respect terminal priority. Runner first switches with no PTY bytes; Terminal first with Kitty disambiguation emits CSI 46;5u without switching, outside-terminal focus still switches, and Settings shows Outside terminals. After popping keyboard flags, legacy input emits literal dot. TERM-INPUT-06 routing slice. |

Remote path is covered in-process by the coder's automated tests; no new remote fixture is required. No broader IME, runtime or regression matrix is selected.

## Results

**Overall: Incomplete — all five selected macOS checks Passed; required Windows native routing is Blocked. Keep `needs-live-test`.** No product failure was observed in the selected macOS checks. This is not a full regression pass.

Screenshot filenames below are relative to the evidence root's `evidence/` directory.

| Check | Result | Observed result and evidence |
|---|---|---|
| View switching, collapsed expansion, button parity | **Passed** | With owned terminal A focused, Cmd+. changed Projects → Active → Projects. Cmd+S collapsed Projects; Cmd+. expanded into Active. Collapsing Active and pressing Cmd+. expanded into Projects. Clicking the existing filter button selected Active and clicking it again returned to Projects. `03-shortcut-active.png`, `04-shortcut-projects.png`, `05-collapsed-projects.png`, `06-expanded-active.png`, `07-collapsed-active.png`, `08-expanded-projects.png`, `09-button-active.png`. |
| Saved view | **Passed** | Selected Active with the shortcut, quit the wrapper through its native dialog using Keep them running, verified the app process exited, then launched the same wrapper binary again. Active reopened with both owned terminal rows. The saved file contained `sidebarActiveOnly: true` and `sidebarCollapsed: false` before reopening. `25-active-before-quit.png`, `26-quit-keep-running.png`, `27-active-after-reopen.png`, `before-reopen-state.json`. |
| Number selection | **Passed** | Projects put owned B before A in Recents: Cmd+5 selected A. After switching to Active, owned terminals were oldest-first after two Agents rows: Cmd+4 selected B and Cmd+3 selected A. After switching back to Projects, Cmd+4 selected B. Header title, terminal working directory and sidebar highlight agreed. No number shortcut targeted a pre-existing row. `10-projects-cmd5.png`, `11-active-cmd4.png`, `12-active-cmd3.png`, `13-projects-cmd4.png`. |
| Settings and rebinding | **Passed** | Keyboard shortcuts → Navigation and creation displayed Switch sidebar view, its expansion description and Cmd+.. Captured Cmd+Shift+Y; the saved override was `KeyY` with Meta and Shift. Back in Projects, Cmd+. left the view unchanged; Cmd+Shift+Y selected Active. Used the row's reset control, verified the override map returned to `{}`, then Cmd+. selected Projects again. `14-settings-default.png`, `15-settings-rebound.png`, `16-default-inactive-after-rebind.png`, `17-new-binding-active.png`, `18b-binding-restored.png`, `19-restored-default-projects.png`, `rebound-settings.json`. |
| Command palette | **Passed** | Cmd+K, search Switch sidebar view, Return selected Active from Projects. Collapsed Active, opened the palette again, searched and executed the same command: the sidebar expanded into Projects. `20-palette-command.png`, `21-palette-active.png`, `22-before-palette-collapsed.png`, `23-palette-collapsed-command.png`, `24-palette-expanded-projects.png`. |
| Windows Ctrl+. routing and Outside terminals | **Blocked** | No Windows host with native computer-use access is exposed in this session. Preflight asked once; Jason chose Blocked with `needs-live-test` retained. Runner first, Terminal first, negotiated CSI 46;5u, legacy dot, outside-terminal focus and the Outside terminals tag were not executed natively. Automated tests and Windows CI cannot replace this check. |

The coder separately reported 1,658 runner-app/runner-terminal Nextest tests passed, 5 skipped; workspace Clippy, updater-feature Clippy, formatting and diff checks passed. Those automated results include remote synchronization and Ctrl+. protocol coverage; QA did not rerun them or count them as native results. CI remains the coder's PR-stage gate.

### Rebase provenance

The native execution above tested `4bb3d39253d3d77ab35e364031898c2ddf73421c` on base `b841b51a63fae25426fd9a36b4da23b927213efa`; it is not a new native run on the later PR head. PR #1275 first used `759a8c13410384145ef9b25bfe0a9effe776e924`, whose only tree difference was this QA record, then rebased to `de29fcf6d4f27dcbc1a8db179856c59ba5e8d793` on `976cce4d4c58f84e676b5fd7b464e1437817e22d` after #1274 changed remote per-computer settings. In the Runner integration review at 2026-11-12 03:39:50 UTC, `reviewer-2` confirmed feature-patch equality and no additional native rerun: local shortcut behavior, Settings/palette and terminal encoding are unchanged, while the affected remote persistence is within the selected in-process coverage. The coder must complete the rebased automated checks and new CI. The macOS evidence remains attributed to its original binary; Windows remains Blocked and `needs-live-test` remains required.

## Execution notes

The QA lock was initially held by mission `01M76S2AK9YP5B0PP7D0HXJJ7Q`; QA waited in the prescribed bounded acquire and notified the coder. Build and launch began only after acquisition.

CUA's immediate same-call screenshots sometimes preceded the settled app update. Every claimed result was verified in a later screenshot; early input experiments are not product failures. Literal `super+.` was rejected by CUA, while the supported `super+period` produced the working Cmd+. combination. The first reset attempt landed on General because reopening Settings selects General; the saved override check caught that, and the actual per-row reset on Keyboard shortcuts then restored `{}`. `18-binding-restored.png` retains that intermediate General-page capture; `18b-binding-restored.png` is the verified reset evidence.

Owned fixtures: terminal A `01M76VF8RADJTMDRWF7KVJ2ZGF`, started through the native launcher in `/private/tmp/runner-1266-qa-gtysecjn/terminal-a`; terminal B `01M76VKF2MEBZN1RVQZYFQ44FM`, started through Cmd+T in the existing default directory, then immediately changed through native terminal input to `/private/tmp/runner-1266-qa-gtysecjn/terminal-b`. That shell ran only the harmless `cd` command. Both were renamed through the native header to QA1266 A/B. No test mission, role or crew was created and no global agent configuration was changed.

## Cleanup

**Complete.** Restored the original empty shortcut override map, Projects view and expanded sidebar through the UI. Both owned shells were stopped through the candidate's development CLI and disappeared from `session list` and the sidebar; shell exit closes its row automatically, so a subsequent archive of B returned `session not found` rather than leaving a stopped chat to archive. No pre-existing session was individually stopped, archived, renamed or typed into. `28-restored-cleanup.png` shows the neutral launcher with the owned rows gone.

Quit the QA wrapper, observed it absent from the native app inventory, stopped the development daemon started by this run (`daemon stop` returned `stopped: true`), and unregistered the wrapper from LaunchServices. As required by daemon cleanup, its automatically resumed development processes ended with it. The installed Runner app and daemon were untouched. All foreground build/app commands ended; `ps` showed only the pre-existing installed Runner app/daemon and no development app/daemon or owned shell. Released `script/qa-lock` successfully.

Keep them running during the reopen check changed only `lastQuitChoice` from `stop` to `keep`. After the app exited, restored that one JSON preference to the backed-up value; the final semantic comparison of every `ui-settings.json` key with `ui-settings-before.json` found **zero differences**. No Windows terminal-priority setting was touched. Wrapper binaries, scratch folders, screenshots and logs are retained; no repository branches, worktrees or evidence files were deleted.
