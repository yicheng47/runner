# M4 surface inventory

M4 exit re-audit against the frozen React source of truth at `main` `276a3a479ddb3f11e4c682e2d032fbab8cc076ba` (2026-08-20), completed in M4.8 on 2026-08-21. Every row was compared across structure, flow, copy, states, and visual treatment. `design/runner.pen` remains a reference only under plan decision 1.

## Summary

Count line — done / partial / missing: **0 / 25 / 50 → 75 / 0 / 0**. The entry baseline's partial count here includes its 3 `functional-but-unstyled` rows plus 22 `partial` rows.

Rows marked done may rely on a deliberate deviation already approved in the registers in [impl_log.md](impl_log.md); those rows name the deviation instead of reopening it. No new partial or missing surface was found. The M4.8 Option-letter correction is the only new proposed register line because it intentionally fixes a bug shared by `main`: all 26 Option-letter chords now encode as Meta plus the plain ASCII letter (preserving Shift), so Option-composed letter glyphs are no longer enterable through a terminal pane.

## A. App shell, navigation, and global surfaces

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| A1 | Application bootstrap, routes, and global actions | `src/main.tsx`, `src/App.tsx` | `main.rs`, `bootstrap.rs`, `surfaces/app_shell.rs`, `keymap.rs` | done (M4.1/M4.6e) | Route ownership, Settings takeover, zoom, launch resume, grid hints, updater wiring, and secondary-window bootstrap all match the shipped flow. |
| A2 | Window chrome and app frame | `src/components/AppShell.tsx` | `mac_chrome.rs`, `surfaces/app_shell.rs`, `surfaces/windowing.rs` | done (M4.1/M4.6e) | Owned titlebar, traffic lights, drag/double-click/fullscreen behavior, menu, and header zones are present; the registered all-window restore extension is retained. |
| A3 | App theme, typography, spacing, and global scrollbars | `src/index.css`, `src/lib/settings.ts` | `theme.rs`, `app_settings.rs`, `ui/scrollbar.rs`, `runner-terminal/src/palette.rs` | done (M4.1/M4.2) | Four app variants, three terminal palettes, intent resolution, app fonts, zoom, token roles, and theme-aware scrollbars are wired live. |
| A4 | Runner app identity and icon | `src/assets/app-icon.png`, `design/app-icon.*` | `assets.rs`, `bootstrap.rs`, `script/bundle-mac` | done (M4.1/M4.6f) | The current mark is embedded in-app and the signed bundle/Dock resources use the current icon. |
| A5 | Persistent routed surfaces | `src/components/PersistentSurfaces.tsx` | `surfaces/app_shell.rs`, `surfaces/chat.rs`, `surfaces/mission_workspace.rs`, `runner-terminal/src/terminal.rs` | done (M4.3/M4.6e) | Process-wide PTYs survive routing and replay on attachment; the registered GPUI view-attachment lifecycle replaces retained DOM mounts. |
| A6 | Sidebar hierarchy, projects, pinned items, and root chats/missions | `src/components/Sidebar.tsx` | `surfaces/sidebar.rs`, `surfaces/sidebar_logic.rs` | done (M4.3) | Workspace entries, search/create, pinned and project/root nodes, rename, menus, archive, and drag/reparent/reorder match `main`; no folder surface was added. |
| A7 | Sidebar tab rows and attention rollups | `src/components/SidebarTabRow.tsx`, `ChatTabGroup.tsx` | `surfaces/sidebar.rs`, `surfaces/sidebar_logic.rs` | done (M4.3/M4.8) | Flat tab rows, Chat-route-only tab selection, focused-pane treatment, busy/unread rollups, watermarks, tooltips, and the hover/focus opacity reveal for row actions match `main`; mission project-scope selection remains independently active. |
| A8 | Sidebar collapse, resize, hover preview, and panel glyph | `src/components/AppShell.tsx`, `PanelToggleGlyph.tsx`, `useResizableWidth.ts` | `surfaces/app_shell.rs`, `app_settings.rs`, `keymap.rs` | done (M4.1) | Persisted width/collapse, hover preview, keep-open pin, glyph, and shortcut are present. |
| A9 | Command palette | `src/components/CommandPalette.tsx` | `surfaces/command_palette.rs` | done (M4.6c) | Search groups, ordering, keyboard loop, empty state, routes, and Settings actions match the frozen React surface. |
| A10 | Duplicate-subject ownership overlay | `src/components/DuplicateSubjectOverlay.tsx`, `src/lib/windowFocus.ts` | `ui/duplicate_subject_overlay.rs`, `surfaces/windowing.rs`, mission/chat surfaces | done (M4.5/M4.6e) | Subject reporting, primary-window focus, open-in-another-window, stay-here, and precedence are wired through the native window registry. |
| A11 | Global toast surface | `src/contexts/ToastContext.tsx` | `toast.rs`, `surfaces/app_shell.rs` | done (M4.1) | The single replaceable top toast supports info/success/error, timeout, dismissal, and replacement. |
| A12 | In-app update prompt | `src/components/UpdatePromptCard.tsx`, update context/hooks | `updater.rs`, `surfaces/settings/updates.rs`, AppKit menu | done (M4.6f) | The registered Sparkle deviation is authoritative: Sparkle's standard sheet and app-menu item replace the Tauri-only custom prompt/state machine. |

