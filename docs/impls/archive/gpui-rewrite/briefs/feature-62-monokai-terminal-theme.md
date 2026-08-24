# Mission brief — Feature 62: Monokai terminal theme

Drafted 2026-08-24. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`, coder = codex lead, reviewer = claude-code), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), no `cwd`, title `Feature 62 — Monokai terminal theme`, the text below as `goal_override`. Lands as a nightly; closes #406. Updated during the smoke test with the final palette values, the Runner background lift, select-option alignment correction, popup hitbox occlusion, and retired-theme cleanup.

---

Implement **feature 62 — Monokai terminal theme** (`docs/features/62-monokai-terminal-theme.md`, issue #406; this brief also lives at `docs/impls/archive/gpui-rewrite/briefs/feature-62-monokai-terminal-theme.md`). **Branch off `main`**; one feature branch in this checkout; leave any uncommitted docs alone. Standing GPUI rules in `docs/impls/gpui-rewrite/README.md` apply. **Crews do not launch the app**: Jason smoke-tests; you implement and verify with tests.

## The change (surveyed 2026-08-24, all sites confirmed)

The Monokai implementation began in three files; smoke-test follow-ups also touch the existing Runner palette plus the shared select and popup renderers:

1. `crates/runner-terminal/src/palette.rs` — add `pub const MONOKAI: TerminalPalette` after `CATPPUCCIN_MOCHA`, same shape (`rgb(...)`): background `#2D2A2E`, foreground `#FCFCFA`, cursor `#C1C0C0`, cursor_accent `#8E8D8D`, selection `#5B595C`, ansi 0–15 `#2D2A2E #FF6188 #A9DC76 #FFD866 #FC9867 #AB9DF2 #78DCE8 #FCFCFA #727072 #FF6188 #A9DC76 #FFD866 #FC9867 #AB9DF2 #78DCE8 #FCFCFA`. The spec's table is authoritative. Also raise the existing Runner palette background from `#15161B` to the app panel color `#1D1E23` and its selection highlight from `#272930` to `#3B3E49`, leaving ANSI black and `cursor_accent` at `#15161B`.
2. `crates/runner-app/src/app_settings.rs` — add `Monokai` to `TerminalTheme`; use Serde's unknown-variant fallback on Runner so removed or future labels do not invalidate the settings file; add the `palette()` arm → `palette::MONOKAI`.
3. `crates/runner-app/src/surfaces/settings_page.rs` — add `SelectOption::new("monokai", "Monokai").swatch(0xff6188)` after Catppuccin Mocha and update `terminal_theme_value` plus `parse_terminal_theme`.
4. `crates/runner-app/src/ui/select.rs` — center the contents of genuinely single-line option rows; keep detailed or described two-line rows top-aligned with the existing swatch offset.
5. `crates/runner-app/src/ui/menu.rs` — occlude hitboxes beneath popup panels so an Agent option click cannot bleed through and open the covered Model field.

The compiler owns exhaustiveness: `cargo check` flags any other non-wildcard `match` on `TerminalTheme` (there are none today — `app_store.rs:143` stores it in the settings fingerprint, which is how attached panes already repaint on switch; no per-variant code anywhere else).

## Tests

- Extend the existing settings tests in `app_settings.rs` (module at the bottom): `"terminalTheme": "monokai"` persists and loads back to `TerminalTheme::Monokai`, mirroring how `persisted_labels_match_the_react_settings_contract` (`:427`) asserts the kebab-case contract.
- Pin the compatibility fallback: an unknown persisted terminal-theme label loads as Runner without resetting another setting.
- Pin popup occlusion with a visual interaction test that overlaps a popup and a capture-phase control.
- If `settings_page.rs` tests pin the option-list/value mapping, extend them. Existing assertions extended, never weakened or deleted.

## Gates

- `make verify` green; `git diff --check` clean.
- Handoff to Jason: the branch name plus the manual pass from the spec §Verification (dropdown contains Runner, Catppuccin Mocha, and Monokai only; Monokai has the pink swatch and repaints open panes; Monokai background tracks `#2D2A2E`; Runner background tracks `#1D1E23` and its text-selection highlight uses `#3B3E49`; theme selection persists across relaunch; single-line options center vertically; described two-line options remain top-aligned; changing Agent does not open Model suggestions).
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) palette values differing from the final Monokai table (check all 21 hex values digit by digit, especially background `#2D2A2E`, pink `#FF6188`, orange `#FC9867`, cursor `#C1C0C0`, and selection `#5B595C`); (2) the Runner palette must use background `#1D1E23` and selection `#3B3E49`, with ANSI black and `cursor_accent` still `#15161B`; (3) the picker, palette constants, enum, value mapper, and parser must expose exactly Runner, Catppuccin Mocha, and Monokai; (4) unknown persisted terminal-theme labels must fall back to Runner without a hand-rolled deserializer; (5) the enum, `palette()`, `SelectOption`, `terminal_theme_value`, and `parse_terminal_theme` must agree on `"monokai"`; (6) no app-chrome theming or refactors; (7) tests extended rather than weakened; (8) the select alignment gate must remain per-option so described rows stay top-aligned; (9) popup occlusion must block covered capture-phase controls without breaking menu clicks or outside dismissal.
