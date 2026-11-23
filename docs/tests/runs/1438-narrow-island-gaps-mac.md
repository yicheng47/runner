# #1438 narrow island gaps macOS live QA

## Candidate and environment

On 2026-11-23, Jason assigned the five macOS checks below in Runner mission `01M842EBH1ZBFEED8GD3PBHRKM`. QA tested reviewed commit `5cab9f7092a26ac7fa96f186c64bb4d8864a6929` from `fix/narrow-island-gaps` in its named worktree. The worktree was clean on entry and through the run. The approved scope was the island spacing, affected resize handles, overlay transition and terminal clipping.

macOS 26.6.2 (25G83), arm64, 2× Retina, Runner 0.20.3 (dev), CLI 0.20.3. QA acquired `script/qa-lock acquire --timeout 540` before `make run`. After launching the bare candidate, QA stopped that process and used `script/screenshot-app bundle` and `launch` with `/tmp/runner-qa-1438.Z1TXvH`. The helper compared the wrapper and worktree binaries byte for byte. Their Runner SHA-256 is `3ac9e1574e8f514d08b6317d7d71aa4f6080a6bcd843349a566dece82227281e` and CLI SHA-256 is `a06afe6546e7dde2c0550e34c77ccc23bbf597a0ec52ee4e0e92907a0528d271`. The development CLI confirmed the `com.wycstudios.runner-dev/runnerd.sock` endpoint before UI input. See `artifacts.sha256`, `make-run.log` and `endpoint.json` in the scratch root.

UI actions and full-resolution screenshots used Computer Use against the temporary development wrapper. Real-cursor drags used `mac-drag` with Retina coordinates. Window screenshots, cursor-inclusive crops, and the drag JSON are under `/tmp/runner-qa-1438.Z1TXvH/evidence/`. Measurements use physical screenshot pixels at 2×, with rounding and antialiased borders. The fixture used one Codex 0.171.0 chat (GPT-6 Luna, low effort, zero prompts), a plain zsh split pane, and an empty mission with a zsh drawer. Zoom was changed via the app shortcuts. QA ran no automated checks in this live session. The [PR test report](https://github.com/yicheng47/runner/pull/1438) lists the candidate's automated results.

## Required results

Four of five checks passed. The column-gutter cursor observation below leaves check 2 unresolved and keeps `needs-live-test` on the PR.

| Check | Result | Observation and screenshot evidence |
| --- | --- | --- |
| 1. Gap geometry at 100% and 130% (WS-CARD-04 subset) | Passed | The sidebar, split panes, docked Files panel, mission rail and drawer gaps were consistent with 4 logical px at 100% and 5.2 logical px at 130%. Outer insets, including the collapsed left edge, were consistent with 8 px before zoom. Evidence: `chat-geometry-{100,130}.png`, `chat-geometry-sidebar-collapsed-{100,130}.png`, `mission-geometry-{100,130}.png`, `mission-geometry-sidebar-collapsed-{100,130}.png`. |
| 2. In-flow gutter drags at 100% and 60% (WS-CARD-03 subset) | Failed | Both split directions and the drawer resized from the gap centers at both zooms, including the 2.4 px targets at 60%. Divider movement matched the drag within screenshot rounding, with no visible jump at mouse-down. One 100% column drag showed the standard pointer before release. A slower repeat showed the resize cursor both while moving and held. See the finding below and the `split-columns-{100,060}-*.png`, `split-rows-{100,060}-*.png` and `mission-drawer-{100,060}-*.png` series. |
| 3. Edge splitter drags at 100% (WS-CARD-03 subset) | Passed | Sidebar, chat panel and mission rail resized by approximately 50, 50 and 60 logical px, respectively. Cursor-inclusive mouse-down and held crops show their island edges within approximately 1 logical px of the pointer. Evidence: `edge-sidebar-100-*.png`, `edge-chat-panel-100-*.png`, `edge-mission-rail-100-*.png`. |
| 4. Docked compact panel to overlay sheet | Passed | In a 1676 logical px window at 100%, opening `qa-1438.md` widened the docked Files panel into a right-aligned overlay. The dimmed split divider remained at the same x position within 1 physical px, the sheet covered the reserved right-hand area with the same right inset, and the PTY stayed at 34 columns × 23 rows without a resize signal. Closing the file restored the compact dock with the same pane bounds. Evidence: `overlay-compact-before.png`, `overlay-file-sheet.png`, `overlay-compact-restored.png`, `overlay-pty-at-open.log`. |
| 5. Explicit terminal background and selection (WS-CARD-04 subset) | Passed | Vim painted an explicit truecolor background in a narrowed pane. Two drags extended past the pane edges. The highlight changed terminal cells only, and sampled adjacent gutter strips had zero changed pixels. Evidence: `tui-background-100.png`, `tui-selection-100.png`, `tui-selection-right-100.png`. |

## Unresolved finding

The column gutter cursor reverted to the standard pointer in one capture before mouse-up. At 100% zoom with the sidebar and compact Files panel open, split an owned chat to the right and fill that pane with a plain terminal. Drag the center of the gap 65 logical px right. The failing attempt used 12 mouse-move steps at 20 ms intervals and sampled the new gap while the pointer was still held, approximately 1.47 seconds before the drag command returned. The resize cursor appeared at hover and mouse-down. The actual held crop shows the standard pointer over the shifted gap, and the after shot confirms that the panes resized. The expected cursor while dragging is the horizontal resize cursor. The affected surface renders the gutter in [panes/tree.rs](../../../crates/runner-app/src/surfaces/panes/tree.rs) using [ui/resize.rs](../../../crates/runner-app/src/ui/resize.rs).

Evidence: `split-columns-100-hover.png`, `split-columns-100-mouse-down.png`, `split-columns-100-held.png`, `split-columns-100-after.png` and `split-columns-100.json`. A later 60 logical px drag with 30 steps at 33 ms showed resize cursors in `split-columns-100-steady-moving.png` and `split-columns-100-steady-held.png`. The initial observation remains open. Frequency and cause are unestablished, and no baseline comparison was run.

Computer Use intermittently returned an app-resolution error immediately after a real-cursor drag. Fresh wrapper snapshots confirmed that the app stayed running and provided the after evidence. The error did not prevent any selected observation.

## Cleanup

QA stopped the two ephemeral shell sessions `01M84346A49F5TYVYJ2K7MMKA4` and `01M843EC873XV86T7WWW5H5MTG`. They disappeared from the development CLI session list after stopping. The direct-chat archive command reported that these ephemeral shells had no session to archive. QA stopped and archived the Codex chat `01M8432VVZRCR03NMA8YQCZ26G`, then unpinned, stopped and archived mission `01M8432VXDNPX0EHY0RD950AK3`, which showed zero live sessions. The CLI ledger and evidence are retained under the scratch root.

QA quit the wrapper, stopped the development daemon, verified its socket was gone, unregistered the wrapper, and restored `ui-settings.json`, `window-state.json` and `window-layout.json` from byte-identical backups. The restored `appZoom` was 1.4. QA released the lock. The installed Runner process remained active.
