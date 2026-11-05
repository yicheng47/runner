# 634 m6 — Settings → Side panel

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m6-side-panel-settings`, on branch `feat/634-m6-side-panel-settings`. The branch was cut from the umbrella `origin/feat/634-file-preview` at `eeab67b6`, where m1–m5 have merged, and this brief is its tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase (`git rebase origin/feat/634-file-preview`), never by merge.

## Problem and outcome

The side panel and its Files view are built, but nothing about them can be set. The file icon theme, the code font and size, highlighting, single-click previews and how Markdown opens all run at m3–m5's defaults, and the panel can't be turned off. This mission adds Settings → **Side panel**, a group after App with a **Panel** page and a **Files** page. Every option changes the view it names, and every default matches today's behaviour, so nothing changes until someone changes a setting.

## Read first

- [The plan](../634-file-preview/plan.md), Phase 4, m6: the authoritative scope. The [spec](../../features/634-file-preview.md) wins on behaviour; see its "Settings → Side panel" section.
- The design, exported as PNGs into this worktree's ignored `target/design-ref/` from `runner/desktop/specs/634-file-preview.pen` (`yeelinks_design` `e8d94f2`). The PNGs win on visual detail. Match them with theme tokens; their sample data is illustrative. Do not delete them, and do not open `.pen` files.
  - `settings-side-panel-board.png` (board `c518A`): the whole board.
    - Its last section, "reordering views", is **not** in v1: no grips until #1141 gives a group two views.
  - `settings-panel-page.png`: the Panel page:
    - Show the side panel;
    - the Views card with a live header preview;
    - the This chat (Info) and Project (Files) rows.
  - `panel-off-closeup.png`: the Panel page with the panel off. The view switches keep their values, dimmed and inactive.
  - `settings-files-page.png` and `settings-files-option-lists.png`: the Files page and each option's list as it opens, with the default checked.
  - `side-panel-header.png` (board `uBcLR`): the header the preview redraws.
- The existing Settings pages and navigation under `crates/runner-app/src/surfaces/settings/`, `app_settings.rs`, and `i18n/en.toml` and `i18n/zh-hans.toml`.
- What m6 hooks into:
  - m2's `surfaces/side_panel.rs` (views, `PanelContext`);
  - m3's Files tree and icon-theme loader;
  - m4's viewer and its preview-tab model;
  - m5's highlighting and the Markdown `</>` toggle;
  - `terminal_fonts.rs` (#931's installed monospace list and `BUNDLED_FONT`);
  - Settings → Terminal's font family and size controls, to reuse.

## Deliverables

1. **The Side panel group** in the Settings navigation, after App, with Panel and Files pages, in English and Simplified Chinese.
2. **Panel page.**
   - Show the side panel (on). Turned off, the panel's toggle leaves every tab, the panel shortcut does nothing, and the view switches below are dimmed and inactive but keep their values.
   - Info (on) and Files (on) switches. A mission's Sessions view always stays, before Info.
   - A chat or terminal with every view off has no panel.
   - Turning Files off doesn't stop ⌘-click on a path from opening the file in the chosen editor.
   - The Views card opens with a real-size preview of a chat's panel header, which redraws as the switches change. Its rows are grouped This chat and Project, with no grips.
3. **Files page, Tree card:** File icons — **Catppuccin**, Material or Minimal. Catppuccin follows light and dark.
4. **Files page, Viewer card:**
   - **Font:** a searchable list of **Same as terminal** (the default), then **Bundled** (JetBrains Mono), then **Installed** (#931's monospace list, shared with the terminal's Font family). It sets code and Markdown code blocks. A saved font that's missing at launch falls back to JetBrains Mono and the choice is kept.
   - **Font size:** a 10–20 px stepper like Terminal font size. It follows Terminal font size until changed; then a Reset link appears and returns it to following. Rendered Markdown text scales with it.
   - **Syntax highlighting:** on. Off shows plain text.
   - **Open with a single click:** on. Off, every click opens a kept tab and there are no preview tabs.
   - **Markdown files open as:** **Rendered** or Source.
5. **Persistence:** every option is stored with the app's other settings in `app_settings.rs`, with normalisation for missing or invalid values. The defaults reproduce today's behaviour exactly.
6. **Guide:** document Settings → Side panel in [Chats and windows](../../guide/guides/chats-and-windows.md) and the settings reference, if there is one, in both locales.

Non-goals:
- reordering views and the grips;
- any Changes view (#1141) or plugin view (#1072);
- an app UI font setting;
- bundling more fonts;
- any protocol, daemon or CLI change.

This is expected to be `frontend-only`.

## Validation

Unit tests:
- each setting's default and its round trip through the settings file;
- normalisation of bad values;
- the missing-font fallback, keeping the saved choice;
- Font size following Terminal font size until changed, and again after Reset;
- Show the side panel off: no toggle, the shortcut a no-op, the switches dimmed;
- every view off: no panel;
- Files off: ⌘-click on a path still opens;
- Open with a single click off: no preview tabs;
- Markdown opening as Source;
- highlighting off;
- both pages from snapshots in both themes and both languages.

Run the `runner-app` tests, workspace clippy with the CI profile (with and without `--features runner-app/updater`), fmt and the guide translation check, and log exit codes.

Live checks are required, run by `qa` on macOS in dark theme, and named in the clean review. They include:
- the plan's checks: each option changes the view it names, and turning the panel off removes its toggle and shortcut;
- the Panel page against `settings-panel-page.png` and `panel-off-closeup.png`, and the Files page and its lists against their PNGs;
- Simplified Chinese on both pages.

Deferred, not run (Jason, 2026-11-04): the light theme, real-pointer checks (drags, resize, hover-only states) and remote access. qa launches the app through a QA wrapper bundle registered with LaunchServices, as m3–m5's qa did; computer use can't bind plain `target/debug/Runner`. If another mission's qa holds the development app, qa tells the operator rather than waiting.

## Authorization

Approved crew: the claude trio. Once review is clean and `qa` has reported, the crew may:
- commit and push this branch;
- open a PR against `feat/634-file-preview` (not `main`), titled `feat(ui): Settings → Side panel (#634 m6)`;
- label it `risk-medium`, plus `needs-live-test` until qa passes, and `frontend-only` if nothing outside `runner-app` changes;
- drive CI green on every job.

The operator merges it into the umbrella; the crew never merges.
