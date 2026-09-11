# Translucent window backdrop

Tracking issue: [#557](https://github.com/yicheng47/runner/issues/557). Status: dropped — closed as not planned on 2026-09-11; Jason judged it not important enough to carry right now after seeing the first Pencil pass. Kept as the record of the GPUI `Blurred` mechanism and the four-ground alpha design. Priority was P2.

## Motivation

Runner's window is opaque: `open_window` never sets `window_background`, so GPUI defaults to `WindowBackgroundAppearance::Opaque` (`crates/runner-app/src/main.rs:1390`). Ghostty, Zed, and the macOS 26 chrome all offer a translucent, blurred backdrop where the desktop shows through a tinted surface, and it is the look people now associate with a native Mac terminal; Jason asked for it on 2026-09-10 after seeing it in Ghostty (`background-opacity` below 1 plus `background-blur`).

GPUI already ships the mechanism. `WindowBackgroundAppearance::Blurred` puts an `NSVisualEffectView` under the Metal layer, marks the window non-opaque, and switches the renderer to alpha compositing; `Window::set_background_appearance` flips it on a live window, and Zed exposes it as its `background_appearance` theme setting. So the work is on Runner's side: every ground surface is painted at alpha 1, so nothing shows through even if the window were blurred.

## Behavior

### Window backdrop setting

Appearance gets a **Window backdrop** row: a segmented Solid / Translucent control (default Solid) and an opacity stepper in the zoom stepper's shape, 70% to 95% in 5% steps, default 85%, enabled only while Translucent is selected. The row caption says it reads best on dark themes; the setting itself is theme-independent. Persisted in `ui-settings.json` as `windowBackdrop` (`solid` | `translucent`) and `backdropOpacity`; serde defaults keep an existing file loading unchanged and never rewrite it.

The setting is app-wide and applies live: changing it calls `set_background_appearance` on every open window, and windows opened afterwards pass `window_background` at open. No relaunch.

### What goes translucent

Exactly four ground surfaces carry the backdrop alpha:

- the app root, painted with `theme::bg()` (`surfaces/app_shell.rs:153`);
- the sidebar, painted with `theme::sidebar()` (`surfaces/app_shell.rs:469`), which on macOS also holds the titlebar drag region and traffic lights;
- pane chrome, painted with `theme::panel()` (`surfaces/panes.rs`);
- the terminal grid's full-bounds fill of the palette background (`terminal/element.rs:1301`).

Default-background cells already skip their own quads, so the terminal only needs that one fill to change. Cells with an explicit background (a TUI status bar, a selection, marked text) stay opaque, which matches Ghostty. Raised rows, overlays, dialogs, the session overlay, the command palette, tooltips, and buttons stay opaque so text over them never sits on blurred wallpaper and the blur never stacks.

One `theme::backdrop_alpha()` reads the settings (1.0 when Solid) and is applied at those sites through the existing `with_alpha` helper; `TerminalStyle` carries the same alpha for its base fill. Nothing else in the theme changes.

### Windows

`WindowBackgroundAppearance::Mica` exists for Windows 11 and rides the same alpha plumbing, but `platform_ui/windows.rs` paints its own header with `theme::bg()` (`:65`) and `theme::sidebar()` (`:118`) and needs the same treatment. macOS lands first; Windows is a follow-up phase on `nightly-windows` per the Windows port rule.

## Non-goals

- macOS 26 Liquid Glass chrome. That is `NSGlassEffectView`, which GPUI does not expose; it would ride the AppKit frame path in `mac_chrome.rs` as raw objc and would only be visible through the same transparent pixels this spec creates. Revisit only if the blurred backdrop is not enough.
- Blur radius or material control. GPUI hardcodes `NSVisualEffectMaterial::Selection` and strips the effect view's tint layer, so the tint is entirely what Runner paints; that is the opacity stepper.
- Per-surface opacity (a translucent terminal under a solid sidebar). One alpha for all four grounds.
- Background images or a wallpaper of Runner's own.
- Changing any theme palette. Runner Light ([#529](https://github.com/yicheng47/runner/issues/529)) picks its own alpha behaviour when it lands.

## Design

`design/runner.pen`: the Appearance settings frame gains the Window backdrop row (segmented control plus stepper), and one direct-chat-with-split frame is rendered over a photo wallpaper at 85% so the tint of the four grounds and the contrast of an opaque overlay on top of them are signed off before code. Pencil-first per the post-cutover rule; the code phase does not start until Jason has approved the frames.

## Implementation Phases

1. **Design.** Appearance row and the translucent chat frame in `design/runner.pen`. Stop for sign-off.
2. **Backdrop.** `AppSettings` fields with serde defaults; `window_background` at `open_window` and `set_background_appearance` on change for every open window; `theme::backdrop_alpha()` on the four ground surfaces and the terminal base fill; the Appearance row with the stepper.
3. **Windows.** Mica on Windows 11 with the `platform_ui/windows.rs` header on the same alpha, on `nightly-windows`.

## Verification

- Fresh `ui-settings.json`: the window is opaque and pixel-identical to today.
- Translucent at 85% over a photo wallpaper: the desktop shows through the sidebar, an empty chat pane, and a terminal at the prompt; `htop` and a codex TUI keep their own opaque backgrounds; the command palette, a confirm dialog, the session overlay, and a tooltip stay solid over the blurred area.
- Toggle Solid to Translucent and back with two windows open: both flip without a relaunch.
- An existing settings file without the new keys loads with Solid and is not rewritten; an `AppSettings` test pins the defaults.
- `make verify` green.