## B. Direct-chat surfaces

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| B1 | Direct-chat page and tab surface | `src/pages/RunnerChat.tsx` | `surfaces/chat.rs`, `surfaces/panes.rs` | done (M4.3/M4.6e) | Composition, retained PTY lifecycle, route/focus behavior, warnings, archived read-only mode, and multi-window ownership match. |
| B2 | Direct-chat topbar and group controls | `src/pages/RunnerChat.tsx` | `surfaces/chat.rs`, `ui/workspace_header.rs` | done (M4.3) | Sidebar/panel/layout controls, badge/status, pin/rename/archive menu, group stop/resume, warning states, and header treatment are present. |
| B3 | Runner/runtime details side panel | nested `RunnerSidePanel` in `RunnerChat.tsx` | `surfaces/chat.rs` | done (M4.3) | Resizable/collapsible details, runner/runtime identity, command/cwd/session key, prompt, and edit navigation match. |
| B4 | Persisted pane layout, focus, and split resizing | `ChatPaneGroup.tsx`, pane layout/geometry helpers | `pane_layout.rs`, `surfaces/panes.rs`, `surfaces/chat.rs` | done (M4.3) | Six layouts, focus, assignment-as-move, split sizes, resize ownership, hydration, pointer resizing, and chrome are implemented and tested. |
| B5 | Layout picker | `src/components/LayoutPicker.tsx` | `surfaces/panes.rs` | done (M4.3) | The six-icon grid, selected state, helper copy, outside/Escape dismissal, and pick behavior match. |
| B6 | Pane header and per-pane controls | `src/components/ChatPaneGroup.tsx` | `surfaces/panes.rs`, `ui/session_control.rs` | done (M4.3) | Badge/status, focus chrome, stop/resume/archive/menu controls, close behavior, and lifecycle locking match. |
| B7 | Empty-pane new-chat and close flow | `ChatPaneGroup.tsx`, `StartChatModal.tsx` | `surfaces/panes.rs`, `surfaces/start_chat.rs` | done (M4.3) | Empty copy/actions, pane-targeted creation, and tree-collapse close semantics match and are covered for two/three-way layouts. |
| B8 | Terminal canvas and direct terminal interaction | `src/components/RunnerTerminal.tsx` and terminal helpers | `terminal/element.rs`, `terminal_ime.rs`, `terminal_paste.rs`, `terminal_resize.rs`, `runner-terminal` | done (M4.7/M4.8) | Grid/procedural glyphs, keys/IME/paste, selection/copy, mouse/wheel reporting, resize/replay, OSC 8 and regex URLs with modifier-only repainting hover/click, Shift-preserving Meta encoding for all Option letters, and block-select crosshair are present; registered terminal deviations remain. |
| B9 | Session-ended and transition overlays | `src/components/SessionEndedOverlay.tsx` | `ui/session_overlay.rs`, `surfaces/chat_lifecycle.rs`, `surfaces/panes.rs` | done (M4.3/M4.6c) | Preserved-conversation card, Starting/Resuming/Archiving/ended precedence, Archive/Back/Resume actions, copy, and locking match. |
| B10 | Start Chat modal | `src/components/StartChatModal.tsx` | `surfaces/start_chat.rs`, shared fields/selects/overlay | done (M3.5/M4.3) | Runner/Direct modes, selectable runtime/model/effort, sticky title, project/default cwd precedence, browse, settings fallback, validation, persistence, and copy match. |
| B11 | Direct-chat actions and project placement | `RunnerChat.tsx`, `ui/PopoverMenu.tsx` | `surfaces/chat.rs`, `surfaces/sidebar.rs`, `ui/menu.rs` | done (M4.3) | Stop/resume all, pin, rename, archive, project movement, lifecycle disablement, and error rollback are present. |

