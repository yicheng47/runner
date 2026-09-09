# Previous and next page arrows in the macOS header

Tracking issue: [#494](https://github.com/yicheng47/runner/issues/494). Status: **shipped 2026-09-08 in [#514](https://github.com/yicheng47/runner/pull/514)**, released in 0.8.4; design `runner.pen` node `YRWg3`. Priority P2.

## Motivation

Runner 0.8.0 added **Previous page** and **Next page** arrows to the Windows header, beside the sidebar toggle. macOS has the same page-history navigation underneath but does not expose those two header buttons. Add them on macOS so the navigation controls and visual style align across platforms.

## Scope

- Add the existing `chevron-left.svg` and `chevron-right.svg` icon buttons to the macOS header, grouped with the sidebar toggle in the same order as Windows: sidebar, previous, next.
- Match the Windows buttons' visual treatment. No tooltips on macOS, decided 2026-09-08 after the smoke test; Windows keeps its **Previous page** / **Next page** tooltips. Adapt placement to the macOS traffic lights and titlebar spacing.
- Reuse `navigate_runtime_page(-1, ...)` and `navigate_runtime_page(1, ...)` and the existing per-window history. These navigate visited app pages; they do not cycle through adjacent tabs.
- Match the existing enabled/disabled behavior: previous is disabled at the start of history, next at the end, and both are disabled in Settings.
- Keep both buttons available with the sidebar expanded or collapsed and in fullscreen. Preserve traffic-light clearance, window dragging, double-click behavior, and terminal focus after navigation.

The Windows implementation is the behavior and styling reference: `crates/runner-app/src/platform_ui/windows.rs`, `render_windows_titlebar`. macOS chrome stays in `crates/runner-app/src/platform_ui/macos.rs`. The existing page history, keyboard shortcuts, and Windows layout retain their current behavior.

## Implementation Phases

1. Record the macOS placement in Pencil, covering sidebar expanded/collapsed and fullscreen states. Done in the active canvas `design/runner.pen` rather than a separate file: the arrows sit in `cmp/SidebarC`'s header so all screens carry them, and frame `YRWg3` is the chrome spec.
2. Add the two macOS header buttons using the existing icons, button component, and navigation behavior.
3. Verify navigation and platform chrome behavior on macOS, with a Windows regression check.

## Verification

- Previous/next icons appear in the macOS header with the same order and styling as Windows, without tooltips.
- After visiting multiple chat or mission pages, clicking the arrows follows the same history as the existing navigation shortcuts and restores focus correctly.
- Disabled states match Windows at history boundaries and in Settings; an empty history is safe.
- Buttons remain usable with the sidebar collapsed, expanded, and in fullscreen, including at different app zoom levels.
- Traffic lights remain unobstructed; clicking a button does not drag the window. Existing window dragging and double-click behavior still work.
- Runner app tests and workspace Clippy pass; Windows header appearance and behavior remain unchanged.
