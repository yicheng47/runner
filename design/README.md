# Design

Pencil (`.pen`) source files for the Runners UI. Open these in the Pencil app; they are encrypted on disk and not meant to be read as plain text.

Conventions:

Feature specs, since 2026-09-18, live one file per spec in `specs/<issue>-<slug>.pen`, mirroring `docs/features/`. A spec file starts from a copy of `runner.pen`'s tokens and the components it needs, so it is a snapshot of that day; Pencil cannot reference components across files. `runner.pen` stays the product canvas, screens plus `cmp/` components, and changes only when a shipped surface changes or a spec introduces a component that ships (`cmp/MarkPi` from #539 is on both). Spec frames designed before then stay on `runner.pen` in the spec rows, where the feature docs point at them by frame id. The first is `specs/539-pi-runtime.pen`.

The macOS titlebar cluster (sidebar toggle, Previous page, Next page; #494) is recorded in `runner.pen`: the two arrows live inside `cmp/SidebarC`'s header row `sbDrag` (`sbBackIcon`, `sbNextIcon`) so every screen that instances the sidebar shows them, and the chrome spec is frame `YRWg3` ("Header navigation — #494 chrome spec") beside the #246 sidebar-toggle spec `w83yF`: control states, expanded/collapsed/fullscreen placement, and behavior notes.

The Windows shared title bar is recorded in `windows-header.pen`, frame `winShell` and header `winTitlebar`: a 32 px row above the sidebar and workspace, sidebar/history controls on the left, and 46 px caption buttons on the right. It is separate from the macOS chrome.

Windows in-app updates (#493) are recorded in `windows-updates.pen`: the centered update dialog component `cmp/UpdateDialog` (`kmgxV`) and its four visible states (`rZRA3`), the main window with the dialog opened from the sidebar icon (`AsdU1`), the Settings → Updates page in the Ready state (`uDGHz`) and with the dialog open while downloading (`Jr0d9`), both with the macOS-style hero card, the hero's status line and button for the five states (`f230`), the sidebar icon states (`f266`), and the flow note (`n300`). Tokens mirror `runner.pen`; the dialog is the only update surface, opened from the sidebar icon or the Settings button.

- One `.pen` file per major surface (e.g. `home.pen`, `crew-editor.pen`, `runner-card.pen`).
- `runner-mvp-design.pen` is the historical MVP canvas; do not add new feature work to it.
- `chat-attention-indicators.pen` frame `R4LJz` contains the issue #285 working, unread, and collapsed-rollup states.
- Exports land in `/assets/design/` once we need them in-app.

Follow-ups parked in design (no issue yet):

- Tooltip primitive (kebab-menu language): `runner-mvp-design.pen` node `sE5dM`. Spec'd during issue #34 Phase 4 but deferred — v1 ships with native `title`. Pick this up when a sidebar/rail tooltip primitive is genuinely needed.