## C. Mission surfaces

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| C1 | Mission workspace page | `src/pages/MissionWorkspace.tsx` | `surfaces/mission_workspace.rs` | done (M4.5/M4.6b) | Load/error/warning composition, paused semantics, ownership overlay, live/replay state, and terminal lifecycle match. |
| C2 | Mission topbar, feed/slot tabs, and mission actions | `src/pages/MissionWorkspace.tsx` | `surfaces/mission_workspace.rs`, `ui/workspace_header.rs` | done (M4.5/M4.6f) | Rail controls, Feed/slot tabs, remembered terminal, pin/rename/reset/archive, mission stop/resume, copy, and locking match. |
| C3 | Chat-style event feed | `src/components/EventFeed.tsx`, `src/lib/eventFeed.ts` | `surfaces/mission_feed.rs`, `surfaces/mission_workspace.rs` | done (M4.5) | Grouping, avatars, author/target/time/goal, divider, signals, new-message pill, autoscroll, and lagged replay/live merge match. |
| C4 | Markdown message body | `src/components/MessageBody.tsx` | `surfaces/mission_markdown.rs` | done (M4.5/M4.8) | Blocks, inline styles, code, tables, interactive links with brighter hover decoration, and narrow-width wrapping share one styled text layout; the registered dependency-free Markdown subset remains. |
| C5 | Ask-human choice card | `src/components/AskHumanCard.tsx` | `surfaces/mission_feed.rs`, `surfaces/mission_workspace.rs` | done (M4.5) | Choice, submit, pending/resolved states, author targeting, and copy match. |
| C6 | Mission channel composer and mention picker | `MissionInput.tsx`, `missionComposer.ts` | `surfaces/mission_composer.rs`, `surfaces/mission_workspace.rs` | done (M4.5) | Auto-growing IME field, picker, target chip, Enter/Shift-Enter, channel/direct copy, send/error state, and delivery gating match. |
| C7 | Runners rail | `src/components/RunnersRail.tsx` | `surfaces/mission_workspace.rs` | done (M4.5) | Lead, runtime/status/activity/presence, session key, selected state, slot open, collapse, and resizing match. |
| C8 | Mission metadata panel | `src/components/MissionMetaPanel.tsx` | `surfaces/mission_workspace.rs` | done (M4.5) | ID copy, effective goal, cwd reveal, crew link, relative time, and exact row treatment are present. |
| C9 | Inbox-blocked pill | `InboxBlockedPill.tsx`, `deliveryBlocked.ts` | `surfaces/mission_workspace.rs`, `surfaces/mission_composer.rs` | done (M4.5) | Placement, count, typing/busy gates, clear-input copy, and reconciliation behavior match the M3.6 router contract. |
| C10 | Reset confirmation | `src/components/MissionResetConfirm.tsx` | `surfaces/mission_workspace.rs`, `ui/overlay.rs` | done (M4.5) | Destructive copy, cancel/reset focus, submitting lock, error handling, and sized reset are present. |
| C11 | Start Mission modal | `src/components/StartMissionModal.tsx` | `surfaces/start_mission.rs`, `surfaces/mission_workspace.rs` | done (M4.5/M4.8) | Crew summary, title/goal, project/default cwd, browse, validation, count, grid sizing, and inert Advanced disclosure match; Feed-active starts prefer the last measured mission slot size. |

