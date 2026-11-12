# #1279 — Codex usage credits

## Authorization and checkout

Jason selected the codex trio for #1279 and approved option A on 2026-11-12. The approved trio was initially authorized to implement, review, run the selected development-app checks, commit, push and open a PR against `main`, including the spec, brief and QA record. The later scope change under Review and validation defers live QA while preserving implementation, review and PR authorization. Finish with an open PR and green required CI. No merge or release is authorized.

Use branch `feat/1279-codex-usage-credits` in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1279-codex-usage-credits`, created from `origin/main` at `a9a097b5`. Keep all Runner work in this worktree, with its own Cargo build outputs. Fold the preparation commit into the implementation history before the final push.

## Goal

Implement [#1279](https://github.com/yicheng47/runner/issues/1279) according to `docs/features/1279-codex-usage-credits.md`. Option A is approved: add **Credits remaining** as a separate right-aligned value row below Codex's usage windows in the popover and Settings → Usage. B is not selected.

Design: `runner/desktop/specs/1279-codex-usage-credits.pen` in `/Users/jason/repos/yeelinks/yeelinks_design`, revision `9adb911`. Approved frames: `OQ9oc` dark popover, `z0VAi` light, `tIls6` Settings, `HKlDU` card detail; state frames and rules are listed in the feature spec. Read only the relevant frames through Pencil. This approval supersedes the draft annotations on the canvas. The design checkout contains unrelated user edits; do not save or modify it. Preserve #1277's merged compact preview and transparent-at-rest footer styling; the older #1249 preview is obsolete.

Consume `rateLimits.credits` from the existing Codex RPC, not reset credits. Carry it additively through the shared snapshot, persistence and existing transport. Handle finite/fractional balances, zero, unlimited, unavailable and credits-only responses; preserve exact underlying precision, while rounding the displayed balance to the nearest whole credit with grouping per Jason’s later 2026-11-12 approval. Missing data is not zero. Per Jason’s 2026-11-12 live-QA correction, remove the inline refresh-failure warning. Preserve last-known amounts and their original age silently on failed refresh, replace them on successful refresh, and keep credits when expired quota windows are removed. Existing sidebar percentages and near-limit alerts stay quota-based. No purchase/redemption flow, new settings or polling loop. Update the English/Chinese chats-and-windows guide and locale strings.

## Review and validation

On 2026-11-12, Jason explicitly deferred live QA because it cannot run in this environment. This supersedes the original live-QA gate: complete implementation, automated checks and independent code review, then commit, push, open the PR and drive required CI green without waiting for live QA. Do not build or launch the development app for QA or request local QA access. CRED-01 through CRED-05 and the scoped SET-USAGE-04 subset below remain deferred/not run at Jason’s request; none is a live pass. Keep `needs-live-test` on the open PR. No merge or release is authorized.

Evidence before the scope change: independent code review of implementation candidate `1a774650d4c94b8d976b7e27b3cd80ed91d86448` found no remaining must-fix issues and named the outstanding live checks. QA performed prerequisite reads, macOS/Codex version and login-status checks, and a native computer-use inventory only. QA reported no candidate build, app/daemon launch, fixture, test session, preference/configuration change, scratch resource or acquired QA lock; no cleanup was needed. The live procedures below are retained for the deferred work, not as executed results.

Coder runs the spec's focused parser/cache/protocol/formatting tests, relevant core/daemon tests, runner-app tests, workspace Clippy, applicable updater-feature Clippy and formatting. Reviewer reviews the identified candidate and names the required live checks; neither coder nor reviewer launches the app. Follow the normal coder → reviewer → QA loop.

Required native platform is macOS. QA alone takes `script/qa-lock` and runs CRED-01 through CRED-05 from the spec: real account fetch/display in both surfaces; representative credit states; 5-hour filtering and manual refresh; failed-refresh retention/recovery; and dark/light, default/150% zoom, narrow/800×600 layout. Each check directly covers the new credit value or shared usage rendering. Use the spec's scoped SET-USAGE-04 subset; this is not a full smoke or remote-network run. The spec authorizes a temporary provider fixture through the existing development-only Codex command override, changed through UI and restored afterward; the real account check remains required. Confirm fixture prerequisites first and report missing prerequisites honestly; do not alter global authentication/configuration or add product debug controls. Record and restore every development preference changed for the checks.

QA appends candidate-specific outcomes, screenshots and cleanup to `docs/tests/runs/YYYY-MM-DD-mac.md`. Native Windows/Linux and two-host live tests are not selected; report that limitation separately from configured cross-platform CI. A failed or blocked required check is never a pass and retains `needs-live-test`.

The PR closes #1279 and records the review and QA evidence. Expected labels are `risk-medium`, `needs-live-test` until the selected required checks pass, and `reviewed` after independent review of the current head. This is not frontend-only; preserve any automatic `core-review` label. Do not use native subagents or add agents beyond the approved trio.
