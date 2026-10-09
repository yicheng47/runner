# Liquid glass appearance

Tracking issue: [#793](https://github.com/yicheng47/runner/issues/793). Priority: P1, 0.14 (moved from 0.13 on 2026-10-07), and the release blocker of 0.14.0 since 2026-10-09, because it is a new design primitive. It headed 0.14 until file preview and code review ([#634](../634-file-preview-and-review.md)) took that place on 2026-10-09, and it still goes first so #634 is designed in it. Status: design signed off by Jason on 2026-10-09; implementation next.

Design: [Liquid glass and files](../../../design/specs/793-634-glass-and-files.pen), shared with file preview and code review ([#634](../634-file-preview-and-review.md)) so its panel is designed in the new chrome. Selected direction: A · Frosted chrome. Primary frames: `DJM0c` (dark) and `P1ZnQS` (light); shared sidebar: `N4yXC`.

Layout: only the sidebar is chrome. Everything to its right, the chat header, the panes and the side panel, sits in one card inset 8 px from the window, and the columns inside it are split by plain 1 px dividers that run from the card's top to its bottom, so a header divider can never drift from the seam below it. The layout is universal: Glass and Solid share it on macOS and Windows alike, and Glass only changes the material of the sidebar and the frame around the card. On Windows the card sits below the existing 32 px title bar with the caption buttons, which takes the sidebar's color so it reads as part of the frame. Decided by Jason on 2026-10-09, replacing a rounded surface per section.

Titlebar controls stay anchored to the window when the sidebar collapses or expands. On macOS the traffic lights, sidebar toggle and back/forward controls share a centre 30 px below the window top, scaled by app zoom, in line with the inset card header. The native traffic-light group starts 24 px from the left edge, with the sidebar toggle 24 px past it. The sidebar reserves a 52 px top row. On Windows the sidebar toggle and back/forward share the same row below the existing 32 px caption bar; the caption buttons stay in that bar. Collapsed workspace headers reserve space for these controls rather than rendering a second cluster. Jason corrected the alignment on 2026-10-09 after native QA found the 8 px offset and collapse jump; see `target/design-ref/titlebar-alignment.png`.

Headers have no background of their own: the chat tab, archived chat, side panel, mission and mission rail headers sit on the card's opaque `theme::bg()` with a `theme::border()` bottom line, as does the terminal drawer strip. Jason chose option B on 2026-10-09, superseding option E and the earlier panel-colour correction.

Split-pane identity lines are the exception: they take `theme::panel()`, so a split tab's pane headers read apart from the tab header above them. The mission tab strip stays on the card colour, and its active tab is a `theme::panel()` pill like a selected sidebar row, with no accent underline (option 4, `kk1Tt`). The mission feed sits on `theme::bg()` like a terminal pane, with its event payloads and composer field on `theme::panel()`. The chat side panel and the mission rail sit on `theme::bg()` from header to bottom, with their cards (the runtime card, session cards and the working-dir button) on `theme::panel()` as the only lighter shapes, and the sidebar, side panel and mission rail splitters keep the resize cursor without the accent bar. Jason chose option 3 (`bjm0K` in the design file) on 2026-10-09, after native QA found the split headers too close to the tab header and the side panel stacking three tones.

Glass uses one fixed opacity with no user control: 88% alpha in dark and 90% in light for the sidebar, the 8 px frame and the Settings nav. Derive chrome from the active palette's sidebar colour with `with_alpha`. Jason raised these values on 2026-10-09, superseding the brief's 72% / 76% chrome values. Menus and the usage popover use their existing opaque palette colours and corner radii in every appearance.

Defaults: Glass on macOS, Solid on Windows. Windows offers only Solid in 0.14, so its Appearance settings leave out the Window material row; Mica, Windows 11's own material, is the candidate for a later Windows option. Decided by Jason on 2026-10-09.

## Motivation

Give Runner a coherent liquid glass feel inspired by [Diri](https://github.com/cristicretu/diri): blurred backdrops, layered translucent chrome, subtle edge highlights and polished floating surfaces, while keeping terminal text easy to read.

Jason requested this after comparing Diri with Runner on 2026-10-03. This revisits the visual direction of [#557](https://github.com/yicheng47/runner/issues/557), which was closed as not planned, with a broader focus on the material treatment rather than a uniform window-opacity setting.

## Scope

- Design the treatment in Pencil before implementation, using a feature-scoped file under `design/specs/` and Runner's existing product components. Explore light and dark themes over varied desktop backgrounds.
- Apply glass to the macOS sidebar and persistent window chrome. Keep terminal surfaces dense enough for long coding sessions and preserve explicit TUI cell backgrounds, selections and IME legibility.
- Menus, the usage popover, dialogs, toasts and tooltips stay solid in every appearance. Menus and usage render inside the parent window on macOS, Windows and Linux (Jason, 2026-10-09).
- Keep persistent chrome translucent and work surfaces and floating controls opaque, with restrained borders and highlights.
- Provide a solid appearance option and account for the system's Reduce Transparency preference. Settle the default and any transparency controls during design.
- Preserve Windows functionality; evaluate an appropriate native material treatment separately rather than assuming macOS blur behavior transfers directly.

## Technical investigation

Diri uses GPUI's `WindowBackgroundAppearance::Blurred` with AppKit backdrop blur and translucent theme tokens. Its glass menus open in separate non-activating blurred panels because GPUI cannot blur behind an individual element. Review the positioning, focus, dismissal, accessibility and multi-window costs before choosing that approach for Runner. The requested appearance does not require adopting `NSGlassEffectView` or Diri's GPUI fork; verify what Runner's pinned GPUI version supports first.

On 2026-10-09 Jason initially chose blurred panels for menus and the usage popover. Native visual QA then found doubled or overlapping lower/left rims and corners on the usage popup (F2 in [the QA record](../../tests/archive/793-liquid-glass.md)). Jason withdrew that attempt and selected the fallback: keep the sidebar and persistent chrome glass, restore all menus and usage to the existing solid in-window presentation, and remove the native popup implementation. This decision supersedes the floating-panel deliverable in the mission brief and the `floating-glass.png` export.

References: [material tokens](https://github.com/cristicretu/diri/blob/55cfa75c89695f3fcb2d52c759e3f961f80b536b/diri/crates/diri-ui/src/tokens.rs), [floating panels](https://github.com/cristicretu/diri/blob/55cfa75c89695f3fcb2d52c759e3f961f80b536b/diri/crates/diri-app/src/floating.rs), [AppKit panel behavior](https://github.com/cristicretu/diri/blob/55cfa75c89695f3fcb2d52c759e3f961f80b536b/diri/crates/diri-app/src/macos/floating_panel.rs), and the archived Runner backdrop spec at `docs/features/archive/557-window-backdrop.md`.

## Implementation phases

1. Design and compare representative chat, sidebar and floating-control frames; settle material densities, settings and platform scope.
2. Validate the blur/compositing approach for persistent chrome in Runner's pinned GPUI version; keep menus and popovers solid inside the parent window.
3. Implement the approved material treatment and settings, then validate native interactions and platform fallbacks.

## Verification

- Light and dark themes remain readable over bright, dark and busy backdrops, including opaque terminal output, explicit TUI backgrounds and selection near the card edges.
- Solid in-window menus and the usage popover retain correct focus, keyboard navigation, anchoring, edge placement and outside-click/Esc dismissal in both materials. Their fills and corner radii match the existing solid style. Native popup lifecycle and separate-window display checks no longer apply.
- Appearance changes apply consistently to existing and newly opened windows; solid appearance and Reduce Transparency behavior are verified.
- Frame rate, memory and CPU are not measured in this feature; the UI performance tests ([#831](https://github.com/yicheng47/runner/issues/831)) cover them after the glass change. Jason dropped them from this feature's QA on 2026-10-09.
- Run runner-app tests and workspace Clippy; record macOS visual validation and Windows compatibility evidence.

Jason narrowed live QA on 2026-10-09 to the feature's changed behavior and directly affected regressions. The card padding requires terminal rendering and selection checks; unchanged IME composition and general runtime behavior are outside this feature's live QA. Required automated gates and macOS/Windows CI remain.
