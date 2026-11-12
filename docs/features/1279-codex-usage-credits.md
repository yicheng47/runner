# 1279 — Codex usage credits

Tracking issue: [#1279](https://github.com/yicheng47/runner/issues/1279).

## Design decision

Jason approved option A, the separate **Credits remaining** row, on 2026-11-12 after selecting the codex trio for this work. Design: `runner/desktop/specs/1279-codex-usage-credits.pen` in the private `YeelinksCo/yeelinks_design` repo, revision `9adb911`. Approved frames: dark popover `OQ9oc`, light popover `z0VAi`, Settings screen `tIls6`, Codex card detail `HKlDU`. State frames: zero `H1ONQ`, unlimited `d9EJW`, unavailable `J6110`, 5-hour limit hidden `Y3rX63`, refresh failure `dW8aR`, credits without windows `iMNw4`. Option B is not selected. This recorded approval supersedes the draft/awaiting-sign-off annotations still on the comparison canvas.

On 2026-11-12, during live QA, Jason requested removal of the inline refresh-failure warning. Retain the last known values and their original timestamps silently; this supersedes the warning shown in frame `dW8aR` and the earlier failure-message expectations below.

Later on 2026-11-12, Jason approved rounding the displayed balance to the nearest whole credit with grouping: `54,135.9439300000` becomes `54,136`. Halfway values round away from zero (`1,250.5` becomes `1,251`). Preserve the exact provider balance in snapshots, cache and transports; only the shared display formatter rounds it. This supersedes the original fractional-display requirement. The separate-row layout and the approved whole-number design example remain unchanged.

The corrected Settings preview follows #1271's compact “Sidebar preview / Weekly usage” row with a 240 px footer sample and transparent-at-rest usage control. #1277 is merged into the starting base. Preserve that implementation; do not copy the obsolete large recent-chat preview from #1249 or redesign the footer.

## Motivation

Runner currently shows Codex's 5-hour and weekly quotas but drops the credit information returned alongside them. A person should see the available credit balance from either existing Usage surface without leaving Runner or mistaking a missing balance for zero.

## Scope and behavior

Add one labelled credit row below Codex's quota windows in the usage popover and Settings → Usage, separated as option A shows. The label is **Credits remaining** and the value is right-aligned. It is a credit balance, with no currency symbol, usage bar, percentage, or reset timer. Keep the row visible when the 5-hour window is hidden and when valid credit data is returned without quota windows. Use the existing shared usage rendering path where practical. Apply the same display contract wherever that shared Codex section is rendered, including the read-only remote usage view, without expanding remote controls or capabilities.

Read nullable `rateLimits.credits` from the existing `account/rateLimits/read` RPC. The installed Codex CLI 0.167.2 schema exposes `balance` (nullable string), `hasCredits` (boolean), and `unlimited` (boolean). This is different from `rateLimitResetCredits`, which is outside scope. [Official documentation](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt).

| Returned state | Display |
| --- | --- |
| `unlimited: true` | `Unlimited`; this takes precedence over a balance. |
| Finite reported balance | The amount rounded to the nearest whole credit with readable grouping; example `1,250.49` → `1,250`, `1,250.5` → `1,251`. Do not convert it to money or round the stored provider value. |
| Explicit zero balance, or explicit no credits with no balance | `0`. |
| Absent/null credit snapshot, or credits available but balance absent | `Unavailable`, in secondary text. Missing data alone never means zero. |
| Initial whole-usage fetch still pending | Preserve the existing checking/loading feedback; do not flash a zero balance. |
| Fetch fails after a successful result | Keep the last known amount and its original age without an inline warning. Do not present a retained amount as freshly fetched. |

A successful response that explicitly lacks credit data replaces the prior credit state with unavailable; it is not a fetch failure. Keep account/provider isolation intact, and keep Claude Code free of Codex credit rows. A credits-only result is usable even with no quota windows; a response with neither usable windows nor credit data must retain appropriate unavailable/error behavior.

Use existing open/manual/scheduled refreshes and last-known-value persistence. Credit balances do not reset on the quota windows' timestamps: restoring a cached credits-only snapshot must not drop it merely because all quota windows expired. Extend the shared snapshot additively, with defaults for old serialized snapshots and old peer payloads; preserve existing local and companion transport behavior. Do not introduce a database schema migration, new request or separate polling loop unless the implementation demonstrates it is necessary.

Sidebar percentages remain weekly quota usage. Existing badge switches, interval controls, near-limit alerts, Claude account selection, transparent-at-rest footer styling and the #1271 preview behavior remain intact. Credits do not enter percentage-threshold alerts.

No credit purchase/redemption flow, spending history, new alerts, new preferences, or new provider integration. Do not add production debug controls to exercise states.

## Implementation phases

1. Extend the usage value and Codex adapter to carry optional credit state through fetching, in-memory snapshots, persisted last-known values and existing serialization. Review the empty-window assumptions in `usage::unexpired` and the Codex parser.
2. Add option A to the shared Codex usage presentation, with localized labels, value formatting, loading/unavailable handling and honest stale/error feedback. Preserve existing behavior for other providers and old snapshots.
3. Add focused automated coverage, update the affected English and Chinese guide pages, then complete independent review and the selected native checks on the reviewed candidate.

Starting points: `crates/runner-core/src/protocol/usage.rs`, `crates/runner-daemon/src/runtimes/codex/usage.rs`, `crates/runner-daemon/src/usage.rs`, usage persistence in `db/`, and `crates/runner-app/src/surfaces/{app_shell,settings,remote_client}/usage.rs`. Trace actual consumers before editing. Guide pages: `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`.

## Verification

Current validation decision (2026-11-12): Jason later superseded the historical deferral below and requested native QA. The local whole-credit display and warning-removal follow-up passed its selected real-account/fixture/failure/layout rerun and required automated checks. Initial empty-cache Checking remains an unobserved coverage limit; Jason subsequently reported his live test passed and authorized merge once current-head CI passes. This human acceptance closes the live-test gate with that limit disclosed; it does not establish an empty-cache native pass. The run record preserves candidate identities and earlier failures/blocked attempts.

On 2026-11-12, Jason explicitly deferred live QA because it cannot run in this environment. This supersedes the original live-QA gate: complete implementation, automated checks and independent code review, then commit, push, open the PR and drive required CI green without waiting for live QA. Do not build or launch the development app for QA or request local QA access. CRED-01 through CRED-05 and the scoped SET-USAGE-04 subset below remain deferred/not run at Jason’s request; none is a live pass. Keep `needs-live-test` on the open PR. No merge or release is authorized.

Evidence before the scope change: independent code review of implementation candidate `1a774650d4c94b8d976b7e27b3cd80ed91d86448` found no remaining must-fix issues and named the outstanding live checks. QA performed prerequisite reads, macOS/Codex version and login-status checks, and a native computer-use inventory only. QA reported no candidate build, app/daemon launch, fixture, test session, preference/configuration change, scratch resource or acquired QA lock; no cleanup was needed. The live procedures below are retained for the deferred work, not as executed results.

Before running checks, list the changed behaviors and the checks selected for them. This is a feature pass, not a full regression run.

Automated coverage must exercise finite and fractional balances, whole-credit rounding down/up, halfway and carry/grouping cases, large integer precision, zero, unlimited, omitted/null credits, missing balance, credits-only answers, and existing window-only answers. Cover additive protocol/cache compatibility, failed-refresh retention and age, successful refresh clearing previously available credit data, cache restore after quota expiration, and unchanged quota-only percentage/notification behavior. Include focused shared-renderer/value-formatting coverage for the credit states and provider isolation. Run relevant `runner-core` and `runner-daemon` tests plus `runner-app` tests, workspace Clippy, the applicable updater-feature Clippy pass, formatting and normal required CI.

Required native platform: macOS. QA alone drives the reviewed development candidate through computer use after acquiring `script/qa-lock`. Follow `AGENTS.md` and `docs/tests/full-smoke-test.md` for candidate identity, configuration isolation and cleanup. The current signed-in account is used read-only; no purchase, credit redemption, account switch or sign-out is needed. State fixtures must be identified as fixtures and exercised through an existing isolated provider boundary and the real UI; they do not replace the live account integration check. A missing safe fixture is reported honestly rather than bypassing the UI or changing global credentials/configuration.

| ID | Selected check and reason |
| --- | --- |
| CRED-01 | Open the popover and Settings → Usage with the existing Codex account. Verify the same reported balance/state in the separate row, matching option A; compare with the provider response without exposing credentials. This covers the real fetch-to-display path. |
| CRED-02 | Verify finite/fractional, zero, unlimited, unavailable and credits-only presentation in both surfaces using safe isolated fixtures where the account cannot supply them. Confirm no percentage/bar/reset treatment and no credit row under Claude Code. This covers the newly introduced states. |
| CRED-03 | Hide and restore the 5-hour limit, and manually refresh. Credits remain visible, shared values update, weekly percentages remain quota percentages, and Claude usage remains intact. This covers window filtering and shared rendering. |
| CRED-04 | After a successful value, exercise an isolated provider failure and recovery. The last balance and original timestamp survive the failure without an inline warning, then recovery replaces them. This covers the new cached amount and its freshness contract. |
| CRED-05 | Inspect both surfaces in dark/light, at default and 150% zoom, with a narrow window including 800×600. The amount and status labels fit, the bottom row remains reachable, and the popover can be dismissed. This covers the added height and value width. |

Select only the related parts of `SET-USAGE-04`: preserved compact preview/resting style, matching weekly values, and readable/reachable lower popover rows, naming this as a scoped subset rather than a full case pass. Unchanged Host actions, session/agent launch, IME, auth and remote-network workflows are outside scope. Remote payload compatibility is covered by focused automated tests; native Windows/Linux and two-host live checks are not selected unless the actual diff exposes a concrete additional risk.

For CRED-02 and CRED-04, a temporary Codex-compatible executable under `/tmp` may provide controlled rate-limit responses through the existing development Settings → Agents → Codex command override. Record the prior override, select the fixture through the app UI, and restore it afterward. The fixture must support the normal version probe and app-server handshake; do not start chats or missions through it. Exercise failures at that provider boundary, without changing the machine network, global Codex configuration, credentials or Runner database. This is isolated fixture evidence; CRED-01 still requires the real signed-in Codex path.

Any development preference changed for QA (5-hour visibility, theme, zoom, geometry, or a provider override used by an existing safe fixture) must be recorded first and restored. Keep evidence outside Git and append candidate-specific results and screenshot paths to `docs/tests/runs/YYYY-MM-DD-mac.md`. A required blocked/failed check keeps `needs-live-test`. Expected PR risk is medium; because daemon/shared protocol behavior changes, it is not `frontend-only`. Preserve any automatic `core-review` label, add `reviewed` only for the independently approved current head, and leave the PR open for Jason.
