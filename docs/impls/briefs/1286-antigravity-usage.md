# #1286 — Antigravity CLI usage indicator

## Starting point

Implement [#1286](https://github.com/yicheng47/runner/issues/1286) on branch `feat/1286-antigravity-usage` in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1286-antigravity-usage`, branched from `origin/main` at `95d74efa`. Keep the root checkout untouched on `main` and build strictly inside this worktree (`target/` stays worktree-local).

Historical context: `1f107b5a` (`fix(usage): drop Antigravity CLI usage`, #1181) removed the original Antigravity usage integration because `agy -p /usage --output-format json` opens a browser OAuth login window when `agy` is signed out. Do not blindly revert `1f107b5a` — `#1271`/`#1277` (compact footer & preview) and `#1279`/`#1282` (Codex `Credits remaining` and silent refresh-failure LKG retention) landed later and must be preserved.

## Problem and outcome

Restore Antigravity CLI usage across the sidebar footer, Settings `Sidebar preview`, Usage popover, Settings → Usage, remote read-only usage, and background near-limit notifications **without ever triggering automatic browser login when signed out**, per `docs/features/1286-antigravity-usage.md`.

Design authority: `runner/desktop/specs/1286-antigravity-usage.pen` in `/Users/jason/repos/yeelinks/yeelinks_design`, revision `a2321c8` (read-only; do not modify `yeelinks_design`).

- **Approved Option F1** (`QSiVA`, `o78yO7`, `N4yXC`, `asuJW`, `FFibS`, `syztz`): tighten the shared `sidebar_usage_element` pill (`6 px` horizontal padding, `5 px` badge gap, `2 px` icon-to-`%` gap; keep `8 px` Host/Settings gap and transparent resting fill) so 3 weekly badges fit in `240 px`; show Antigravity's **highest weekly pool `%`**; when badges exceed available footer width (`200 px` min sidebar or `100% × 3`), show highest-usage badges + `+N`.
- **Approved Option P1** (`F288uj`, `Z2MEX`, `mXc9d`, `CpeG5`, `HLn8E`, `fKPvG`, `uSsDR`, `vx7Wn`, `syztz`): render muted `Gemini Models` and `Claude and GPT models` pool subheadings over standard `76 px`-label `5 hours` and `Week` rows alongside `#1282` Codex `Credits remaining`; silently retain LKG values + original timestamp on refresh failure; show calm `"Sign in to Antigravity CLI to see usage"` / `"Run agy login in a terminal. Runner won't open login automatically."` when signed out; filter both pools' `5 hours` windows when `5-hour limit` is hidden; include Antigravity pools in `Notifications` → `Usage alerts`.

If details between historical commit `1f107b5a~1` and the spec/design differ, `docs/features/1286-antigravity-usage.md` and `1286-antigravity-usage.pen` revision `a2321c8` win.

## Read first

- `docs/features/1286-antigravity-usage.md` (full behavioral & QA contract)
- `git show 1f107b5a` (prior implementation & removal commit — note later `#1271`/`#1282` diffs on top)
- Daemon & protocol:
  - `crates/runner-daemon/src/runtimes/antigravity/mod.rs` and `models.rs`
  - `crates/runner-daemon/src/usage.rs` (`UsageService`, `apply_result`, `unexpired`)
  - `crates/runner-core/src/protocol/{usage.rs,runtime_metadata.rs}` and `crates/runner-core/src/companion/actions.rs` (`UsageUnavailableReason` wire compatibility)
- App UI, settings & alerts:
  - `crates/runner-app/src/app_settings.rs` (`UsageSettings`)
  - `crates/runner-app/src/runtime_ui.rs`
  - `crates/runner-app/src/surfaces/app_shell/{usage.rs,sidebar.rs,mod.rs}` (`sidebar_usage_element`, `weekly_usage_percent`, `is_five_hour`, `usage_section_with_account`, `unavailable_line`, `credit_row`)
  - `crates/runner-app/src/surfaces/settings/usage.rs` and `crates/runner-app/src/notifications.rs` (`UsageAlerts`)
  - `crates/runner-app/i18n/{en,zh-hans}.toml`
- Docs & expectations:
  - `docs/guide/guides/chats-and-windows.md`, `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/` counterparts
  - `crates/runner-app/src/expectations/catalog-usage-ui.json`, `crates/runner-core/tests/expectations/catalog-usage-macos.json`

## Deliverables and boundaries

1. **Non-interactive zero-login probe (verify first in Phase 1):** Inspect how installed `agy` (`1.5.7+` uses OS keyring / `agy` CLI/ACP) behaves when signed out vs signed in inside isolated test environments (`HOME`, Keychain/keyring absence, etc.), and implement a preflight/probe in `crates/runner-daemon/src/runtimes/antigravity/usage.rs` guaranteed **never** to launch browser OAuth or interactive login when signed out (returning `UnavailableReason::SignIn`). Document cross-platform assumptions/gaps for Windows/Linux where macOS Keychain differs from Secret Service / Credential Manager.
2. **Protocol, daemon & settings:** Restore `UsageSource` on `Antigravity` and `RuntimeCapabilities::usage = true`. Add `UsageSettings.antigravity` with backward-compatible serde default `true`. Handle Antigravity CLI errors cleanly while preserving companion protocol forward/backward compatibility (`UsageUnavailableReason`). Preserve SQLite LKG persistence cleanly across restarts and failed refreshes (silent retention matching `#1282`).
3. **Option F1 footer & preview:** Tighten `sidebar_usage_element` padding/gaps, calculate highest weekly pool `%` across Antigravity pools, implement width-aware highest-usage + `+N` overflow collapse in both sidebar footer and `Settings → Usage` compact `Sidebar preview` (`240 px`, narrow `200 px`, `100% × 3`), keeping Host/Settings gaps and transparent-at-rest style intact.
4. **Option P1 popover, Settings, Alerts, i18n & docs:** Restore Antigravity pool subheadings (`Gemini Models`, `Claude and GPT models`) with aligned `76 px` rows, update `is_five_hour` to match pool-prefixed 5-hour windows, show runtime-aware signed-out copy + `"Run agy login in a terminal. Runner won't open login automatically."`, add `Antigravity CLI` switch (`Shows the highest weekly pool.`), extend `UsageAlerts` across pools, update `en.toml`/`zh-hans.toml` and EN/ZH guide pages, and update golden/unit/UI tests.
5. **Boundaries:** No automatic login launch from usage refresh, no credential values in logs/UI, no new Settings polling loop or SQLite migration, and no changes to Codex `#1282` credits or existing Claude multi-account behavior.

## Validation and handoffs

- `coder` verifies automated tests (`runner-daemon`, `runner-core`, `runner-app` usage/layout/notification/settings tests, golden expectations), `cargo fmt --check`, and `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (plus `--features updater`). Neither `coder` nor `reviewer` launches the running Runner GUI app.
- Hand off a frozen local commit SHA on `feat/1286-antigravity-usage` to `reviewer` through Runner messages before any QA or PR. Fold any `fixup!`/review commits cleanly.
- After `reviewer` reports a clean review on the frozen SHA, `coder` hands that reviewed SHA to `qa` with the live checks `reviewer` names plus `AGY-USAGE-01` through `AGY-USAGE-04` from `docs/features/1286-antigravity-usage.md`. `qa` acquires `script/qa-lock`, tests only that candidate binary on macOS using isolated `/tmp` fixtures where needed, restores all dev settings/overrides, cleans up processes/test artifacts, writes the evidence record to `docs/tests/runs/YYYY-MM-DD-mac.md` (no credentials), and reports the verdict back to `coder`.

## Authorization

This mission authorizes the approved **`agy trio`** (`coder`, `reviewer`, `qa`; 3 slots only, no extra agents or native subagents) in **PR mode** on branch `feat/1286-antigravity-usage` inside `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1286-antigravity-usage`:

- `coder` is authorized to make local commits on `feat/1286-antigravity-usage` for snapshot handoffs to `reviewer` and `qa`, folding this preparation commit and review fixes into a clean commit history.
- Only **after** `reviewer` posts a clean review and `qa` reports its verdict on that reviewed snapshot is `coder` authorized to include `qa`'s test record (`docs/tests/runs/YYYY-MM-DD-mac.md`), push `feat/1286-antigravity-usage`, open a pull request against `main` (`Closes #1286`, `risk-medium`, not `frontend-only`, preserving any automatic `core-review`, `reviewed` on the approved current head, removing `needs-live-test` only if all selected macOS checks passed while recording Windows/Linux live coverage gaps), and drive required CI green.
- Stop at the open PR with green CI; do **not** merge or release.
