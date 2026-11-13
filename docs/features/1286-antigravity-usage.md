# 1286 — Antigravity CLI usage indicator

Tracking issue: [#1286](https://github.com/yicheng47/runner/issues/1286).

## Design decision

Jason approved **Option F1** (tighter 3-badge sidebar footer with highest-usage + `+N` overflow collapse) and **Option P1** (Antigravity CLI pool subheadings `Gemini Models` and `Claude and GPT models` with standard `76 px` quota rows), signed off on 2026-11-13 (`needs-design` removed on `#1286`).

Design source: `runner/desktop/specs/1286-antigravity-usage.pen` in the private `YeelinksCo/yeelinks_design` repo, revision `a2321c8`. Approved frames:

- **Sidebar footer & preview (Option F1):** `QSiVA` (comparison and narrow/3-digit states), `o78yO7` (`cmp/UsageFooter`), `N4yXC` (`cmp/SidebarC` footer `TrVD6`), `asuJW` and `FFibS` (Settings compact `Sidebar preview` states).
- **Usage popover & Settings (Option P1):** `F288uj` (`V08Bl` Dark, `Gc538` Light), `Z2MEX` (full chat screen with popover `qJV6T`), `mXc9d` (Settings → Usage Dark), `CpeG5` (Settings → Usage Light), `HLn8E` / `fKPvG` (680 px Limits card detail), `uSsDR` (Settings → Notifications usage alerts).
- **Edge states & notes:** `vx7Wn` (`aGtDg` 5-hour hidden with Codex credits preserved, `NIGUI` signed-out calm instruction, `weQu4` silent refresh-failure retention matching `#1282`, `d6f7Vn` CLI error with no LKG), `syztz` (`Design notes · Approved (2026-11-13)`).

Option P2 (flat prefixed rows) and the other footer options are not selected. The signed-off design reflects shipped [#1279](https://github.com/yicheng47/runner/issues/1279) / [#1282](https://github.com/yicheng47/runner/issues/1282) Codex **Credits remaining** and silent refresh-failure LKG retention.

## Motivation

Runner dropped Antigravity CLI usage in [#1163](https://github.com/yicheng47/runner/pull/1181) (`1f107b5a`) because running `agy -p /usage --output-format json` blindly caused a signed-out `agy` CLI to start a new session and open an unrequested browser OAuth login window during startup or background polling.

People using Antigravity CLI alongside Claude Code and Codex should see Antigravity's dual-pool usage (`Gemini Models` and `Claude and GPT models`) across the footer badge, Usage popover, Settings → Usage, remote read-only usage and near-limit notifications, while guaranteeing that signed-out or expired `agy` installations never trigger browser login automatically.

## Scope and behavior

### 1. Non-interactive zero-login sign-in gate and daemon usage fetch

- **Hard invariant — never trigger browser login:** Usage probing on startup, open, schedule, or manual refresh must NEVER launch an interactive sign-in flow or open a browser OAuth URL when `agy` is signed out or missing credentials.
- **Verify probe in Phase 1 before wiring polling:** Before enabling background polling, inspect the installed `agy` CLI behavior (currently `1.5.8` stores auth in OS keyrings — macOS Keychain / Windows Credential Manager / Linux Secret Service — or `~/.gemini` state) in an isolated signed-out environment to establish a 100%-non-interactive sign-in/auth check across supported platforms, and fail closed when `agy --version` is unknown or below the verified minimum (`1.5.8`).
- If the non-interactive preflight indicates signed-out or unauthenticated state (or if the probe explicitly reports unauthenticated without starting OAuth), return `UnavailableReason::SignIn` immediately without calling a session-starting command.
- Only when signed in, read Antigravity usage non-interactively (matching `agy -p /usage --output-format json` or an equally non-login-opening `agy` interface verified not to spawn browser OAuth) and parse the `usage` command JSON pools (`Gemini Models` and `Claude and GPT models`, mapping `5h` to `5 hours` and `weekly` to `Week`).
- Missing answer / CLI error / timeout maps cleanly to an Antigravity CLI unavailable error (`AntigravityNoAnswer` or equivalent); invalid/empty JSON maps to `InvalidResponse`. Never log or surface raw OAuth tokens or secrets in diagnostics, UI, or companion payloads.
- Restore `RuntimeCapabilities { usage: true, .. }` for Antigravity in `runtime_metadata.rs` and `UsageSource` on `Antigravity`'s `RuntimeAdapter`, reusing existing `UsageService` scheduling, manual refresh, and SQLite last-known-good (`UsageLkg`) persistence with no DB schema migration or separate polling timer. Keep remote usage compatibility (`UsageSnapshot` and `runner-core/src/companion/` actions) additive and backward-compatible.

### 2. Sidebar footer and compact preview (Option F1)

- Add `antigravity: bool` (default `true`, backward-compatible serde default for older `ui-settings.json` files lacking the key) to `UsageSettings` and honor `badge(Runtime::Antigravity)` and `checks(Runtime::Antigravity, alert)`.
- **Sidebar weekly % metric:** Antigravity's sidebar footer badge displays the **highest weekly `used_percent` across its active weekly pools** (`Gemini Models` and `Claude and GPT models`, e.g., showing `82%` if Gemini is `48%` and Claude/GPT is `82%`), using the existing warning (`>= 80%`) and danger (`>= 100%`) color thresholds.
- **Option F1 spacing:** Tighten the shared `sidebar_usage_element` pill (rendered in both the real sidebar footer and Settings → Usage's compact 240 px `Sidebar preview`):
  - `6 px` horizontal padding, `5 px` badge gap, `2 px` icon-to-`%` gap (preserving transparent resting background, hover highlight, and `8 px` gap to Host & Settings buttons).
  - All 3 enabled badges (`Claude Code`, `Codex`, `Antigravity CLI` with typical 2-digit `%` values) fit in one row inside the `240 px` default sidebar and `240 px` Settings preview without clipping or pushing Host/Settings.
- **Overflow `+N` collapse:** Measure rendered/estimated badge widths against the available footer width rather than using a fixed multi-badge constant. When enabled badges exceed available space (such as the `200 px` minimum sidebar width or three wide 3-digit values like `100% × 3`), keep the **highest-usage** badges that fit (breaking ties deterministically by `Runtime::ALL` / visible order) followed by a compact `+N` overflow badge (e.g. `100% 100% +1` or `82% +2`). Missing (`—`) values sort after numeric values. Clicking the pill still opens the full Usage popover showing all enabled agents.

### 3. Usage popover, Settings → Usage, and Notifications (Option P1)

- **Option P1 pool subheadings:** In `usage_section_with_account` (popover, Settings → Usage card, and read-only remote popover), restore `usage_groups: true` for Antigravity so pools render with muted subheadings (`Gemini Models`, `Claude and GPT models`) above standard `76 px`-label `5 hours` and `Week` rows, preserving horizontal alignment of the `80 px` progress bars and `84 px` reset labels across Claude Code, Codex, and Antigravity.
- **5-hour window filtering across pools:** Update `is_five_hour` so hiding `5-hour limit` in Settings → Usage suppresses every pool's 5-hour window (`Gemini Models · 5 hours used`, `Claude and GPT models · 5 hours used`) across popover, Settings card, and near-limit alerts while keeping weekly pool rows intact.
- **Shipped `#1282` behavior preserved:** Keep Codex's **Credits remaining** row unaffected under Codex, and retain **silent failure LKG behavior**: when prior `AgentUsage` windows exist and a subsequent refresh fails, keep the cached windows and original `updated_at` timestamp silently with no inline error banner.
- **Signed-out and error states (no LKG):**
  - Signed out (`UnavailableReason::SignIn`): Antigravity section renders `"Sign in to Antigravity CLI to see usage"` plus the calm secondary hint `"Run agy login in a terminal. Runner won't open login automatically."`.
  - CLI didn't answer (`NoAnswer`): `"Antigravity CLI didn't answer."`.
  - Invalid data (`InvalidResponse`): `"Antigravity CLI returned invalid usage data."`.
- **Settings → Usage switches and copy:** Add the `Antigravity CLI` sidebar-badge toggle beneath `Codex` with subtitle `"Shows the highest weekly pool."`, and update `usage.subtitle` / `usage.interval_caption` / `usage.near_limit_hint` strings in `en.toml` and `zh-hans.toml`.
- **Near-limit alerts (`Notifications`):** Extend `UsageAlerts` (`notifications.rs`) to evaluate `Runtime::Antigravity` windows across both pools, respecting `settings.usage.five_hour`, threshold percentage, and independent `(runtime, account, window, resets_at)` deduplication per pool and reset period.

### 4. Documentation and localization

- Update `crates/runner-app/i18n/en.toml` and `crates/runner-app/i18n/zh-hans.toml`.
- Update the user guide pages and Chinese translations together:
  - `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`
  - `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md`

## Implementation phases

1. **Zero-login probe & daemon integration:** Verify safe non-interactive auth detection against `agy` (`1.5.8+` and isolated signed-out fixture; fail closed when `agy --version` is `< 1.5.8` or unknown), implement `crates/runner-daemon/src/runtimes/antigravity/usage.rs`, wire `UsageSource` in `antigravity/mod.rs`, restore `RuntimeCapabilities::usage`, extend `UsageSettings` (with backward-compatible serde defaults) and additive protocol/companion reason mapping if needed, and add daemon unit tests covering JSON pool parsing, signed-out gating, LKG preservation, and expired window pruning.
2. **Option F1 footer & Option P1 UI + alerts:** Implement Option F1 spacing and highest-usage + `+N` overflow collapse in `sidebar_usage_element` / footer layout, restore `usage_groups` (`Option P1`) and grouped `is_five_hour` filtering in `app_shell/usage.rs`, add highest-weekly-pool calculation, runtime-aware signed-out copy + secondary instruction, Settings → Usage toggle row, `UsageAlerts` Antigravity pool checks, `en.toml`/`zh-hans.toml` strings, and EN/ZH guide updates.
3. **Tests, review, and macOS live QA:** Update UI/protocol golden expectations (`catalog-usage-ui.json`, `catalog-usage-macos.json`, companion/protocol tests), pass workspace checks (`cargo test`, `clippy` including `--features updater`, `fmt --check`), complete independent code review, and execute the selected live checks below on macOS.

Starting points: `crates/runner-daemon/src/runtimes/antigravity/{mod.rs,models.rs}`, `crates/runner-daemon/src/usage.rs`, `crates/runner-core/src/protocol/{usage.rs,runtime_metadata.rs}`, `crates/runner-core/src/companion/actions.rs`, `crates/runner-app/src/{app_settings.rs,notifications.rs,runtime_ui.rs}`, `crates/runner-app/src/surfaces/{app_shell/usage.rs,app_shell/sidebar.rs,settings/usage.rs,remote_client/}` and commit `1f107b5a` (`fix(usage): drop Antigravity CLI usage`) in git history.

## Verification

Before running checks, confirm every changed behavior has corresponding automated or selected live coverage; this is a targeted feature validation, not a full regression sweep.

### Automated checks

- Antigravity usage JSON parsing (dual pools, 5-hour + weekly windows, single/empty pool, malformed output) and zero-login signed-out gate (proving no login-opening CLI command is spawned when unauthenticated).
- `UsageSettings` deserialization compatibility for existing settings files missing `antigravity` (defaults to `true`).
- `weekly_usage_percent` selects highest weekly pool across Antigravity pools (`Gemini Models` vs `Claude and GPT models`).
- `is_five_hour` filters Antigravity pool-prefixed 5-hour windows while leaving weekly windows and Codex credits intact.
- Option F1 footer geometry/overflow layout tests: 3 × 2-digit badges fit in 240 px sidebar & Settings preview without clipping; `100% × 3` and `200 px` narrow sidebar collapse deterministically to highest-usage + `+N`.
- `UsageAlerts` multi-pool notification triggers, 5-hour toggle exclusion, and reset deduplication.
- Updated `usage_ui_catalog_golden` and `runner-core` catalog/companion round-trip expectations, plus `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (and `--features updater` on macOS), workspace tests (`runner-core`, `runner-daemon`, `runner-app`), and `make fmt-check`.

### Selected live checks (macOS `qa`)

Required native platform: macOS (`qa` acquires `script/qa-lock` per `AGENTS.md`). Use existing signed-in development accounts read-only and isolated `/tmp` fixtures for signed-out / failure checks; never mutate global `agy` credentials or leave running app/daemon processes behind. Record results and screenshot paths in `docs/tests/runs/YYYY-MM-DD-mac.md`.

| ID | Selected check and reason |
| --- | --- |
| AGY-USAGE-01 | **Zero-login signed-out gate** (core safety trigger from `#1163`): Using an isolated signed-out `agy` home/executable override under `/tmp` (recording and restoring any development Settings → Agents override), restart/refresh Usage in the dev app. Verify no browser OAuth window or `agy login` prompt opens, and both Usage popover and Settings → Usage render `"Sign in to Antigravity CLI to see usage"` + `"Run agy login in a terminal. Runner won't open login automatically."`. |
| AGY-USAGE-02 | **Signed-in dual-pool display & `#1282` credits coexistence** (Option P1 + shared renderer): With the real signed-in `agy` CLI, inspect the footer badge, open Usage popover, and open Settings → Usage. Verify `Gemini Models` and `Claude and GPT models` pool subheadings with aligned `5 hours` / `Week` rows alongside Codex's **Credits remaining** row; toggle **5-hour limit** off/on and verify both Antigravity pool 5-hour rows hide while weekly rows and Codex credits remain. |
| AGY-USAGE-03 | **Option F1 footer, compact preview, `+N` narrow overflow & zoom** (extends `SET-USAGE-04` for 3 badges + collapse): Compare Settings → Usage `Sidebar preview` and the live footer across all 3 badges enabled, Antigravity toggled off/on, and Usage toggled off; verify footer badge shows the highest weekly pool `%`, hover/transparent-at-rest state, 8 px Host/Settings gap, dark & light themes at 100% and 150% zoom, narrow `200 px` sidebar and `800×600` window (using an isolated fixture if needed to exercise `100% × 3` overflow into `+N`), plus popover dismiss and collapsed sidebar hiding usage. |
| AGY-USAGE-04 | **Silent LKG retention, CLI error state & Notifications alert** (failure contract + `UsageAlerts` multi-pool scope): With an isolated Antigravity CLI fixture via Settings → Agents override, test no-LKG error (`"Antigravity CLI didn't answer."`), LKG failure (cached pools and original age survive silently with no inline warning) + recovery, and background Notification alert firing once per exceeding Antigravity pool/reset period when **Near the limit** is enabled. |

Remote two-host network testing and native Windows/Linux execution are outside live QA scope (covered by automated protocol/path unit tests and platform CI; disclose native Windows/Linux verification status explicitly in the PR).