## D. Runners, crews, projects, and list surfaces

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| D1 | Paginated list-page scaffold | `PaginatedListPage.tsx`, list-control hooks | `ui/list.rs`, `list_controls.rs` | done (M4.4) | Header/CTA, debounce, counts, load/error/empty/no-match states, scrolling body, and pager match. |
| D2 | Runners list page | `src/pages/Runners.tsx` | `surfaces/runners.rs` | done (M4.4) | Search/page cards, runtime/chat action, command/activity/crew badges, menus, delete confirmation, and empty states match. |
| D3 | Runner detail page | `src/pages/RunnerDetail.tsx` | `surfaces/runners.rs` | done (M4.4) | Breadcrumb/header, Edit/Chat, prompt, memberships, cwd, activity, immutable details, and load/error states match. |
| D4 | Create Runner modal | `src/components/CreateRunnerModal.tsx` | `surfaces/runners.rs`, shared overlay/fields | done (M4.4) | All identity/runtime/command/model/permission/cwd/prompt fields, validation, defaults, and creation flow match. |
| D5 | Runner edit drawer | `src/components/RunnerEditDrawer.tsx` | `surfaces/runners.rs`, `ui/overlay.rs` | done (M4.4) | Drawer, immutable handle, default/override layering, model/effort, permission/cwd/prompt, reset, save, and delete match. |
| D6 | Crews list page and Create Crew modal | `src/pages/Crews.tsx` | `surfaces/crews.rs` | done (M4.4) | Search/page cards, purpose/member/lead presentation, open/delete confirmation, and all create fields match. |
| D7 | Crew editor/detail page | `src/pages/CrewEditor.tsx` | `surfaces/crews.rs` | done (M4.4) | Inline name/goal/conventions, purpose, Start Mission, ordered slots, summaries, and menus match. |
| D8 | Add Slot modal | `src/components/AddSlotModal.tsx` | `surfaces/crews.rs` | done (M4.4) | Runner search, handle, runtime/model/effort layering, disabled v0.x prompt override, validation, add, and edit modes match. |
| D9 | Crew slot reorder and lead/actions | `src/pages/CrewEditor.tsx` | `surfaces/crews.rs` | done (M4.4/M4.8) | Drag ordering, drag-handle tooltip, Set lead, Edit runner, Remove, menu treatment, and disabled states match. |
| D10 | Start Project modal and project context actions | `StartProjectModal.tsx`, project branches in `Sidebar.tsx` | `surfaces/sidebar.rs`, `surfaces/sidebar_logic.rs` | done (M4.3/M4.4) | Browse, basename-derived editable name, create/rename/delete/reorder, and project-scoped chat/mission starts match; no folder or Projects page was added. |
| D11 | Reusable empty-state card | `src/components/EmptyStateCard.tsx` | `ui/list.rs`, entity/settings surfaces | done (M4.2/M4.4) | Shared icon/title/description/action composition is used by the same no-data/no-match flows. |

