# 1271 — Compact Usage preview and shared footer styling

Tracking issue: [#1271](https://github.com/yicheng47/runner/issues/1271).

Design: `runner/desktop/specs/1271-usage-preview.pen` in `YeelinksCo/yeelinks_design`, option A approved by Jason on 2026-11-12 and pushed at `df4f78e`. Approved screen `EXs4b`, Sidebar card `yCqmc`, light card `C5y7XD`, response states `ar2VP`, shared footer `kgReh`, decision notes `HGArk`. B and C are unselected. This spec supersedes #1249's faded recent-row preview and the issue's reference to `MYiIi` for the preview container; the issue's footer parity requirement remains.

Jason revised the footer styling after native QA on 2026-11-12: “I want the previous style, the button style shows only when hover.” This supersedes option A’s filled-at-rest usage pill; the approved compact container, shared rendering and control geometry remain. The design checkout contains user edits and is not modified for this correction. The first QA record retains the earlier filled-at-rest candidate’s actual result.

## Motivation

Settings → Usage currently centers a narrow sidebar fragment in a wide preview panel, with a recent chat title above it. The preview also has a filled usage pill, 8 px Host/Settings gap and 14 px Host icon that the real footer does not share. A compact preview row should explain the result of the badge settings without unrelated chat content, and the sample must look like the actual footer.

## Scope

- Put the preview at the top of the existing Sidebar settings card, above its switches and separated from them by a divider. On the left, show “Sidebar preview” and “Weekly usage”; on the right, show the footer sample on the sidebar background. Remove the recent chat row, its divider and the large standalone preview area.
- At normal zoom, the sample is 240 px wide with 12 px padding, a 216 px inner footer and 32 px controls. The compact row uses 16 px vertical and 20 px horizontal padding. Follow the approved A card and the app's settings primitives for remaining styling. Scale with app zoom. Keep the sample at its natural size; let the row stack when available width cannot contain the label and sample without clipping. At narrow window widths and enlarged zoom, reduce the Usage page’s outer gutters as necessary to retain the full-size sample and the row’s padding.
- Render the preview and real footer through shared styling/layout. Both have a transparent-at-rest usage control with its button background shown on hover, 8 px between Host and Settings, a 14 px Host icon and a 16 px Settings icon. Keep the border space transparent at rest so hover does not shift the contents. Remove preview-only overrides that cause drift. Keep the actual footer's hover, focus, click and popover-open behavior and its meaningful visual feedback.
- Preserve existing provider eligibility and order, usage data, active Claude Code account and missing-data placeholders. The design's percentages are examples. Toggling a provider updates both the sample and the real footer; turning all usage off hides the badges and disables/dims provider switches, while keeping Host and Settings anchored at the right. The sample's Host and Settings controls are inert.
- Keep the existing Limits section, refresh behavior, settings persistence, notifications and remote data-selection behavior. Shared footer changes must preserve remote dimming and the older-host usage hint. No daemon, protocol or storage changes are intended.
- Localize new or revised copy in English and Simplified Chinese. Update `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md` to describe the compact preview and its behavior. Correct the existing #1249 preview paragraph with a pointer to this superseding design.

## Implementation phases

1. Share footer appearance and layout using existing helpers, implement A and update copy and guides.
2. Adjust focused in-process UI coverage, run the required automated gates, and obtain a clean code review.
3. Run the selected native checks on the reviewed candidate, record evidence, resolve findings through review and affected QA, then open the PR and drive CI green.

## Verification

Automated: run `runner-app` tests, workspace Clippy and formatting checks, including the applicable updater-feature Clippy pass under the repo's normal commands. Update existing footer/usage geometry or rendering assertions where their contract changes; cover missing data, one/both/no badges and shared helper callers without introducing a new testing framework. Required CI remains in scope.

Native checks are required on macOS for this mission because the change alters visible layout and footer controls. Windows/Linux share the UI implementation and must remain build-compatible; report their native checks as not run, without claiming a cross-platform live pass. Follow `AGENTS.md` Test Scope and `docs/tests/full-smoke-test.md` for candidate identity, isolation and cleanup. Only QA launches the app, after acquiring the shared QA lock. Back up and restore the development app's usage visibility settings, theme/material, app zoom and window/sidebar geometry if changed. Do not operate existing sessions.

| Check | Behavior and evidence | Why selected |
| --- | --- | --- |
| PREVIEW-01 | At default size and 100% zoom, verify the A row in the Sidebar card, no recent chat title, full-size sample, readable labels, proper divider and unchanged surrounding Limits layout. Compare screenshots against `EXs4b` and `yCqmc`. | The preview's layout is being replaced. |
| PREVIEW-02 | Compare the real footer and sample with both agents, each agent alone, and usage off. Verify weekly values/marks match, the sample updates immediately, provider rows disable/dim when off, controls remain right-aligned and preview Host/Settings clicks do nothing. | Shared rendering and toggle-dependent geometry change. |
| PREVIEW-03 | Inspect the real footer at rest, hovered and with usage popover open. Verify the transparent resting control and shared hover treatment, Host/Settings spacing and icon sizes, usage open/dismiss, Host popup open/dismiss and Settings navigation. | Footer appearance changes around existing interactive controls. |
| PREVIEW-04 | Repeat the compact preview and footer geometry in light/dark, at 100% and 150% app zoom, including a narrow sidebar and an 800×600 window. Labels, sample, toggles and gear stay visible and operable; narrow layout stacks rather than squeezing the sample. Capture representative screenshots. | Fixed sample dimensions and new row layout can clip when zoomed or narrow. |

Use the earlier `SET-USAGE` cases as sources for directly affected footer/zoom/popover checks, not as authorization to expand into unrelated provider/account behavior. `SET-USAGE-02` remains retired for its superseded provider/preview contract; the original filled-at-rest `SET-USAGE-03` is also retired after Jason’s correction, and `SET-USAGE-04` owns the compact preview with transparent resting controls. Preserve the historical rows and run results. No full smoke, account switching, authentication, agent launch, daemon lifecycle or remote-network exercise is required by this presentation change.

QA appends a candidate-specific result block to `docs/tests/runs/YYYY-MM-DD-mac.md`, with pass/fail/blocked per selected check, screenshot paths and cleanup. The PR description names the exact checked candidate, results and platform gaps; `needs-live-test` remains while any selected required check has not passed.
