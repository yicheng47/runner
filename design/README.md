# Design

Pencil (`.pen`) source files for the Runners UI. Open these in the Pencil app; they are encrypted on disk and not meant to be read as plain text.

Conventions:

Feature specs, since 2026-09-18, live one file per spec in `specs/<issue>-<slug>.pen`, mirroring `docs/features/`. A spec file starts from a copy of `runner.pen`'s tokens and the components it needs, so it is a snapshot of that day; Pencil cannot reference components across files. `runner.pen` stays the product canvas, screens plus `cmp/` components, and changes only when a shipped surface changes or a spec introduces a component that ships (`cmp/MarkPi` from #539 is on both). The lifecycle of a design (Jason, 2026-09-19): it is drawn and signed off in its spec file; when the feature ships, the design that was actually used is applied to the product screens in `runner.pen`, so the main file always shows the shipped app; anything in it that repeats or will be reused becomes a `cmp/` component in `runner.pen` rather than another hand-drawn copy; and only then does the spec move to `specs/archive/`. Archiving a spec whose design never reached the main screens loses the only picture of what shipped.

Spec frames designed before then stay on `runner.pen` in the spec rows, where the feature docs point at them by frame id, until their feature has shipped; then they move to an archive file under `specs/archive/`, one file per band of the canvas. An archive is made the same way as a spec file (a copy of `runner.pen` stripped to the frames and the components they instance), so frame ids do not change and an archived doc that names a frame id still resolves there. The first is `specs/539-pi-runtime.pen`.

Archives:

- `specs/archive/chat-feature-specs.pen` (2026-09-19): the chat band's feature-spec row, 30 frames and their labels: ⌘1–9 tab shortcuts (M6.15), sidebar entry points (M6.21), fork chat (60), pane × archives (567), split panes redesign (570), drag to reorder (568), terminal tab split (574), session status (347) with its approval-wait screens, header options and light variants, terminal-provided titles (587), provider icons (593), GitHub Copilot CLI (540), rail glyphs (606) and session liveness in the rail, the brand mark exploration, and status details (624). A label in `runner.pen`'s chat band points at the file.
- `specs/archive/493-windows-updates.pen` (2026-09-19): the four spec frames of Windows in-app updates (#493) and a snapshot of `cmp/UpdateDialog`, left over when `windows-updates.pen` was merged into `runner.pen`.

The macOS titlebar cluster (sidebar toggle, Previous page, Next page; #494) is recorded in `runner.pen`: the two arrows live inside `cmp/SidebarC`'s header row `sbDrag` (`sbBackIcon`, `sbNextIcon`) so every screen that instances the sidebar shows them, and the chrome spec is frame `YRWg3` ("Header navigation — #494 chrome spec") beside the #246 sidebar-toggle spec `w83yF`: control states, expanded/collapsed/fullscreen placement, and behavior notes.

Windows surfaces live in `runner.pen`'s WINDOWS band since 2026-09-19, when `windows-header.pen` (a title bar sketch) and `windows-updates.pen` (#493) were merged into the product canvas so that Windows screens instance the same sidebar and settings components as macOS ones; Pencil cannot share components across files, and the old files had redrawn them by hand. The shared title bar is the component `cmp/WinTitlebar` (`C1YbSg`): a 32 px row above the sidebar and workspace, a left cluster with the 28 px sidebar toggle and the previous and next arrows, the draggable title centred, and three 46 px caption buttons on the right; it is laid out, so it holds at any window width. A Windows screen is `cmp/WinTitlebar` plus an instance of `cmp/SidebarC` with its macOS title row `sbDrag` switched off, or of `cmp/SettingsNav` with its traffic lights switched off. It is separate from the macOS chrome.

Windows in-app updates (#493): the centered update dialog is the component `cmp/UpdateDialog` (`oUZ6u`), and the band holds the home window with the dialog opened from the sidebar icon (`GrhHR`), Settings → Updates in the Ready state (`X5YnsM`) and with the dialog open while downloading (`wFByp`), whose rows are `cmp/SettingsRow` instances. The dialog is the only update surface, opened from the sidebar icon or the Settings button. The spec frames (the hero's status line and button for the five states `f230`, the sidebar icon states `f266`, the dialog's four states `rZRA3`, the flow note `n300`) are in `specs/archive/493-windows-updates.pen` with their original ids.

Settings controls are components since 2026-09-19: `cmp/ToggleOn` (`BUCWv`), `cmp/ToggleOff` (`dkqPP`), `cmp/Button` (`IelRd`, a label and an icon that is off by default) and `cmp/SettingsRow` (`O75mx`: a title, a description and a slot for the control). `Settings — General` and the Windows Updates screens are built from them; the other Settings screens still draw their rows by hand. `cmp/SettingsNav` has no default selection: each instance sets the fill and border on its own item.

Plan usage (#706) is in `runner.pen` since 2026-09-23: `cmp/SidebarC`'s Settings row ends with the gauge button `sbUsageBtn` (`TrVD6`), the Runner update icon moved beside the Settings label, the popover is the component `cmp/UsagePopover` (`y655u`), and the chat band shows it open in `Runner chat — usage popover open (706)` (`Z2MEX`). The popover's gear has an `update_dot` that is off; it belongs to #533 and is drawn in `specs/533-agent-cli-updates.pen`.

- One `.pen` file per major surface (e.g. `home.pen`, `crew-editor.pen`, `runner-card.pen`).
- `runner-mvp-design.pen` is the historical MVP canvas; do not add new feature work to it.
- `chat-attention-indicators.pen` frame `R4LJz` contains the issue #285 working, unread, and collapsed-rollup states.
- Exports land in `/assets/design/` once we need them in-app.

Follow-ups parked in design (no issue yet):

- Tooltip primitive (kebab-menu language): `runner-mvp-design.pen` node `sE5dM`. Spec'd during issue #34 Phase 4 but deferred — v1 ships with native `title`. Pick this up when a sidebar/rail tooltip primitive is genuinely needed.