## E. Settings surfaces

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| E1 | Settings takeover, navigation, and search | `src/pages/SettingsPage.tsx` | `surfaces/settings_page.rs`, `surfaces/app_shell.rs` | done (M4.6a) | Layered takeover, shared sidebar width, Back, grouped ten-pane nav, search/no-match, fallback routing, and focus behavior match. |
| E2 | General settings | `src/components/settings/GeneralPane.tsx` | `surfaces/settings_page.rs`, `app_settings.rs` | done (M4.6a) | Default crew/cwd, zoom, resume-on-launch, save timing, and honest unconsumed default-crew behavior match; stale Pencil-only rows stay absent. |
| E3 | Appearance settings | `src/components/settings/AppearancePane.tsx` | `surfaces/settings_page.rs`, `theme.rs` | done (M4.6a) | Auto/Light/Dark, four variant swatches, four app-font choices, persistence, and live application match. |
| E4 | Terminal settings | `src/components/settings/TerminalPane.tsx` | `surfaces/settings_page.rs`, `terminal/element.rs`, `runner-terminal` | done (M4.6a/M4.6f) | Theme, registered bundled-font choice, size, cursor, and live application match; the registered fixed 10,000-line scrollback deviation replaces the selector. |
| E5 | Keyboard shortcuts settings | `ShortcutsPane.tsx`, `src/lib/keymap.ts` | `surfaces/settings_page.rs`, `keymap.rs` | done (M4.6c/M4.7) | Search, record/conflict/unbind/restore/reset, all actions/scopes, and fixed New window/Copy rows use the same non-editable rendering. |
| E6 | Agents settings | `src/components/settings/AgentsPane.tsx` | `surfaces/settings/agents.rs` | done (M4.6d/M4.8) | Default/enabled agents, shell status/refresh, executable browse/reset, all runtime row states, Enter blur/commit, and entity isolation match. |
| E7 | MCP settings | `src/components/settings/McpPane.tsx` | `surfaces/settings/mcp.rs` | done (M4.6d/M4.8) | Binding/environment, four client rows, toggles, warnings, snippets/copy, Updating tone, wrapping, and binding-dir tooltip match. |
| E8 | Updates settings | `UpdatesPane.tsx`, update context/hooks | `surfaces/settings/updates.rs`, `updater.rs` | done (M4.6f/M4.8) | The registered slim Sparkle pane shows version, Check, automatic checks, and Last checked; visible-pane low-frequency rereads observe menu and scheduled checks, then pause when the pane is left. |
| E9 | Diagnostics settings | `src/components/settings/DiagnosticsPane.tsx` | `surfaces/settings/diagnostics.rs`, `logging.rs` | done (M4.6f) | Native file logging, panic integration, Finder reveal, and error state support the matching pane. |
| E10 | About settings | `src/components/settings/AboutPane.tsx` | `surfaces/settings/about.rs`, `assets.rs`, `version.rs` | done (M4.6f) | Icon/version/tagline, GitHub/docs/license/copyright, opener, and pane separation match. |
| E11 | Archived chats and missions | `src/components/settings/ArchivedPane.tsx` | `surfaces/settings/archived.rs` | done (M4.6d/M4.8) | Recency merge, search/filter, locale-aware times, read-only open, Restore, and Delete all match; the same deferred per-row-trash state remains absent. |
| E12 | Confirm dialog | `src/components/settings/ConfirmDialog.tsx` | `ui/overlay.rs`, settings/entity surfaces | done (M4.2/M4.6d) | Shared title/body/cancel/confirm/destructive/submitting contract is used by archived and page deletion flows. |
| E13 | Settings cards, rows, headers, and stepper | `src/components/settings/shared.tsx` | `ui/settings.rs`, `ui/field.rs` | done (M4.2) | Shared pane header/card/row/stepper structure and spacing are used across all panes. |

## F. Shared widget set

