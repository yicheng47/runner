# Brief: #1374 — Clearer Appearance theme controls and visible selected segmented state

## Starting point

- **Issue:** https://github.com/yicheng47/runner/issues/1374 (`feature`, `P2`, milestone `0.20`)
- **Signed-off design:** `YeelinksCo/yeelinks_design` (`~/repos/yeelinks/yeelinks_design`) commit `98ccb49`, `runner/desktop/specs/1374-appearance-theme-controls.pen`:
  - Frame `lBNZF` — Signed off Appearance page in Runner Light
  - Frame `b9YYq` — Segmented control selected state before and after (Runner Light & Runner dark)
- **Branch / worktree:** `feat/1374-appearance-theme-controls` at `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1374-appearance-theme-controls`
- **Base commit:** `8012ecb7dea011bf99811fd7450500afeee5a45c` (`origin/main`)
- **Parallel isolation:** Sibling worktree `.worktrees/feat-1213-worktrees-app` is active. Stay strictly inside this worktree, never share `CARGO_TARGET_DIR`, and avoid running heavy daemon-spawning workspace suites concurrently with sibling runs.

## Problem and expected behavior

Settings → Appearance currently uses "Theme" for the Auto/Light/Dark mode selector and "Light theme" / "Dark theme" for the two galleries, with only a faint `· in use now` suffix indicating the active gallery. Worse, segmented controls paint the selected segment with `theme::raised()` on a `theme::bg()` track — in Runner Light (`#ECEDF1` on `#F2F3F5`, ~1.05:1 contrast), the selected option reads as a faint darker dip rather than a chosen button.

Implement the signed-off design (`lBNZF`, `b9YYq`):

1. **Appearance page copy and hierarchy (`appearance.rs`, EN + zh-CN i18n):**
   - Page caption: `"Choose a mode, then the theme each mode uses. A theme sets the app and terminal colours together."`
   - Mode row (was **Theme**): title `"Mode"`, subtitle `"Follow the system, or always use light or dark."`, segments `Auto` (`monitor.svg`), `Light` (`sun.svg`), `Dark` (`moon.svg`).
   - Galleries (were **Light theme** / **Dark theme**): titles `"Light mode theme"` and `"Dark mode theme"`, right-aligned meta `"Used while Runner is light"` and `"Used while Runner is dark"`.
   - Active gallery marker: replace the `" · in use now"` meta suffix with an `"Active now"` pill beside the active gallery's title (`gap(8px)`): rounded-full pill, accent tint fill (`theme::with_alpha(theme::accent(), 0.12)` matching `#00A66A1F`), `px(8px)`, `py(2px)`, `gap(4px)`, `6px` accent dot + `11px` (`theme::text_meta()`) semibold `theme::accent()` text. Active gallery title uses `theme::text()` (`FontWeight::SEMIBOLD`); inactive gallery title uses `theme::muted()` (`FontWeight::MEDIUM`) with no pill.
2. **Visible selected state on segmented controls (`b9YYq`):**
   - Track: `bg(theme::bg())`, `border_1()` with `theme::border_strong()` (`$line-strong`), `rounded(rems(8. / 16.))`, `p(rems(3. / 16.))`, `gap(rems(2. / 16.))`.
   - Selected segment: `bg(theme::seg_selected())`, `border_1()` with `theme::seg_selected_border()`, 1 px outer drop shadow (`offset(0, 1px)`, `blur_radius(2px)`, `rgba(0, 0, 0, 0.12)` / `#0000001F`), `rounded(rems(6. / 16.))`, `px(rems(12. / 16.))`, `py(rems(5. / 16.))`, `FontWeight::SEMIBOLD`, `theme::text()` foreground (label and icon).
   - Unselected segment: transparent background and transparent `border_1()` (preventing layout shift on toggle), `theme::muted()` foreground hovering to `theme::text()`.
   - Preserve keyboard accessibility (`tab_index(0)`, plain Enter/Space activation, focus-visible ring, pointer cursor) and existing debug selectors where tests rely on them.
3. **Per-theme tokens for all 12 themes (`theme.rs`):**
   - Add `seg_selected` and `seg_selected_border` to `ThemeColors` (and `theme::seg_selected()` / `theme::seg_selected_border()`).
   - Use exact signed-off values for `RUNNER_LIGHT` (`#FFFFFF` / `#C6C9D1`) and `CARBON` (`#2E3038` / `#3B3E48`), and pick harmonious, clearly visible elevated fills and borders for the other 10 themes (light themes lifted bright against `bg`, dark themes lifted above `bg`/`panel`), verified by unit test across all 12 themes.
4. **Shared segmented control across Settings (`ui/settings.rs`):**
   - Extract a small shared segmented-control helper in `crates/runner-app/src/ui/settings.rs` and use it in all four Settings segmented controls so they stay consistent:
     - Mode (`appearance.rs`: `render_theme_segmented`)
     - Window material (`appearance.rs`: `render_material_segmented`, macOS)
     - Archived filter (`archived.rs`: `ArchivedFilter::ALL`)
     - Terminal Ctrl keys (`shortcuts.rs`: `render_ctrl_keys_row`, Windows/Linux)

