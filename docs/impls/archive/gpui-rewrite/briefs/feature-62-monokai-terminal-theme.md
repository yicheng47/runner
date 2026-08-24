# Mission brief — Feature 62: Monokai terminal theme

Drafted 2026-08-24. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), no `cwd`, title `Feature 62 — Monokai terminal theme`, the text below as `goal_override`. Lands as a nightly; closes #406.

---

Implement **feature 62 — Monokai terminal theme** (`docs/features/62-monokai-terminal-theme.md`, issue #406; this brief also lives at `docs/impls/archive/gpui-rewrite/briefs/feature-62-monokai-terminal-theme.md`). **Branch off `main`**; one feature branch in this checkout; leave any uncommitted docs alone. Standing GPUI rules in `docs/impls/gpui-rewrite/README.md` apply. **Crews do not launch the app**: Jason smoke-tests; you implement and verify with tests.

## The change (surveyed 2026-08-24, all sites confirmed)

Purely additive — three files:

1. `crates/runner-terminal/src/palette.rs` — add `pub const MONOKAI: TerminalPalette` after `SOLARIZED_DARK` (`:76`), same shape (`rgb(...)`): background `#272822`, foreground `#F8F8F2`, cursor `#F8F8F2`, cursor_accent `#272822`, selection `#49483E`, ansi 0–15 `#272822 #F92672 #A6E22E #E6DB74 #66D9EF #AE81FF #A1EFE4 #F8F8F2 #75715E #F92672 #A6E22E #F4BF75 #66D9EF #AE81FF #A1EFE4 #F9F8F5`. The spec's table is authoritative.
2. `crates/runner-app/src/app_settings.rs:30` — `Monokai` variant on `TerminalTheme` (last position; `#[serde(rename_all = "kebab-case")]` gives `"monokai"` for free) and the `palette()` arm at `:39` → `palette::MONOKAI`.
3. `crates/runner-app/src/surfaces/settings_page.rs` — dropdown entry after Solarized Dark at `:269`: `SelectOption::new("monokai", "Monokai").swatch(0xf92672)`; a `terminal_theme_value` arm at `:1799`; a `parse_terminal_theme` arm at `:1807`.

The compiler owns exhaustiveness: `cargo check` flags any other non-wildcard `match` on `TerminalTheme` (there are none today — `app_store.rs:143` stores it in the settings fingerprint, which is how attached panes already repaint on switch; no per-variant code anywhere else).

## Tests

- Extend the existing settings tests in `app_settings.rs` (module at the bottom): `"terminalTheme": "monokai"` persists and loads back to `TerminalTheme::Monokai`, mirroring how `persisted_labels_match_the_react_settings_contract` (`:427`) asserts the kebab-case contract.
- If `settings_page.rs` tests pin the option-list/value mapping, extend them. Existing assertions extended, never weakened or deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- Handoff to Jason: the branch name plus the manual pass from the spec §Verification (dropdown shows Monokai with the pink swatch, live repaint of open panes, background tracks `#272822`, selection persists across relaunch).
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) palette values differing from the spec table (check all 21 hex values digit by digit, including the two bright deviations `#F4BF75` and `#F9F8F5`); (2) a site missed — the enum, `palette()`, the `SelectOption` list, `terminal_theme_value`, and `parse_terminal_theme` must all agree on `"monokai"`; (3) scope creep — any diff outside the three files plus tests (no `theme.rs`, no app-chrome theming, no refactors); (4) tests weakened or deleted instead of extended; (5) a hand-rolled serde impl or string constant where the kebab-case derive already does it.
