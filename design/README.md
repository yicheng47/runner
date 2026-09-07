# Design

Pencil (`.pen`) source files for the Runners UI. Open these in the Pencil app; they are encrypted on disk and not meant to be read as plain text.

Conventions:

The Windows shared title bar is recorded in `windows-header.pen`, frame `winShell` and header `winTitlebar`: a 32 px row above the sidebar and workspace, sidebar/history controls on the left, and 46 px caption buttons on the right. It is separate from the macOS chrome.

Windows in-app updates (#493) are recorded in `windows-updates.pen`: the centered update dialog component `cmp/UpdateDialog` (`kmgxV`) and its four visible states (`rZRA3`), the main window with the dialog opened from the sidebar icon (`AsdU1`), the Settings → Updates page in the Ready state (`uDGHz`) and with the dialog open while downloading (`Jr0d9`), both with the macOS-style hero card, the hero's status line and button for the five states (`f230`), the sidebar icon states (`f266`), and the flow note (`n300`). Tokens mirror `runner.pen`; the dialog is the only update surface, opened from the sidebar icon or the Settings button.

- One `.pen` file per major surface (e.g. `home.pen`, `crew-editor.pen`, `runner-card.pen`).
- `runner-mvp-design.pen` is the historical MVP canvas; do not add new feature work to it.
- `chat-attention-indicators.pen` frame `R4LJz` contains the issue #285 working, unread, and collapsed-rollup states.
- Exports land in `/assets/design/` once we need them in-app.

Follow-ups parked in design (no issue yet):

- Tooltip primitive (kebab-menu language): `runner-mvp-design.pen` node `sE5dM`. Spec'd during issue #34 Phase 4 but deferred — v1 ships with native `title`. Pick this up when a sidebar/rail tooltip primitive is genuinely needed.