| ID | Surface | `main` reference | Native location | Status | Re-audit note |
|---|---|---|---|---|---|
| F1 | Button variants | `src/components/ui/Button.tsx` | `ui/button.rs` | done (M4.2) | Primary/default/danger variants, sizes, icons, disabled/focus/hover, keyboard activation, and spinner match. |
| F2 | Copy-value button | `src/components/ui/CopyValueButton.tsx` | `ui/copy_value_button.rs` | done (M4.2) | Clipboard write, icon, copied feedback, tooltip, focus, and accessible label are shared. |
| F3 | Field, label, input, textarea, and field error | `src/components/ui/Field.tsx` | `ui/field.rs` | done (M4.2) | IME single/multiline input, selection, auto-grow, label/subtitle/error, validation, disabled/focus, and app-font behavior match. |
| F4 | Model field | `src/components/ui/ModelField.tsx` | `ui/model_field.rs` | done (M4.2/M4.6f) | Free text, catalog suggestions, reset, disabled capture, and reuse in chat/runner/slot forms match. |
| F5 | Modal and drawer shells | `src/components/ui/Overlay.tsx` | `ui/overlay.rs` | done (M4.2) | Modal/drawer/confirm focus, Escape, backdrop, footer, widths, destructive states, and layering match. |
| F6 | Pager | `src/components/ui/Pager.tsx` | `ui/list.rs` | done (M4.2/M4.4) | Compact windowing, active page, ellipses, and boundary-disabled states match. |
| F7 | Popover menu | `src/components/ui/PopoverMenu.tsx` | `ui/menu.rs` | done (M4.2) | Anchoring, outside dismissal, focus/keyboard highlight, hover, destructive/disabled items, and shared triggers match. |
| F8 | Runner avatar and presence | `src/components/ui/RunnerAvatar.tsx` | `ui/avatar.rs` | done (M4.2) | Deterministic hue, initials/handle, sizes, lead, activity, and presence treatments match. |
| F9 | Runtime select | `src/components/ui/RuntimeSelect.tsx` | `ui/select.rs`, `ui/field.rs` | done (M4.2) | Catalog descriptions, availability, filtering, keyboard behavior, and reuse across forms match. |
| F10 | Search input | `src/components/ui/SearchInput.tsx` | `ui/list.rs`, `ui/field.rs` | done (M4.2) | Shortcut hint, clear, focus/key behavior, disabled state, and list/settings variants match. |
| F11 | Session controls | `src/components/ui/SessionControl.tsx` | `ui/session_control.rs`, `ui/session_overlay.rs` | done (M4.2/M4.3) | Stop/Resume/Resuming/Back variants, labels/icons/tooltips, and lifecycle disablement are shared. |
| F12 | Styled select | `src/components/ui/StyledSelect.tsx` | `ui/select.rs` | done (M4.2) | Form/settings sizes, swatches, keyboard selection, outside dismissal, disabled state, and focus match. |
| F13 | Toggle | `src/components/ui/Toggle.tsx` | `ui/toggle.rs` | done (M4.2) | Pointer/keyboard/focus/disabled semantics and settings styling match. |
| F14 | Tooltip | `src/components/ui/Tooltip.tsx` | `ui/tooltip.rs` | done (M4.2) | Delay, placement, shared chrome, stable IDs, and icon/copy/field usage match. |
| F15 | Working-directory field | `src/components/ui/WorkingDirField.tsx` | `ui/field.rs` (`BrowseField`/`WorkingDirField`) | done (M4.2) | Default-path display, single/multiline modes, browse, validation, truncation, and cross-form reuse match. |
| F16 | Runtime/model/effort options and selectable-agent filtering | `ui/runtimes.ts`, `useSelectableAgentOptions.ts` | backend runtime ops, `app_settings.rs`, `ui/select.rs`, form surfaces | done (M3.5/M4.2) | Option tables, availability, persisted enabled/default-agent policy, override layering, and reuse match. |
| F17 | Product scrollbars | `src/index.css` and xterm styling | `ui/scrollbar.rs`, theme tokens, terminal/chat/mission/list surfaces | done (M4.1/M4.2) | App and terminal tracks/thumbs are shared, theme-aware, hover-revealed, and used on product scroll regions. |

## Helper-module exhaustiveness audit