## Read first

1. `~/repos/yeelinks/yeelinks_design/runner/desktop/specs/1374-appearance-theme-controls.pen` (frames `lBNZF` and `b9YYq`).
2. `crates/runner-app/src/surfaces/settings/page/appearance.rs`
3. `crates/runner-app/src/surfaces/settings/archived.rs` (around lines 385–425)
4. `crates/runner-app/src/surfaces/settings/page/shortcuts.rs` (around lines 506–562)
5. `crates/runner-app/src/ui/settings.rs` and `crates/runner-app/src/ui/surfaces.rs`
6. `crates/runner-app/src/theme.rs`
7. `crates/runner-app/src/surfaces/settings/page/mod.rs` (`search_terms` around line 160), `tests.rs` (around lines 1200–1450), and `crates/runner-app/i18n/{en,zh-hans}.toml`
8. `docs/guide/guides/appearance.md`, `docs/guide/zh-CN/guides/appearance.md`, and `docs/tests/regression/settings.md` (`SET-THEME-01`, `SET-THEME-02`)

## Non-goals

- Do not redesign theme gallery cards, card focus rings, or Window material behavior.
- Do not touch daemon, CLI, persistence schema, or unrelated Settings controls (e.g., `agents/launch.rs` button tabs).

## Deliverables and tests

- Implement the shared segmented-control component, `ThemeColors` tokens for all 12 themes, Appearance page copy/Active-now pill/Mode row updates, and adopt the shared control in `appearance.rs`, `archived.rs`, and `shortcuts.rs`.
- Update EN and zh-CN catalogs (`en.toml`, `zh-hans.toml`), remove `format.theme_in_use` if no longer referenced, and update `Self::Appearance` search terms in `settings/page/mod.rs` (include `"Mode"` and new gallery labels while keeping `"theme"` searchable).
- Update `docs/guide/guides/appearance.md`, `docs/guide/zh-CN/guides/appearance.md`, and `docs/tests/regression/settings.md` (`SET-THEME-01`/`02` labels); verify guide parity with `python3 script/check-guide-translations.py`.
- Update and add deterministic `runner-app` tests:
  - All 12 themes' `seg_selected` / `seg_selected_border` contrast and distinctness against `bg` and `fg`.
  - Appearance page layout, Mode row, `Active now` pill switching between light/dark galleries, narrow/zoomed fit, EN + zh-CN strings, and Archived/Shortcuts segmented selection.
  - Follow `AGENTS.md`'s non-flaky test rule (no fixed sleeps; loop any new app/async UI test ≥ 20× via `cargo nextest run -p runner-app --stress-count 20 -E '...'` and state the count).
- Run `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, and `python3 script/check-guide-translations.py` before review handoff.

## Role and QA boundaries

- `coder` and `reviewer` never launch the app. `reviewer` reviews local commits, checks test determinism and design fidelity across all 12 themes, applies `reviewed` on the final PR head, and confirms the live QA checks.
- `qa` takes `script/qa-lock acquire --timeout 540` before building/launching, holds it through teardown (preserving pre-existing dev sessions), records results and screenshots in `docs/tests/runs/1374-appearance-theme-controls-mac.md` and the PR description, and releases `script/qa-lock`.
- Required macOS live checks:
  1. **`SET-THEME-01` + #1374 layout (EN & zh-CN, 960 px/100% and 1440 px/130%):** Verify new caption, Mode row (`Auto`/`Light`/`Dark` + icons), Window material, `Light mode theme` / `Dark mode theme`, `Active now` pill on the active gallery only, Tab/Shift-Tab + Enter/Space + mouse selection, and clean non-clipping labels.
  2. **`SET-THEME-02` + selected-segment visibility across themes:** Toggle Mode between Light and Dark (confirm `Active now` pill moves and open terminal recolours live); visually verify selected segment contrast, border, and subtle shadow in Runner Light, Runner (Carbon), and alternate light/dark themes (at least Gruvbox Light, Solarized Light, Tokyo Night), plus Settings → Archived's filter segmented control in Runner Light.
  3. **`SET-PREF-02`:** Toggle Window material between Glass and Solid and confirm the selected segment clearly reflects state; restore original settings.
- Disclose macOS live coverage vs Windows/Linux CI (`shortcuts.rs` Ctrl-keys segmented control is Windows/Linux-only and uses the shared component).

## Authorization

- **Approved crew:** `neon trio` (explicitly approved by Jason).
- **Git / PR mode:** Authorized to commit, fold this brief into the implementation commit(s) before final push (no standalone brief commit in PR history), push `feat/1374-appearance-theme-controls`, open a PR against `main` (`Fixes #1374`, `frontend-only`, `risk-low`, `reviewed`, `needs-live-test` per `AGENTS.md`), and drive CI green on all platforms.
- **Stop gate:** Stop at an open, reviewed, QA-verified, CI-green PR. Do not merge, delete branches, or cut a nightly/release.
