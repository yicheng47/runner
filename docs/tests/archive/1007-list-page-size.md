# 1007 — List page-size validation

Historical design recovery (2026-11-12): superseded desktop spec canvases referenced in this record are preserved in `YeelinksCo/yeelinks_design` at revision `4605d1e43ab6835a555fdd3957469a6b55ddaec8`, under `runner/desktop/specs/` (including `archive/`). Their original frame ids resolve there; current product designs are in `runner/desktop/runner.pen`.

Date: 2026-10-23. Scope: [Roles and Crews pagination](https://github.com/yicheng47/runner/blob/97fc5b901ba2fa45e970f1eaa44f04c876d1d4dd/docs/features/1007-list-page-size.md). Platform: macOS, local Rust CI profile.

Implementation automated-check candidate: then-uncommitted changes on `feat/1007-list-page-size`, based on `0696c9e1bb163052ecd2f90f40b7e9f3f5782010`. SHA-256 of `git diff -- crates/runner-app`: `31361d12392539ed5ee7ed61f0f90aa3d34c478b04aaaa9ece02ff440438725d`. The approved design is `runner/desktop/specs/1007-list-page-size.pen` in the private design repo, committed as `45c085e`: Roles `FQVg3`, Crews `vGI5F`, selector and narrow footer `A0tnE`. The reviewer's committed native candidate is identified below.

## Selected coverage

The change adds a size selector and persistent preferences and increases the number of rows inside the existing scrolling region. Checks therefore cover request sizes, page counts, stale requests, search preservation, scroll reset and overflow, footer layout, settings compatibility, remounting and synchronization between windows. The existing regression suite has no dedicated role/crew pagination case; unrelated chat, agent, mission and window-chrome cases are outside this feature's live scope.

## Automated results

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-app --profile ci` | Passed: 800 tests; 3 existing manual/runtime tests ignored | `/tmp/runner-1007-app-tests.log` |
| `cargo test --locked -p runner-app --profile ci page_size` | Passed: 4 tests, rerun after adding the cross-window assertion | `/tmp/runner-1007-page-size-tests.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed | `/tmp/runner-1007-clippy-retry.log` |
| `cargo fmt --all -- --check` and `git diff --check` | Passed | Exit 0 |
| Relative links in changed guide/spec pages | Passed | All targets exist |

The first Clippy attempt was blocked by the sandbox when the Metal compiler tried to write its module cache; the same command passed with cache access. The initial failure is retained in `/tmp/runner-1007-clippy.log`. The in-process GPUI tests operate disposable databases and test windows; they are not a native development-app pass.

## Native review

### Reviewer candidate and code review

Reviewer run: 2026-10-23, macOS 26.6.2 (25G83), Apple Silicon. Candidate `81aa30baa2bc05438196047f1f3b98f307650b70` on `feat/1007-list-page-size`; the working tree was clean before validation. Base: `0696c9e1bb163052ecd2f90f40b7e9f3f5782010`. Full branch diff SHA-256: `4b6c78f73217b7a8c81671a5fba98efb021cd97f089479917a595f324be79262`. Evidence root: `/private/tmp/runner-1007-native-20261023/`, including `candidate.txt`, `initial-git-status.txt`, `candidate.diff` and `os.txt`.

Read the complete 19-file branch diff, list controllers and callers, shared selector and footer, settings load/save and shared-store observation, daemon paging implementation, and added tests. Code review found no must-fix issue before native validation. Existing automated log results above were inspected and reused; unchanged broad checks were not rerun. The selected live scope remains LIST-SIZE-01–04 because the changed behavior is the selector, scrolling, footer layout and remembered choices.

Initial development settings had neither page-size key (effective defaults 10/10), app zoom 1.3, expanded sidebar width 230.17729. The initial settings were captured before launch. No development app or development daemon was running. Build/launch command: `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`, using this worktree's own `target/`; `CARGO_TARGET_DIR` was unset. Build output is `make-run.log` in the evidence root.

Per AGENTS.md, the reviewer performed these checks after code review was clean, using the development app and owned fixtures only. No live app was launched during implementation.

| Case | Actions and expected result | Status |
| --- | --- | --- |
| LIST-SIZE-01 | On Roles and Crews, verify the compact default `10 / page` control against the approved design, open its upward menu, and choose 25 and 50 by mouse and keyboard. The current choice is marked. | Passed on macOS |
| LIST-SIZE-02 | With more than 50 owned rows, page and search, then change size. The first page loads, the search and counts remain correct, and the table starts at the top. A zero-match search retains its count and size control. | Passed on macOS |
| LIST-SIZE-03 | Scroll a long page and resize from a wide window to the minimum width, including an enlarged app zoom. Rows scroll beneath the header; the footer stays visible and its count, pager and selector do not overlap. | Failed on macOS: middle-page pager clips both navigation arrows at 640 points / 130% zoom |
| LIST-SIZE-04 | Choose different sizes for Roles and Crews, revisit each list, open another development window, and relaunch the development app. Each list retains its own size and open windows synchronize changes. | Passed on macOS |

### Native setup and evidence

The worktree build completed successfully. Native control used `cua_repl` bound to `/private/tmp/runner-1007-native-20261023/Runner List Size Dev.app`, bundle ID `com.wycstudios.runner.qa1007`. The wrapper contains byte-identical copies of `target/debug/Runner` and `target/debug/runner-agent-cli`; SHA-256 values are `a997764339aafcf0c64c79028a577f4582ccd8acf407ddeaa89d1dfea3ac6f2d` and `b81b947158c8054c95164048a7ac3ca35cc17d0103f7136ab5d6628bcad5825f` respectively (`binary-hashes.json`). The initial bare app process from `make run` was stopped before wrapper launch. `status --json` from the absolute development CLI reported app `0.17.1 (dev)`, CLI `0.17.1`, and `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/runnerd.sock`; mission environment variables were omitted for wrapper launches and development CLI state/fixture commands. Endpoint evidence is `status.json`. Opening Roles was the harmless native-control identity check; no input was sent into existing chats.

Fixture command: `python3 /tmp/runner-1007-native-20261023/fixtures.py seed`. It used only development CLI `role create` and `crew create` calls, recording every result in `fixture-commands.ndjson` and `fixture-ledger.json`. Created exactly 61 roles (`qa1007-r-00` through `qa1007-r-60`, display names `QA1007 Role 00` through `QA1007 Role 60`) and 61 empty crews (`QA1007 Crew 00` through `QA1007 Crew 60`), with no test sessions or missions. Independent development CLI lists show 100 roles and 85 crews after seeding, versus 39 and 24 beforehand. Search `QA1007` therefore produces exactly 61 matches in each list.

LIST-SIZE-01: both lists initially displayed `10 / page`. Native menus opened upward, offered 10/25/50, and marked the current choice. Roles selected 25 by mouse and 50 with Down/Return; Crews selected 25 with Down/Return and 50 by mouse. The compact unboxed trigger was compared with the approved PNGs for `FQVg3`, `vGI5F` and `A0tnE`. Evidence: `roles-default-10.png`, `roles-menu-default.png`, `crews-default-10.png`, `crews-menu-default.png`, `crews-menu-25.png`, plus the size/result captures below.

LIST-SIZE-02: at size 25, both lists showed page 1 ending at fixture 24 and page 2 starting at fixture 25; page navigation reset scroll to the top. From scrolled page 2, changing to 50 returned to page 1 / fixture 00, preserving `QA1007` and counts `61 of 100 roles` / `61 of 85 crews`. Scrolling size 50 ended at fixture 49. A settled `QA1007 no-such-row` search showed the no-match state, `0 of 100 roles` / `0 of 85 crews`, and the size control. Evidence: `roles-25-scroll-bottom.png`, `roles-page2-top.png`, `roles-page2-bottom.png`, `roles-50-reset-top.png`, `roles-50-bottom.png`, `roles-zero-match-settled.png`, `crews-25-bottom.png`, `crews-page2-top.png`, `crews-50-reset-top.png`, `crews-50-bottom.png`, `crews-zero-match.png`. The earlier `roles-zero-match.png` capture occurred before the 200 ms debounce settled and is retained as an intermediate capture, not a no-match pass.

LIST-SIZE-03: wide windows at 100% and 130% zoom kept the header and footer fixed while rows scrolled. At the documented 640-point validation width (1280 screenshot pixels on the 2x display), the footer changed to count/selector above the pager. At 100% zoom it fit, including a middle page. At 130% zoom, page 1's pager extended into the page padding, and page 4's wider pager clipped both navigation arrows at the card edges. Reproduced on Roles with 61 filtered rows / 7 pages and Crews with 85 total rows / 9 pages. Evidence: `roles-640-zoom100-10.png`, `crews-640-zoom100.png`, `roles-640-zoom130-10.png`, `roles-640-zoom130-middle-page.png`, `crews-640-zoom130-middle-page.png`. Window height was 802 points. The long existing heading also consumed the Crews row area at that size and zoom (`crews-640-zoom130.png`); its regression attribution was not established, and it was not expanded into a separate header-redesign request.

LIST-SIZE-04: Roles 25 and Crews 50 survived route visits and were inherited by a second native window opened with Shift-Command-N. Changing Roles to 50 in the peer updated the already-open original Roles page. Changing Crews to 25 updated the peer's preference, and changing it back to 50 in the peer updated the original active Crews page. Roles remained independently 25. Closed only the newly created peer window, stopped the owned wrapper process and relaunched the same binary with the development daemon left running; native Roles showed 25 and Crews 50 after relaunch. Evidence: `roles-persist-25.png`, `crews-persist-50.png`, `peer-inherits-roles25.png`, `original-sync-roles50.png`, `peer-sync-crews25.png`, `original-sync-crews50.png`, `settings-before-relaunch.json`, `relaunch-roles25.png`, `relaunch-crews50.png`, `relaunch.log`.

Tool recovery: one screenshot was unavailable while the native View menu remained selected; its exposed Cancel action restored native control. One resize action overshot to 552 points and the subsequent old-coordinate click was rejected by native control. The window was explicitly resized to 640 points and observed again before the reported layout checks. These control attempts were not counted as product passes. No implementation changes or additional automated test runs were made to obtain any native result.

Windows native checks: Blocked on this local run because no native Windows surface was available. The operator's Windows CI is separate evidence and cannot establish a native UI pass.

### Must-fix finding

**P2 — Keep the narrow pager usable at enlarged zoom.** `crates/runner-app/src/ui/list.rs:752–757` centers the unchanged fixed-width `Pager` without making it fit the compact footer. At 640 points wide, 130% zoom, expanded sidebar width 230.17729, size 10 and page 4 of 7 or 9, the nine 28-pixel pager items plus gaps exceed the available card width. Previous and Next are clipped. Preserve visible, operable navigation at the supported narrow/enlarged-zoom combination and add coverage for the widest middle-page state; the current footer-bounds tests exercise 100% zoom and page 1 only. Reported through Runner during the run, with `roles-640-zoom130-middle-page.png`; the corresponding Crews reproduction is retained too. LIST-SIZE-03 remains Failed until a revised candidate passes an affected-case rerun.

### Cleanup and verdict

Cleanup command: `python3 /tmp/runner-1007-native-20261023/fixtures.py cleanup`. Deleted only the 122 ledger-owned role/crew fixtures through individual development CLI operations. The final role and crew JSON lists exactly equal their initial lists (39 roles / 24 crews), and no `QA1007` rows remain. No database files were opened or modified directly. Existing sessions and missions were neither operated nor archived. No test agent sessions were launched.

Restored effective page sizes to 10/10 and app zoom to 1.3 through native controls, restored the original window dimensions (3270 × 2328 screenshot pixels), then stopped the owned wrapper. Removed only the two test-added page-size keys from the development UI settings file while the wrapper was stopped; semantic JSON equality against the initial settings snapshot passed. Unregistered the wrapper with `lsregister -u`; its files and evidence remain under the evidence root. The development daemon and its automatically resumed preexisting sessions were left intact. Cleanup evidence: `final-roles.json`, `final-crews.json`, `cleanup-summary.json`, `restored-window.png` and `fixture-commands.ndjson`.

Overall: **one remaining must-fix issue**. LIST-SIZE-01, LIST-SIZE-02 and LIST-SIZE-04 Passed; LIST-SIZE-03 Failed. Automated checks remain green, but native acceptance is not complete. Only this validation record was edited by the reviewer; no implementation edits or Git writes. These feature-specific live cases should be considered for promotion when this record is archived at release.

## Revised pager candidate and affected-case rerun

R2, 2026-10-23: candidate HEAD remains `81aa30baa2bc05438196047f1f3b98f307650b70`, plus the implementation's uncommitted changes to `crates/runner-app/src/ui/list.rs`, `crates/runner-app/src/surfaces/roles/tests.rs` and `crates/runner-app/src/surfaces/crews/tests.rs`. SHA-256 of `git diff --binary -- crates/runner-app` is `7dc95c968b6728851cd3ae1a87ecc1bae081ad2e89d4b53021a1f23aced8279a`, verified before and after native validation. The only other dirty file was this existing reviewer validation record. Evidence root: `/private/tmp/runner-1007-native-r2-20261023/`; `candidate.txt`, `initial-git-status.txt`, `candidate-code.diff` and `candidate-full.diff` capture the candidate before testing. No Git writes were performed by the reviewer.

The reviewer read the complete three-file delta, the pager's render path and list callers, and both new bounds/navigation tests. Narrow footers now use at most three neighboring pages, matching the approved `A0tnE` frame; wide footers retain endpoints and ellipses. The zero-page, edge-page and clamping cases are covered by the compact-window unit test. Code re-review found no remaining must-fix issue before this native rerun.

### Focused automated evidence

The implementation supplied the following results; the reviewer inspected their logs and did not repeat unchanged broad checks.

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-app --profile ci list_pager_fits_narrow_zoomed_footers_and_navigates` | Passed: 2 tests | `/tmp/runner-1007-pager-green.log` |
| `cargo test --locked -p runner-app --profile ci page_window` | Passed: 2 tests | `/tmp/runner-1007-page-window.log` |
| `cargo test --locked -p runner-app --profile ci page_size` | Passed: 4 tests | `/tmp/runner-1007-page-size-fix.log` |
| `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater -- -D warnings` | Passed | `/tmp/runner-1007-pager-clippy.log` |
| `cargo fmt --all -- --check`; `git diff --check` | Passed; reviewer also reran diff-check after documenting this run | Exit 0 |

Both new integration tests first failed against the old pager: at width 640 and zoom 1.3 its 368.5-point bounds exceeded the 245.5-point footer (`/tmp/runner-1007-pager-red.log`). The first fixed-code attempt passed horizontal bounds but failed navigation because the existing heading layout put the synthetic test's buttons below its 802-point viewport (`/tmp/runner-1007-pager-short-window.log`). The final test uses height 1100, with the same widths 640/1100, zoom 100%/130%, sidebar width 230.17729 and page 4 of 7/9; it checks each button's bounds and Previous, Next and numbered-page navigation. Both failed attempts are preserved. Native validation below additionally exercises the original 802-point window height.

### Native setup and actual results

The macOS environment is unchanged from the first run. Before R2, no development app or daemon in the real development namespace was running. Initial development settings were sizes 10/10, zoom 1.3 and expanded sidebar width 311.83295; they were captured in `initial-ui-settings.json`. Build command: `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG make run`, using this worktree's own `target/` with `CARGO_TARGET_DIR` unset (`make-run.log`). Stopped only this run's bare development app, then launched `/private/tmp/runner-1007-native-r2-20261023/Runner List Size Dev R2.app`, bundle ID `com.wycstudios.runner.qa1007r2`, with the same environment omissions. The wrapper copies were byte-compared to this worktree's binaries. SHA-256: app `3b2a1a882760231c95dc7189c509045a9b87b06a1eb3fba188fa64f0a6f7fb83`; CLI sidecar `b81b947158c8054c95164048a7ac3ca35cc17d0103f7136ab5d6628bcad5825f` (`binary-hashes.json`).

The absolute development CLI's `status --json` again reported app `0.17.1 (dev)` and `com.wycstudios.runner-dev/runnerd.sock` (`status.json`). Native control bound to the distinct wrapper and opened Roles before further input. `python3 /tmp/runner-1007-native-r2-20261023/fixtures.py seed` created 61 owned roles named `QA1007R2 Role 00`–`60`, handles `qa1007r2-r-00`–`60`, and 61 empty crews named `QA1007R2 Crew 00`–`60`. No agents or sessions were created. Native sidebar dragging set the reproduction width to 230.00002, approximately the original 230.17729. The first divider drag did not change its width; a second drag succeeded. The initial app window was 3190 × 2074 screenshot pixels.

| Case / variant | Status | Actual outcome and evidence |
| --- | --- | --- |
| LIST-SIZE-03 / Roles / 640 × 802 points / 130% zoom / 10 per page / page 4 of 7 | Passed on macOS in R2 | Both arrows and pages 3/4/5 fit inside the footer. Previous selected 3, Next returned to 4, and clicking 5 selected 5. `roles-640-zoom130-page4.png`, `roles-640-zoom130-previous.png`, `roles-640-zoom130-next.png`, `roles-640-zoom130-number5.png`. |
| LIST-SIZE-03 / Crews / 640 × 802 points / 130% zoom / 10 per page / page 4 of 9 | Passed on macOS in R2 | Both arrows and pages 3/4/5 fit inside the footer; Previous, Next and page 5 worked. `crews-640-zoom130-page4.png`, `crews-640-zoom130-previous.png`, `crews-640-zoom130-next.png`, `crews-640-zoom130-number5.png`. The previously disclosed heading wrapping still consumes the row area at this short height; it was not changed or counted as a pager regression. |
| LIST-SIZE-03 / wide footer and long-page scrolling | Passed on macOS in R2 | Wide middle-page footers retain endpoints/ellipses. With 50 per page, both lists scrolled to fixture 49 while headers and footers stayed fixed. Narrow 100% footers and the narrow Roles 130% size menu also fit. Evidence: `roles-wide-page4.png`, `crews-wide-zoom130-page4.png`, `roles-narrow-size-menu.png`, `roles-640-zoom130-50-bottom.png`, `roles-640-zoom100-50-bottom.png`, `crews-wide-zoom130-50-bottom.png`, `crews-640-zoom100-50-bottom.png`. The Crews long-page capture uses a taller 1100-point window. |
| LIST-SIZE-01, LIST-SIZE-02, LIST-SIZE-04 | Passed in the first run; results reused | Their behavior is unchanged by the pager-only correction. No repeated persistence or peer-window run was required. |
| Windows native validation | Blocked locally | No native Windows surface is available. Windows CI remains separate from native UI evidence. |

R2 resolves the P2 pager finding. The original LIST-SIZE-03 attempt above remains **Failed**; the affected-case rerun is **Passed**. This is feature validation, not a full smoke or regression pass.

### R2 cleanup and final verdict

`python3 /tmp/runner-1007-native-r2-20261023/fixtures.py cleanup` individually deleted all 122 ledger-owned fixtures. Final role and crew JSON lists exactly equal the initial lists (39 roles and 24 crews), with no `QA1007R2` fixtures left. Native controls restored both sizes to 10, zoom to 1.3 and the window to 3190 × 2074 pixels. The attempted native sidebar restoration did not change its width; after stopping the owned wrapper, restored only `sidebarWidth` from the initial settings snapshot and verified complete semantic JSON equality. The wrapper was unregistered from LaunchServices; files and screenshots remain with the evidence. The development daemon and automatically resumed preexisting sessions were left intact. No existing session was operated or archived. Evidence: `fixture-commands.ndjson`, `fixture-ledger.json`, `final-roles.json`, `final-crews.json`, `settings-after-native-restore.json`, `restored-window-size.png` and `cleanup-summary.json`.

Final reviewer verdict for the identified R2 candidate: **NO REMAINING MUST-FIX ISSUES**. Required macOS feature cases have passed across the initial run and affected rerun; native Windows remains disclosed as Blocked. The reviewer edited only this validation record and leaves commits, pushes, PR updates and CI to the external operator.

## Regression coverage

Archived with 0.17.2 on 2026-10-24. Lasting native checks are maintained as `SET-LIST-01` through `SET-LIST-04` in the [settings regression cases](../regression/settings.md). Request invalidation, count math and settings serialization remain automated checks in the owning app tests. This promotion preserves the original failed attempt, successful affected rerun and Windows gap; it establishes no new native pass.