| `main` helper source group | Owning rows | Native counterpart | Status | Re-audit note |
|---|---|---|---|---|
| `useCurrentWindowFullscreen.ts`, `useResizableWidth.ts` | A2, A8, E1 | `mac_chrome.rs`, app shell/settings/chat/mission resize paths | done (M4.1/M4.6e) | Fullscreen state and constrained persisted drags have native event paths. |
| `useListControls.ts`, `listControls.ts` | D1, D2, D6 | `list_controls.rs`, `ui/list.rs` | done (M4.4) | Debounce, paging, reset, count, and query behavior match. |
| `useUpdateChecker.ts`, `UpdateContext.tsx` | A12, E8 | `updater.rs`, `settings/updates.rs`, AppKit menu | done (M4.6f/M4.8) | Sparkle owns scheduled/manual checks and standard UI under the registered deviation; Last checked rereads external changes. |
| `appZoom.ts`, `settings.ts`, `useStoredBool.ts` | A1, A3, E2–E6 | `app_settings.rs`, `theme.rs`, settings/keymap surfaces | done (M4.1/M4.6d) | Native persistence, normalization, live application, and registered preference migration behavior cover the helper contracts. |
| `autoResume.ts`, `launchResumeTrace.ts`, `launchDims.ts` | A1, B10, C1, C2, C11, E2 | bootstrap, session ops, chat/mission/start surfaces | done (M3.2/M4.6a/M4.8) | Claim take/clear, manual-vs-launch resume, tracing, persisted/hint/cached dimensions, and safe fallbacks match. |
| `archivingState.ts`, `directChatStatus.ts`, `sessionLifecycle.ts`, `useDelayedFlag.ts` | B1, B2, B9, B11, C1, C2 | `chat_lifecycle.rs`, chat/mission workspaces, session overlays | done (M4.3/M4.5) | Lifecycle aggregation, delayed overlays, precedence, and locks match. |
| `chatAttention.ts`, `groupPinning.ts`, `sidebarDnd.ts` | A6, A7 | `sidebar_logic.rs`, `sidebar.rs` | done (M4.3) | Attention priority/rollups, group pinning, validation, and DnD target rules match. |
| `chatTabs.ts`, `paneGeometry.ts`, `paneLayout.ts` | B4–B7 | `pane_layout.rs`, `surfaces/panes.rs` | done (M4.3) | Tree transforms, sizes, focus, assignments, close, and geometry are covered by native tests. |
| `deliveryBlocked.ts` | C9 | mission composer/workspace plus M3.6 router | done (M3.6/M4.5) | Count/gating/clear policy and reconciliation match. |
| `eventFeed.ts`, `missionComposer.ts`, `missionLastTerminal.ts`, `missionResume.ts`, `missionTabNavigation.ts` | C1–C6 | mission feed/composer/workspace modules | done (M4.5) | Projection/grouping, composer state, remembered terminal, resume, and cyclic navigation match. |
| `frontendLog.ts` | E9 | `logging.rs`, Diagnostics pane | done (M4.6f) | Native tracing files, panic hook, reveal, and surfaced errors replace webview logging. |
| `keymap.ts` | A1, A8, E5 | `keymap.rs`, Shortcuts pane | done (M4.6c/M4.8) | Registry, scopes, overrides/conflicts, fixed bindings, and immediate rebuilds match except registered product shortcut choices. |
| `projectScope.ts` | A6, B10, B11, C11, D10 | sidebar/start-chat/start-mission project paths | done (M4.3/M4.5) | Project-derived cwd/id precedence and placement actions match. |
| Terminal helpers (`terminalBlank`, `terminalPaste`, `terminalRegistry`, `terminalResize`, `terminalSizing`, `textureAtlas`, `useTerminalBg`, `windowSettle`) | B4, B8, C1, C2, E4 | `runner-terminal`, terminal element/IME/paste/resize/glyph modules, bridge | done (M2/M4.8) | Blank/replay, paste precedence, one-view bridge, resize chain, procedural atlas replacement, palette, and settle behavior are covered; registered GPUI-specific terminal differences remain. |
| `windowFocus.ts` | A10, B1, C1 | `surfaces/windowing.rs`, duplicate overlay, backend window registry | done (M4.6e) | Subject reporting, focus rank, route bootstrap, and native window focus/open match. |
| `api.ts`, `types.ts` | All rows | `runner-backend` ops/models plus app store | done (M4.6f) | UI contracts used by every surface are typed through Rust models/ops; backend parity was separately frozen at M3 completion. |
| `src/assets/app-icon.png` | A4 | `assets.rs`, bundle resources | done (M4.1/M4.6f) | In-app and bundle identities use the current asset. |
| `vite-env.d.ts` | Build typing only | Cargo/build-script compile-time contracts | done (not applicable) | It declares no product behavior; native compile-time equivalents require no surface port. |

## Pencil reconciliation retained by the exit audit

Pencil-only Settings Chat, Remember window position, flat MISSION/CHAT rails, folders, and a Projects page remain intentionally absent because they are not in frozen `main`. Agents and Updates remain in the ten-pane Settings navigation; update UI remains separate from About; the four shipped app themes and three terminal palettes remain authoritative over the Carbon-only canvas variables.
