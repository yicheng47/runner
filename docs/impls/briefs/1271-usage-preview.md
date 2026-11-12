# #1271 — Compact Usage preview

## Authorization and checkout

Jason approved option A, then explicitly selected the codex trio and asked to start this mission on 2026-11-12. The codex trio may implement, review, run the selected development-app checks, commit, push and open a PR against `main`, including the preparation spec/brief and QA evidence record. Finish with an open PR and green required CI; no merge or release is authorized.

Use the existing branch `fix/1271-usage-preview` and worktree `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1271-usage-preview`, created from `origin/main`. Keep all Runner work there. The root checkout stays on `main`. Follow the repository's crew, review, history and QA rules; fold this preparation commit into the implementation history before the final push.

## Goal

Implement [#1271](https://github.com/yicheng47/runner/issues/1271) using `docs/features/1271-usage-preview.md`. The approved design is option A in `runner/desktop/specs/1271-usage-preview.pen` in `/Users/jason/repos/yeelinks/yeelinks_design`, committed at `df4f78e`: screen `EXs4b`, card `yCqmc`, light `C5y7XD`, states `ar2VP`, shared footer `kgReh`, decisions `HGArk`. Read the approved design through Pencil. The design checkout contains user edits; do not save or modify it. B and C are not selected. The old recent-row preview in #1249 and issue node `MYiIi` are superseded by A's compact container, while footer parity remains required.

The Sidebar card begins with a compact row: “Sidebar preview” / “Weekly usage” on the left and the natural-size footer sample on the right. Remove the recent chat content and oversized preview. The sample is 240 px wide including 12 px padding at 100% zoom and stacks below its label only when necessary at narrow widths. Use shared layout/style for the preview and actual footer: transparent-at-rest usage control with its background appearing on hover, 8 px Host/Settings gap, 14 px Host icon and 16 px Settings icon. Preserve interactive hover/focus/popover behavior in the real footer and keep preview Host/Settings inert. Provider toggles, missing values, active-account data, remote dimming and older-host fallback remain correct. Keep host/gear anchors stable when badges disappear.

Main starting points: `crates/runner-app/src/surfaces/settings/usage.rs`, `surfaces/app_shell/sidebar.rs`, `surfaces/app_shell/usage.rs`, Host/Settings renderers, existing UI tests and locale strings. Update the English/Chinese chats-and-windows guide, the superseded #1249 preview paragraph, and the affected regression contract. Avoid unrelated refactoring and backend changes.

## Review and validation

Coder runs `runner-app` tests, workspace Clippy and formatting, with the applicable updater-feature pass; update existing focused assertions where their contract changes. Reviewer reviews the handed-off candidate, confirms the required live checks below and reports must-fix findings through Runner. QA alone takes `script/qa-lock`, builds and drives the reviewed development candidate through computer use, and follows the isolation/candidate/cleanup rules in `AGENTS.md` and `docs/tests/full-smoke-test.md`.

Required native platform: macOS. Run PREVIEW-01 through PREVIEW-04 in the feature spec: A layout; both/individual/no badges and inert preview controls; actual footer at rest/hover/popover-open plus Host and Settings actions; light/dark, 100%/150% zoom, narrow sidebar and 800×600 geometry. Each check directly covers changed layout or shared footer code. Restore development usage switches, theme/material, zoom and geometry after checking; preserve existing sessions. Use existing account data and mark a missing required prerequisite honestly. Do not broaden into a full smoke or agent-launch/auth/remote-network suite.

Jason’s post-QA correction on 2026-11-12 supersedes A’s filled-at-rest style and restores the previous transparent resting control while retaining the compact preview. Preserve the historical `SET-USAGE-02` and filled-at-rest `SET-USAGE-03` retired rows; `SET-USAGE-04` owns the corrected contract under the regression suite’s retire/new-ID policy. QA records per-check outcomes, candidate identity, screenshots and cleanup in `docs/tests/runs/YYYY-MM-DD-mac.md`; native Windows/Linux were not selected for this mission and must be reported as not run, not passed. Required CI still runs across configured platforms.

Follow the coder → reviewer → QA handoff loop and rerun affected checks after fixes. The PR should close #1271, describe A plus shared footer parity, and include the real QA verdict. Expected labels: `frontend-only`, `risk-medium`, `needs-live-test` until all selected required checks pass, and `reviewed` once the independent reviewer passes the final code head. Follow repository rules if the final diff changes those classifications. Do not use native subagents or add agents beyond this approved trio.
