# 793 — Liquid glass appearance

Implement [P2 #793](https://github.com/yicheng47/runner/issues/793). Jason signed off the design on 2026-10-09 and asked for a codex trio mission that ends in an open PR, not a merge. Glass menus and popovers are in scope (Jason, 2026-10-09). Frame rate, memory and CPU are not measured here; [#831](https://github.com/yicheng47/runner/issues/831) covers them later.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-793-liquid-glass`, on branch `feat/793-liquid-glass`. The mission's directory is this worktree. Its tip is this brief, on top of main `01ff0bf4` (the design and spec). Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/793-liquid-glass.md`**: the Layout and Defaults paragraphs, Technical investigation and Verification. It wins over the issue and over this brief on any detail.
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/`: `glass-dark.png`, `glass-light.png` (the two-pane chat in Glass), `solid.png` (the same window in Solid), `side-panel-single-pane.png` (a single-pane chat with the side panel open), `floating-glass.png` (menu and usage popover close-ups), `appearance-settings.png`. Match their layout, hierarchy, spacing and colours with the existing theme tokens; the sample data is illustrative. The side panel's Info and Files tabs in `side-panel-single-pane.png` belong to #634: leave today's side panel header as it is. Do not delete the PNGs, and do not open or edit `.pen` files.
- Shell and chrome: `surfaces/app_shell.rs` (`render_app_shell`: the content column after the sidebar is what the card wraps; `render_app_sidebar`; `render_usage_popover`), `platform_ui/macos.rs`, `platform_ui/windows.rs` (`decorate_window`, the 32 px title bar), `main.rs` (`WindowOptions` near line 1774).
- Surfaces inside the card: `surfaces/panes.rs` (`render_chat_tab_header`, `render_chat_side_panel`, pane identity lines), `surfaces/mission_workspace/` (`view.rs`, `rail.rs`), `surfaces/settings_page.rs` (the Appearance page).
- Floating surfaces: `ui/menu.rs` (`popup_layer`, `popup_layer_sized`, `PopoverMenu`, `ContextMenu`), `ui/select.rs` (`option_menu`), `ui/model_field.rs`, and their call sites in `surfaces/sidebar/menus.rs`, `surfaces/roles/menu.rs`, `surfaces/crews/list.rs`, `surfaces/mission_workspace/composer.rs`.
- Tokens and settings: `theme.rs` (`ThemeColors`, `with_alpha`), `app_settings.rs` (`AppSettings`).
- GPUI `gpui-pre` 0.3.7 in `~/.cargo/registry`: `WindowBackgroundAppearance::Blurred` and `WindowKind::PopUp` (a non-activating `NSPanel` at the pop-up level, which can be blurred) in `gpui-pre-macos-0.3.7/src/window.rs`. `WindowKind::AnchoredPopup` is rejected on macOS, so glass menus position their own `PopUp` window. Diri's [floating panels](https://github.com/cristicretu/diri/blob/55cfa75c89695f3fcb2d52c759e3f961f80b536b/diri/crates/diri-app/src/floating.rs) and [AppKit panel behavior](https://github.com/cristicretu/diri/blob/55cfa75c89695f3fcb2d52c759e3f961f80b536b/diri/crates/diri-app/src/macos/floating_panel.rs) are the reference.

## Deliverable

1. **One work card, every platform and appearance.** Only the sidebar is chrome. Everything right of it sits in one card inset 8 px from the window's top, right and bottom (0 on the sidebar side): 12 px corner radius, `bg` fill, 1 px edge. Inside it the chat header, panes, side panel, terminal drawer and mission rail are split by plain 1 px dividers that run from the card's top to its bottom; no per-pane or per-section rounded surfaces. The tab header and side panel header sit inside the card, transparent over it, with a 1 px bottom line; the old divider at the sidebar edge goes, since the card edge replaces it. Every route gets the card: chat, archived chat, mission, roles, crews, profile and Settings (the Settings nav is the chrome there). Two cases are not drawn and follow the rule: the mission workspace (header, session tabs, panes and rail in the card), and a collapsed sidebar (the card keeps 8 px on all four sides, and the header keeps its traffic-light padding). GPUI clips to rectangles, so keep terminal cells, diff and selection backgrounds clear of the rounded corners with padding rather than relying on clipping.
2. **Material setting.** `AppSettings` gains a window material, Glass or Solid, defaulting to Glass on macOS and Solid elsewhere, with a serde default so existing settings files load. On macOS the Appearance page shows a "Window material" segmented control (Glass | Solid) under Theme, and the note under the page reads that Runner follows macOS Reduce Transparency and uses the solid appearance while it is on, as in `appearance-settings.png`. Windows and Linux are Solid only and hide the row. A change applies live to every open window.
3. **Glass on macOS.** Windows use `WindowBackgroundAppearance::Blurred`. The sidebar, the 8 px frame around the card and, on Settings, the nav are translucent chrome: the palette's sidebar colour at about 72% alpha in dark and 76% in light (the design's `#272930B8` and `#EEEFF3C2`), with `#FFFFFF12` / `#1C1D2218` separators and `#FFFFFF12` / `#1C1D220C` selected rows. The card and everything in it stay opaque, so terminal text, explicit TUI cell backgrounds, selection and IME composition read exactly as in Solid. Derive glass colours from the active palette with `with_alpha` rather than hard-coding one palette.
4. **Reduce Transparency.** When macOS Reduce Transparency is on, Glass renders as Solid: the window is opaque and menus use the solid style. Follow changes while running (`NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`), not only at launch.
5. **Glass menus and popovers, macOS Glass only.** Menus (`popup_layer`, `PopoverMenu`, `ContextMenu`, `option_menu`, the model field's menu) and the usage popover open in their own blurred `WindowKind::PopUp` window anchored to the trigger: about 70% `raised` fill (`#20232CB3` dark, `#FFFFFFB8` light), 1 px rim (`#FFFFFF55` / `#FFFFFFD9`), 16 px radius for menus and the usage popover's own radius, a soft drop shadow, as in `floating-glass.png`. They keep today's behaviour exactly: placement and flipping at screen edges, arrow keys, Enter and Esc, typeahead where it exists, outside-click dismissal, and focus returning to the terminal or field that opened them. They also close when the parent window moves, resizes, minimizes, closes, loses focus to another app or changes Space, and they work in fullscreen and on a second display. Solid, Reduce Transparency, Windows and Linux keep today's in-window menus in the solid style. Tooltips, modals, toasts and the quick switcher are out of scope. If a `PopUp` window cannot meet one of these behaviours, stop and report it to the human through Runner before shipping a workaround.
6. **Tests** beside the code they cover: the material default per platform and the serde default; the effective appearance (Glass with Reduce Transparency is Solid; non-macOS is Solid); which floating surfaces open as windows under each appearance; a headless layout check (`debug_selector` plus `VisualTestContext`) that the content column has the 8 px inset and the card wraps the chat header, the side panel and the mission rail; the Appearance row shown on macOS only. Update assertions the change invalidates. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`.
7. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope: #634's side panel tabs and Files panel, Windows Mica, tooltips and modals, frame-rate measurement, `.pen` files, README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo clippy --locked -p runner-app --all-targets --profile ci --features updater -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

The coder and reviewer never run the dev app; only `qa` launches the frozen candidate. Never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`), and do not start, stop, restart or type into Jason's Runner apps, chats or missions. Native Windows is unavailable; say what is unverified there.

## QA

`qa` tests the frozen candidate on macOS following `docs/tests/full-smoke-test.md` (macOS native control of the debug executable, the environment it says to unset, cleanup). Use shell terminals for panes; do not start agent chats or missions. Do not change the wallpaper or any system setting on your own: put a bright image open in another app behind the window for the bright case, and ask the human through Runner before toggling Reduce Transparency, restoring it afterwards, or mark that check Blocked. No frame-rate, memory or CPU measurement. Checks:

- Glass in dark and light over dark and bright content behind the window; Solid via the setting; the switch applies live to two open windows.
- The card on every route: chat single-pane and split, side panel open, terminal drawer open, roles, crews, Settings, sidebar collapsed, and a mission view if the development data already has one (do not start one; otherwise Blocked). Column dividers line up from the header to the bottom.
- Each glass menu and the usage popover: placement near screen edges, keyboard navigation, Esc and outside-click dismissal, closing on window move, resize, minimize, app switch and Space change, fullscreen, a second display if present (otherwise Skipped), and focus back where it was.
- Terminal legibility near the card corners and edges: `vim` or `htop` with explicit backgrounds, a selection, CJK input through IME composition.
- Reduce Transparency on and off while running, if the human allows it.

`qa` writes the record to `docs/tests/793-liquid-glass.md` with screenshot paths in a scratch directory outside the repository; the coder includes the record in the commit.

## Crew handoff and authorization

The linear pipeline is coder → reviewer → qa, as in the crew's conventions. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the spec, the PNGs and this brief, must-fix findings first with file:line pointers. It checks in particular:

- that terminal, diff and selection surfaces are opaque in every appearance and nothing draws into the rounded corners;
- that every popup window is closed on every dismissal path listed in item 5, with no leaked windows or focus left in a hidden panel;
- that Solid, Reduce Transparency, Windows and Linux keep today's in-window menus and an opaque window;
- that the setting loads from an existing settings file without the new field;
- that no Windows-only or macOS-only code breaks the other platform's build.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`, then hand the frozen candidate to `qa`. Fix QA failures through the reviewer and hand the new revision back naming the checks it affects. No extra agents, crews or subagents.

After the clean review and QA's verdict, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief and the QA record included, into a single commit on top of `main`: imperative subject naming the change (for example `feat(ui): liquid glass appearance with one work card`), no co-author trailers.
- **Push** with `git push -u origin feat/793-liquid-glass`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #793`, a summary, test evidence, QA's verdict as it is (a Failed or Incomplete verdict is never described as a pass), a manual check for Jason (Glass and Solid in dark and light, each menu and the usage popover, Reduce Transparency, the mission view, a collapsed sidebar), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, QA's verdict, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then all slots stand by.
