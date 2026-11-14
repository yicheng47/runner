# Mission Brief — #1319 One theme picker

- **Issue:** [#1319](https://github.com/yicheng47/runner/issues/1319) (`feature`, `P2`, milestone 0.19).
- **Starting point:** worktree `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1319-unified-themes`, branch `feat/1319-unified-themes`, off `origin/main` `627797fd`. Work only in this worktree, with its own `target/`.
- **Spec:** [`docs/features/1319-unified-themes.md`](../../features/1319-unified-themes.md), which wins on every detail. Design: `runner/desktop/specs/1319-unified-themes.pen` in `YeelinksCo/yeelinks_design` (checked out at `~/repos/yeelinks/yeelinks_design`), revision `055fe95`; frames `TV1e1` (Appearance, dark), `iPu8z` (Appearance, light), `c4x25` (Terminal), `hmanH` (`cmp/ThemeCard`), `oCxp2` (decisions).

## Problem And Goal

The app theme and the terminal palette are picked separately, so people assemble pairings by trial. Make a theme one pairing of app colours and terminal palette: Appearance shows a Light theme and a Dark theme gallery of six theme cards each, Terminal loses its Colors group, and four new families ship beside Runner and Catppuccin: Rosé Pine, Tokyo Night, Gruvbox and Solarized. Old terminal theme settings are dropped; app themes carry over.

## Read First

1. The spec, in full, and the design frames above.
2. `crates/runner-app/src/theme.rs` — `LightTheme`, `DarkTheme`, `ThemeVariant`, `resolve_variant`, the `ThemeColors` sets, Glass chrome.
3. `crates/runner-terminal/src/palette.rs` — `TerminalPalette` and the existing palettes.
4. `crates/runner-app/src/app_settings.rs` — `LightTerminalTheme`, `DarkTerminalTheme`, `terminal_palette`, `AppSettings` serialization and its tests.
5. `crates/runner-app/src/surfaces/settings/page/appearance.rs` and `mod.rs` — the theme preview, App palette card, terminal palette selections and the Terminal page's Colors group; `theme_preview` for the current preview element.
6. `docs/guide/guides/appearance.md` and its Chinese page.

## Deliverables And Boundaries

- Spec phases 1–4: the theme model and palettes, the Appearance galleries, the Terminal page, and the guide pages and licence notices, with English and Chinese strings.
- The spec's automated coverage.
- Out of scope as the spec lists. No protocol or daemon change is expected; stop and report if one turns out to be needed.

## Validation And Handoffs

1. **coder:** `make fmt`, `cargo nextest run --locked --workspace --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and the `--features updater` pass. Do not run the app. Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff. Look closely at the settings migration (old keys load and are dropped, unknown values fall back), every variant reaching a terminal palette with the right `is_light`, the new `ThemeColors` against the spec's reference values and readability on Glass, the gallery against the design, removal of every terminal-palette selection, licence credits, and English/Chinese string and guide parity. Pass the clean SHA to `qa` with the checks below plus any the diff adds.
3. **qa:** live checks UT-01 to UT-06 from the spec on macOS, following Test Scope: take `script/qa-lock` before building, run `make run` in this worktree, and use shell chats or tiny prompts on an inexpensive model at low effort in fresh folders under `/tmp`. Back up the development settings file before UT-05 and restore it at the end, including the light and dark themes qa changed; archive only qa's own chats. Capture a screenshot per check, record the run in `docs/tests/runs/1319-unified-themes-mac.md` and in the PR description.

## Authorization

The `agy trio` is authorized for #1319: it may commit, push `feat/1319-unified-themes` and open a PR against `main` closing #1319. Label it `frontend-only` and `risk-medium`, add `reviewed` for the final head, and keep `needs-live-test` until UT-01 to UT-06 have passed. Stop at the open PR.
