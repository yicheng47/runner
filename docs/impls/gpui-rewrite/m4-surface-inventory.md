# M4 surface inventory

Entry inventory for M4, the GPUI UI rebuild to `main` parity. This audit is pinned to `origin/main` at `09532d2e77137ed388b2917fff21278083cd8257` (2026-08-18) and the native task branch's pre-inventory state. The React frontend was read with `git show origin/main:src/...`; the encrypted `design/runner.pen` canvas was inspected only through the Pencil MCP; the native side was read from `crates/runner-app` and `crates/runner-backend`.

## Summary

The inventory contains 75 current-`main` parity surfaces. Only 3 are functional-but-unstyled, 22 are partial, and 50 are missing.

| Status | Count | Meaning |
|---|---:|---|
| functional-but-unstyled | 3 | The native behavior is substantially present and covered, but its rendering or copy still needs the M4 rebuild. |
| partial | 22 | A native fragment or supporting behavior exists, but the current `main` surface or flow is incomplete. |
| missing | 50 | No native product surface exists. A ready backend operation does not make a surface present. |

The three functional-but-unstyled rows are all inside direct chat: persisted split layout and resizing, the six-preset layout picker, and empty-pane create/close behavior. The current native app is otherwise a narrow direct-chat skeleton. It has no runner or crew pages, no mission workspace or feed, no settings, no command palette, no project/pinned node tree, and no multi-window product UI.

The biggest implementation gaps are:

1. The app shell is not `main`'s shell. It uses a visible macOS titlebar titled “Runner Native”, a fixed flat sidebar, three keybindings, one hard-coded Tokyo Night-like palette, and no route model.
2. Every mission surface is absent. The base mission/session/event operations mostly exist, but the M3 router/inbox and human channel-feed work must land before the matching UI can be functional.
3. All runner, crew, project-entry, list, detail, and editor surfaces are absent even though most CRUD operations already exist.
4. All ten shipped settings panes are absent. Several visible panes also depend on native services still assigned to Phase 5: Sparkle, logging/reveal, URL opening, and window-state persistence.
5. `runner.pen` is a useful visual reference but not a complete current-product manifest. Its reusable components and direct-chat/mission/list frames are valuable; several navigation and settings frames predate the current React product and must not override `main`.

Surprising findings:

- The live node type is `project | tab | mission`; folders were retired. A project is the only container, and there is no Projects page on `main`. M4 must port project creation and project-scoped sidebar behavior, not invent folder rows or a project list/detail page.
- `main` currently ships four app palette variants: Runner/Carbon and Catppuccin Mocha for dark, Codex Light and Catppuccin Latte for light, selected through Auto/Light/Dark intent. Native `theme.rs` and `runner-terminal/src/palette.rs` still hard-code Tokyo Night colors.
- Pencil's `cmp/SettingsNav` includes a separate Chat pane and omits Agents and Updates. On `main`, the default-agent control lives in Agents, and Settings has ten panes with a separate Updates pane. The Pencil Chat pane is not an M4 parity target.
- Pencil's common sidebar examples show the older flat MISSION/CHAT rail, not `main`'s project/pinned/root node hierarchy. `main` is authoritative for its structure, behavior, and copy.

## Source-of-truth and classification rules

Decisions 1 and 3 in the program plan resolve conflicts: current `main`'s shipped React code is the source of truth on every axis — which product surfaces, flows, and copy exist, *and* their visual treatment. `design/runner.pen` is a reference only (tightened 2026-08-18): useful for design intent and naming, never an override. Where Pencil and `main` disagree in any respect, port `main`; a Pencil-only idea is not permission to add or change a product surface. The Pencil-conflict rows recorded below are documentation of that drift, not a to-do list.

The Pencil audit saw 68 top-level nodes and 8 reusable components: `cmp/InboxBlockedPill`, `cmp/SearchField`, `cmp/Pager`, `cmp/SidebarC`, `cmp/RunnerCardC`, `cmp/CrewCardC`, `cmp/ConfirmDialog`, and `cmp/SettingsNav`. The canvas variables cover the Carbon base (`bg`, `panel`, `raised`, sidebar colors, three text levels, accent/warn, Inter, and JetBrains Mono); the other shipped palette values must come from `main`'s `src/index.css` and `src/lib/settings.ts`.

Backend notes use these labels:

- **Ready**: an operation already exists in `crates/runner-backend` and can serve the UI.
- **M3 pending**: the program already queues the missing backend behavior in the remaining M3 slices.
- **Phase 5**: the visible surface depends on a native replacement for a Tauri-provided app service.
- **UI-local**: state or behavior lives in the frontend on `main`; native needs an equivalent but no domain-backend port.

## A. App shell, navigation, and global surfaces

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| A1 | Application bootstrap, routes, and global actions | `src/main.tsx`, `src/App.tsx`; represented indirectly by every full-app Pencil frame | `crates/runner-app/src/main.rs`, `bootstrap.rs` | partial | Native boots the shared core and direct sessions only. It lacks the seven-page route/state model, settings takeover, global zoom, launch auto-resume, mission grid hint, update listener, and secondary-window route bootstrap. Auto-resume command seams and mission grid hint are **M3 pending**; window bootstrap is **Phase 5**. |
| A2 | Window chrome and app frame | `src/components/AppShell.tsx`; Pencil `Header — codex style` and `Sidebar toggle — #246 chrome spec` | `crates/runner-app/src/main.rs` | partial | Native opens one 1200×760 window with the visible system titlebar “Runner Native”. It has no hidden-titlebar traffic-light layout, drag regions, double-click zoom, fullscreen-aware chrome, product menu, or `main` header zones. The pulse window patterns remain the implementation reference. |
| A3 | App theme, typography, spacing, and global scrollbars | `src/index.css`, `src/lib/settings.ts`; Pencil variables plus `DS · Option 3 — Carbon & Plasma` and `DS · Option 4 — Codex Light` | `crates/runner-app/src/theme.rs`, `crates/runner-terminal/src/palette.rs`, fixed constants in `terminal_element.rs` | partial | Native hard-codes Tokyo Night colors, Menlo 13 px, and six shell colors. It lacks the full token roles, Auto/Light/Dark resolution, four current app variants, app-font selection, zoom, and product scrollbar treatment. This is **UI-local** settings state. |
| A4 | Runner app identity and icon | `src/assets/app-icon.png`, `design/app-icon.svg`, `design/app-icon.png`; the mark appears throughout Pencil | absent | missing | Reuse the current Runner identity. In-app mark/chrome belongs in M4.1; Dock/bundle resource wiring remains the decision-10 nightly packaging work in **Phase 5**. |
| A5 | Persistent routed surfaces | `src/components/PersistentSurfaces.tsx`; no dedicated Pencil spec | absent | missing | `main` keeps direct-chat and mission terminals mounted across routes and layers Settings above them. Native retains terminal models but renders only the active direct tab, so there is no equivalent mount/visibility/focus contract. |
| A6 | Sidebar hierarchy, projects, pinned items, and root chats/missions | `src/components/Sidebar.tsx`; Pencil `cmp/SidebarC` and full-app frames show an older flat rail | `crates/runner-app/src/sidebar.rs` | partial | Native is a fixed 248 px “DIRECT CHAT TABS” list. Missing: Workspace runner/crew entries, search/brand trigger, pinned section, project containers, root chats/missions, create menu, inline rename, context menus, drag/reparent/reorder, archive, and Settings/update footer. Node/project/session/mission mutation ops are **Ready**. There are no folder nodes. |
| A7 | Sidebar tab rows, multi-pane accordion, and attention rollups | `src/components/SidebarTabRow.tsx`, `src/components/ChatTabGroup.tsx`; Pencil `Tab accordion — sidebar spec` and `Chat attention indicators — Issue #285` | `crates/runner-app/src/sidebar.rs` | partial | Native shows only label plus running/stopped and pane count. It lacks single-versus-group row chrome, accordion collapse, focused pane treatment, kebab actions, working/unread indicators, collapsed-container rollups, and viewed-watermark updates. Watermark/node ops are **Ready**; stale-unread cleanup is **M3 pending**. |
| A8 | Sidebar collapse, resize, hover preview, and panel glyph | `src/components/AppShell.tsx`, `src/components/PanelToggleGlyph.tsx`, `src/hooks/useResizableWidth.ts`; Pencil `Sidebar toggle — #246 chrome spec` | absent | missing | Must match the titlebar control, persisted width/collapse state, left-edge hover preview, keep-open pin, and shortcut. This is **UI-local**. |
| A9 | Command palette | `src/components/CommandPalette.tsx`; no dedicated Pencil frame | absent | missing | Search/group/open chats, missions, runners, crews, and Settings with keyboard selection and `main`'s empty states. List/read ops are mostly **Ready**; pagination is not required for the palette's bounded result sets. |
| A10 | Duplicate-subject ownership overlay | `src/components/DuplicateSubjectOverlay.tsx`, `src/lib/windowFocus.ts`; no dedicated Pencil frame | absent; registry only in `crates/runner-backend/src/windows.rs` | missing | The shared primary-window registry is **Ready**, but native never reports subjects/focus and cannot open/focus another product window. The “Open in another window” overlay and exact actions are missing; the native window adapter is **Phase 5** work required before this surface can function. |
| A11 | Global toast surface | `src/contexts/ToastContext.tsx`; no Pencil frame | bottom error strip in `crates/runner-app/src/main.rs` | partial | Native can show one undismissable error bar. It lacks `main`'s single replaceable info/success/error toast slot, timeout, dismiss, and top placement. This is **UI-local**. |
| A12 | In-app update prompt | `src/components/UpdatePromptCard.tsx`, `src/contexts/UpdateContext.tsx`, `src/hooks/useUpdateChecker.ts`; Pencil `Spec — In-app update prompt (pill ⇄ hover card)` | absent | missing | Port the pill, hover card, dismiss/automatic-update controls, progress, and restart state only on top of the Sparkle/native updater contract. That service is **Phase 5**, not an M4-local fake. |

## B. Direct-chat surfaces

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| B1 | Direct-chat page and tab surface | `src/pages/RunnerChat.tsx`; Pencil `Runner direct chat` and its stopped/resuming/panel-collapsed/archiving variants | `crates/runner-app/src/main.rs`, `panes.rs`, `chat.rs` | partial | A direct tab can render and attach PTYs, but the surface does not match `main`'s composition, spacing, copy, lifecycle, navigation, or retained-route behavior. Direct-session read/start/resume ops are **Ready**. |
| B2 | Direct-chat topbar and group controls | `src/pages/RunnerChat.tsx`; Pencil direct-chat frames | `crates/runner-app/src/panes.rs` | partial | Native has a plain label, “Layout”, and “New tab ⌘T”. Missing sidebar/rail toggles, Chat badge/status metadata, pin/rename/archive menus, stop/resume group controls, warnings, and correct button/icon chrome. |
| B3 | Runner/runtime details side panel | nested `RunnerSidePanel` in `src/pages/RunnerChat.tsx`; Pencil direct-chat RUNNER panel | absent | missing | Port resizable/collapsible panel, runner/runtime identity, command/cwd/session-key copy affordances, system prompt, and edit entry. Runner/session reads are **Ready**. |
| B4 | Persisted pane layout, focus, and split resizing | `src/components/ChatPaneGroup.tsx`, `src/lib/paneLayout.ts`, `src/lib/paneGeometry.ts`; Pencil 2-pane, 3-pane, and empty-pane frames | `crates/runner-app/src/pane_layout.rs`, `panes.rs`, `chat.rs` | functional-but-unstyled | All six layout shapes, focused-pane tracking, assignment-as-move, per-split sizes, one resize owner, tab rehydration, and drag resize are present. Tests cover the layout model, per-split size round-trip, and resize ownership; M4 must restyle the walking-skeleton gutters/borders and exercise the pointer drag and exact keyboard-focus behavior during UI validation. |
| B5 | Layout picker | `src/components/LayoutPicker.tsx`; Pencil `Layout picker popup` | `crates/runner-app/src/panes.rs` | functional-but-unstyled | The six presets work and persist, but native renders text tiles instead of `main`'s icon grid and uses drifted helper copy (“Drag pane dividers to resize” versus the product's current copy). |
| B6 | Pane header and per-pane controls | `src/components/ChatPaneGroup.tsx`; Pencil split frames | `crates/runner-app/src/panes.rs` | partial | Native grouped panes show a dot, title, and close only for an empty pane. Missing Chat badge, status semantics, per-pane stop/resume/archive, action menu, exact focus chrome, and lifecycle-disabled states. Session control ops are **Ready**. |
| B7 | Empty-pane new-chat and close flow | `src/components/ChatPaneGroup.tsx`, `src/components/StartChatModal.tsx`; Pencil `Runner direct chat — 2-pane split, empty pane` | `crates/runner-app/src/panes.rs`, `chat.rs`, `start_chat.rs` | functional-but-unstyled | New Chat targets the pane; close collapses the tree without killing the session and is covered for two/three-way layouts. Restyle and align copy—the native-only “or pick a chat from the sidebar” instruction is not the current empty-pane design. |
| B8 | Terminal canvas and direct terminal interaction | `src/components/RunnerTerminal.tsx` plus terminal helper modules; Pencil terminal areas in direct-chat frames | `crates/runner-app/src/terminal_element.rs`, `terminal_ime.rs`, `terminal_resize.rs`, `chat.rs`; model in `crates/runner-terminal` | partial | Native has live grid rendering, scroll, raw keys, text paste, IME/candidate bounds, replay, resize ownership, and resume seams. Missing current theme/font/size/scrollback wiring, Cmd-click URLs, clipboard image and copied-file-path paste, app wake/window-settle handling, and some `main` focus/visibility behavior. Image paste backend exists; clipboard file paths and wake bridge are **M3 pending**; URL opening is **Phase 5**. |
| B9 | Session-ended and transition overlays | `src/components/SessionEndedOverlay.tsx`; Pencil stopped/resuming/archiving frames | stopped/crashed Resume bars in `crates/runner-app/src/panes.rs` | partial | Native exposes only “Chat stopped/crashed” plus Resume, sometimes below a still-mounted terminal. Missing preserved-conversation card and exact copy, Starting/Resuming/Archiving overlays, Archive and Back actions, progress locking, and duplicate/lifecycle precedence. Lifecycle ops are mostly **Ready**. |
| B10 | Start Chat modal | `src/components/StartChatModal.tsx`; no dedicated Pencil modal frame | `crates/runner-app/src/start_chat.rs`, `modal_text_input.rs` | partial | Runner/Direct modes, discovery-backed agent/model/effort choices, sticky derived title, cwd browse, mode persistence, spawn, and rename exist. Missing project scoping/project-derived cwd, `main`'s default-agent/default-working-dir settings contract, settings navigation from empty-agent state, exact shared widgets, and full copy/style parity. Runtime/session/project ops are **Ready**. |
| B11 | Direct-chat actions and project placement | `src/pages/RunnerChat.tsx`, `src/components/ui/PopoverMenu.tsx`; Pencil direct-chat kebab/menu states | only resume and start-time rename in `crates/runner-app/src/chat.rs`, `start_chat.rs` | partial | Missing stop, stop/resume all, pin/unpin, rename after creation, archive/archive-all, project move, and corresponding optimistic/disabled/error states. Backend session/node/project operations are **Ready**. |

## C. Mission surfaces

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| C1 | Mission workspace page | `src/pages/MissionWorkspace.tsx`; Pencil mission workspace state family | absent | missing | Port loading/error/warning states, workspace composition, mission-wide paused semantics, duplicate-subject handling, and retained terminal lifecycle. Base mission attach/get/list and session operations are **Ready**. |
| C2 | Mission topbar, feed/slot tabs, and mission actions | `src/pages/MissionWorkspace.tsx`; Pencil inbox-blocked/resuming/stopped/archiving workspace frames | absent | missing | Includes sidebar and right-panel toggles, feed tab, opened-slot tabs, last-terminal behavior, pin/rename/reset/archive menu, stop/resume mission controls, and exact lifecycle copy. Most domain ops are **Ready**; launch auto-resume/helper parity is **M3 pending**. |
| C3 | Chat-style event feed | `src/components/EventFeed.tsx`, `src/lib/eventFeed.ts`; Pencil `Feed · chat-style messages` and `Chat-style feed notes` | absent | missing | Port grouping, avatars, author/target/time/goal treatments, mission divider, signal disclosure, new-message pill, autoscroll, and replay/live merge. Event replay and signal posting exist; human channel messages and the remaining feed backend are **M3 pending**. |
| C4 | Markdown message body | `src/components/MessageBody.tsx`; visible in Pencil feed messages | absent | missing | Port the exact Markdown/code/link presentation used by the feed. URL opening is a **Phase 5** dependency; rendering is otherwise **UI-local**. |
| C5 | Ask-human choice card | `src/components/AskHumanCard.tsx`; Pencil `Feed · chat-style messages` NEEDS YOUR INPUT card | absent | missing | Port choice selection, response submission, resolved state, and target copy. Human-signal post/replay is **Ready**; verify it against the remaining M3 feed port. |
| C6 | Mission channel composer and roster mention picker | `src/components/MissionInput.tsx`, `src/lib/missionComposer.ts`; Pencil `Feed · channel composer (378)`, `Composer · @ mention picker`, and targeted state | absent | missing | Port auto-growing IME input, `@` picker, target chip, Enter/Shift-Enter behavior, channel/direct message copy, and send state. `mission_post_human_message` and channel semantics are **M3 pending**. |
| C7 | Runners rail | `src/components/RunnersRail.tsx`; Pencil RUNNER SESSIONS rail in workspace frames | absent | missing | Port lead marker, runtime/status/activity/presence, session key, slot open action, and selected state. Session/slot reads and activity snapshot are **Ready**. |
| C8 | Mission metadata panel | `src/components/MissionMetaPanel.tsx`; represented inside the mission workspace design family but not as a standalone Pencil component | absent | missing | Port ID copy, effective goal, cwd reveal, crew link, and relative start time. Mission/crew reads are **Ready**; Finder reveal/opening is **Phase 5**. |
| C9 | Inbox-blocked pill | `src/components/InboxBlockedPill.tsx`, `src/lib/deliveryBlocked.ts`; Pencil `cmp/InboxBlockedPill`, pill states, and blocked workspace | absent | missing | Port floating placement, counts, typing/busy gating, “Clear input ↵”, and its exact clear rules. The input latch exists, but router reservation/outbox/reconciliation/blocked-delivery events are **M3 pending**. |
| C10 | Reset confirmation | `src/components/MissionResetConfirm.tsx`; Pencil `Mission workspace — reset confirm` and `cmp/ConfirmDialog` | absent | missing | Port exact destructive copy, cancel/reset actions, and submitting state. Reset backend is **Ready**. |
| C11 | Start Mission modal | `src/components/StartMissionModal.tsx`; Pencil `Start Mission modal` | absent | missing | Port crew picker/lead-worker summary, title, goal, project/default cwd, browse, launchability validation, session count, grid sizing, and the current inert Advanced disclosure. Mission start, crew/slot/project reads are **Ready**; mission grid-hint parity is **M3 pending**. |

## D. Runners, crews, projects, and list surfaces

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| D1 | Paginated list-page scaffold | `src/components/PaginatedListPage.tsx`, `src/hooks/useListControls.ts`; Pencil Runners/Crews frames and honest-scroll pager spec | absent | missing | Port header/CTA, debounced search, result count, loading/error/empty/no-match states, scroll body, and bottom pager. Runner/crew page-shaped operations are **M3 pending** even though raw list operations exist. |
| D2 | Runners list page | `src/pages/Runners.tsx`; Pencil Runners default/filtered/no-match/page-1 frames and `cmp/RunnerCardC` | absent | missing | Port search, pagination, runner cards, runtime/chat affordance, command/activity/crew badges, edit/delete menu, and all empty states. Runner CRUD/activity is **Ready**; paged query adapter is **M3 pending**. |
| D3 | Runner detail page | `src/pages/RunnerDetail.tsx`; no matching Pencil frame | absent | missing | Port breadcrumb/runtime header, Edit and Chat now, system prompt, crew memberships, chat cwd, activity stats, immutable details, and loading/error states without inventing a new design. Runner/activity/membership reads are **Ready**. |
| D4 | Create Runner modal | `src/components/CreateRunnerModal.tsx`; no matching Pencil frame | absent | missing | Port handle, display name, agent/runtime, command/args, model, permission mode, cwd, system prompt, validation, and create flow using `main` copy. Runner/runtime operations are **Ready**. |
| D5 | Runner edit drawer | `src/components/RunnerEditDrawer.tsx`; no matching Pencil frame | absent | missing | Port drawer shell, immutable handle, runtime/default/override logic, command/args, model/effort, permission, cwd, system prompt, delete/update behavior, and reset rules. Backend is **Ready**. |
| D6 | Crews list page and Create Crew modal | `src/pages/Crews.tsx`; Pencil `Crews — search · page 2` and `cmp/CrewCardC` | absent | missing | Port list/search/pagination, purpose/member/lead cards, open/delete double confirm, and create fields for name, purpose, default goal, and team conventions. Crew CRUD is **Ready**; pagination is **M3 pending**. |
| D7 | Crew editor/detail page | `src/pages/CrewEditor.tsx`; no matching Pencil frame | absent | missing | Port inline name/default-goal/team-conventions editing, purpose, Start Mission entry, ordered slot list, lead/runtime/model/effort summaries, and action menus. Crew/slot operations are **Ready**. |
| D8 | Add Slot modal | `src/components/AddSlotModal.tsx`; no matching Pencil frame | absent | missing | Port runner search/picker, slot handle, runtime/model/effort configuration, the disabled v0.x system-prompt override, validation, and add/edit modes. Slot/runner/runtime operations are **Ready**. |
| D9 | Crew slot reorder and lead/actions | `src/pages/CrewEditor.tsx`; no matching Pencil frame | absent | missing | Port drag ordering, Set lead, Edit runner, Remove, disabled states, and exact menu behavior. Backend is **Ready**. |
| D10 | Start Project modal and project context actions | `src/components/StartProjectModal.tsx`, project branches in `src/components/Sidebar.tsx`; no matching Pencil modal/frame | absent | missing | Port directory browse, basename-derived editable name, create, rename, delete, reorder, “New chat in project”, and “New mission in project”. Project/node operations are **Ready**. There is deliberately no Projects page or folder surface on `main`. |
| D11 | Reusable empty-state card | `src/components/EmptyStateCard.tsx`; Pencil no-match/list states | absent | missing | Port its icon/title/description/action composition and use it where `main` does. This is **UI-local**. |

## E. Settings surfaces

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| E1 | Settings takeover, navigation, and search | `src/pages/SettingsPage.tsx`; Pencil settings frame family and `cmp/SettingsNav` | absent | missing | Port full-window takeover without unmounting terminals, shared resizable sidebar width, Back to app, pane-label search/no-match state, grouped ten-pane navigation, direct-route fallback, and current nav labels. Pencil's Chat item is superseded; Agents and Updates must come from `main`. |
| E2 | General settings | `src/components/settings/GeneralPane.tsx`; Pencil `Settings — General` | absent | missing | Port default crew, default cwd, app zoom, and Resume running agents on launch. Default crew remains persisted but unconsumed on `main` and must remain equally honest. Launch claim take/clear/automatic-resume seams are **M3 pending**. Pencil's Remember window position row is not shipped. |
| E3 | Appearance settings | `src/components/settings/AppearancePane.tsx`; Pencil `Settings — Appearance` | absent | missing | Port Auto/Light/Dark, two current light variants, two current dark variants, swatches, and Inter/Geist/Roboto/System UI app fonts with live application. State is **UI-local**. |
| E4 | Terminal settings | `src/components/settings/TerminalPane.tsx`; Pencil `Settings — Terminal` | absent | missing | Port Runner/Catppuccin Mocha/Solarized Dark terminal themes, six font choices, 10–20 px sizing, block/underline/bar cursor, and 1k/5k/10k/50k scrollback with live terminal updates. Native currently fixes all of these. |
| E5 | Keyboard shortcuts settings | `src/components/settings/ShortcutsPane.tsx`, `src/lib/keymap.ts`; Pencil `Settings — Keyboard shortcuts` | absent | missing | Port searchable action list, record/rebind, conflict resolution, unbind, restore/reset, fixed bindings, and all current app/chat/mission/pane/zoom actions. Native currently binds only Cmd-Q, Cmd-T, and terminal Cmd-V. |
| E6 | Agents settings | `src/components/settings/AgentsPane.tsx`; Pencil `Settings — Agents` and `Spec — Agent runtime row states` | absent | missing | Port default agent, enable toggles, login-shell status/refresh, executable override browse/reset, and detected/override/not-found/checking/timeout/invalid states for every catalog runtime. Runtime catalog/status/override/refresh is **Ready** from M3.5. |
| E7 | MCP settings | `src/components/settings/McpPane.tsx`; Pencil `Settings — MCP` | absent | missing | Port binding/environment status, Claude/Codex/Qoder/TRAE client rows, toggles, warnings, and manual snippets/copy. MCP operations are **Ready**. |
| E8 | Updates settings | `src/components/settings/UpdatesPane.tsx`, `src/contexts/UpdateContext.tsx`; no dedicated Pencil pane, with older states folded into Pencil About specs | absent | missing | Port current version/state/progress, check/download/restart, and auto-install toggle only against Sparkle. This is a **Phase 5** service-dependent surface. |
| E9 | Diagnostics settings | `src/components/settings/DiagnosticsPane.tsx`; Pencil `Settings — Diagnostics` | absent | missing | Port Reveal logs in Finder and error handling after native tracing/file logging exists. Logging, reveal, and crash/panic integration are **Phase 5**. |
| E10 | About settings | `src/components/settings/AboutPane.tsx`; Pencil `Settings — About` and older About update-state spec | absent | missing | Port app icon/version/tagline, GitHub, documentation, license, and copyright. Opening links is **Phase 5**. Keep updater controls in Updates as `main` now does. |
| E11 | Archived chats and missions | `src/components/settings/ArchivedPane.tsx`; Pencil `Settings — Archived` and Delete-all double-confirm spec | absent | missing | Port merged list, search, All/Missions/Chats filters, read-only open, Restore, and Delete all. Archived mission/session list, restore, and delete operations are **Ready**. |
| E12 | Confirm dialog | `src/components/settings/ConfirmDialog.tsx`; Pencil `cmp/ConfirmDialog` | absent | missing | Port generic title/body/cancel/confirm/destructive/submitting behavior and use it for archived delete-all and page delete flows. |
| E13 | Settings cards, rows, headers, and stepper | `src/components/settings/shared.tsx`; repeated throughout Pencil settings frames | absent | missing | Port `PaneHeader`, `SettingsCard`, `SettingsRow`, and `Stepper` before the pane implementations so spacing and control alignment do not drift pane by pane. |

## F. Shared widget set

| ID | Surface | `main` reference and Pencil coverage | Native location | Status | Parity and backend notes |
|---|---|---|---|---|---|
| F1 | Button variants | `src/components/ui/Button.tsx`; repeated across Pencil frames | ad hoc clickable `div`s in `panes.rs`, `sidebar.rs`, and `start_chat.rs` | partial | Native has behavior-specific buttons but no shared primary/default/danger sizing, disabled/focus, or icon treatment. Consolidation is justified here because every later M4 stage consumes it. |
| F2 | Copy-value button | `src/components/ui/CopyValueButton.tsx`; copy affordances appear in Pencil detail/rail panels | absent | missing | Port icon, copied feedback, tooltip/accessible label, and clipboard behavior. |
| F3 | Field, label, input, textarea, and field error | `src/components/ui/Field.tsx`; repeated in Pencil forms | `crates/runner-app/src/modal_text_input.rs` and local Start Chat field wrappers | partial | A capable single-line IME field exists, but there is no shared field contract, multiline textarea, standard error/subtitle/disabled/focus states, or app-font integration. Generalize without regressing composition. |
| F4 | Model field | `src/components/ui/ModelField.tsx`; model controls appear in Start Chat/settings designs | custom model input/menu in `crates/runner-app/src/start_chat.rs` | partial | Start Chat supports free text plus catalog suggestions, but the behavior/style is local and not reusable by runner/slot forms. |
| F5 | Modal and drawer shells | `src/components/ui/Overlay.tsx`; Pencil Start Mission/reset/confirm examples | Start Chat overlay in `crates/runner-app/src/start_chat.rs` | partial | Native has one fixed-size modal overlay. Missing shared focus/escape/backdrop/footer/width behavior and the drawer shell used by runner editing. |
| F6 | Pager | `src/components/ui/Pager.tsx`; Pencil `cmp/Pager` and `Pager — windowing states` | absent | missing | Port the exact compact page windowing and disabled states after paginated M3 operations land. |
| F7 | Popover menu | `src/components/ui/PopoverMenu.tsx`; kebab menus throughout Pencil direct chat/list frames | absent | missing | Required for sidebar, chat, runner, crew, and mission actions; include anchoring, outside dismiss, keyboard behavior, destructive styling, and disabled items. |
| F8 | Runner avatar and presence | `src/components/ui/RunnerAvatar.tsx`; avatars/presence in Pencil feed, rail, and cards | absent | missing | Port deterministic hue, initials/handle, size variants, lead and presence treatments. |
| F9 | Runtime select | `src/components/ui/RuntimeSelect.tsx`; Agent selectors in Pencil forms/settings | Start Chat's local picker in `crates/runner-app/src/start_chat.rs` | partial | Catalog-backed selection works in one modal, but shared descriptions, availability, disabled-agent filtering, and reuse in runner/slot forms are absent. |
| F10 | Search input | `src/components/ui/SearchInput.tsx`; Pencil `cmp/SearchField` | absent | missing | Port shortcut hint, clear action, focus/key handling, disabled state, and exact list/settings usage. |
| F11 | Session controls | `src/components/ui/SessionControl.tsx`; Stop/Resume/Back controls in Pencil chat/mission states | Resume buttons in `crates/runner-app/src/panes.rs` | partial | Only direct Resume exists. Missing shared Stop, Resuming, Back, icon/label variants, and lifecycle-disabled behavior. Backend kill/resume is **Ready**. |
| F12 | Styled select | `src/components/ui/StyledSelect.tsx`; repeated across Pencil settings | Start Chat's local picker in `crates/runner-app/src/start_chat.rs` | partial | A modal-local menu exists but not the shared form/select behavior, swatches, keyboard interaction, or settings sizing. |
| F13 | Toggle | `src/components/ui/Toggle.tsx`; settings and update designs | absent | missing | Required for General, Agents, Updates, and MCP with the same keyboard/focus/disabled semantics as `main`. |
| F14 | Tooltip | `src/components/ui/Tooltip.tsx`; hover guidance implicit in Pencil specs | absent | missing | Required by icon-only shell/sidebar/chat/settings controls and copy feedback. |
| F15 | Working-directory field | `src/components/ui/WorkingDirField.tsx`; Start Mission/Project cwd fields in Pencil | local cwd input and GPUI directory prompt in `crates/runner-app/src/start_chat.rs` | partial | Native browse works in Start Chat only. Missing shared default-path behavior, single-line mode, validation/presentation, and reuse across runner/project/mission/settings. |
| F16 | Runtime/model/effort option data and selectable-agent filtering | `src/components/ui/runtimes.ts`, `src/components/ui/useSelectableAgentOptions.ts`; Pencil Agent/model/effort states | catalog in `crates/runner-backend/src/ops/runtime.rs`, consumed locally by `start_chat.rs` | partial | Backend tables and availability are **Ready**, and Start Chat filters selectable runtimes. Missing native persisted enable/disable/default-agent policy and reuse across every form. |
| F17 | Product scrollbars | `src/index.css` plus xterm scrollbar styling; visible throughout Pencil scrollable frames | GPUI/default overflow rendering only | missing | Implement app and terminal scrollbars as shared theme-aware chrome; do not treat platform defaults as parity. |

## Pencil reconciliation: do not turn canvas drift into product work

| Pencil item | Current `main` result | M4 disposition |
|---|---|---|
| `Settings — Chat` and the Chat row in `cmp/SettingsNav` | No Chat pane exists. Default agent is in `AgentsPane`; Start Chat reads that setting. | Do not add a Chat pane. Use the visual nav treatment only, with `main`'s ten entries. |
| `cmp/SettingsNav` lacks Agents and Updates | `SettingsPage.tsx` ships both panes. | Add the shipped entries from `main`; do not preserve the stale nav information architecture. |
| General “Remember window position” | `GeneralPane.tsx` explicitly says this row is not shipped there; resume-on-launch is shipped instead. | Do not add the row in M4. Native window-state restore stays Phase 5 unless `main` changes first. |
| About frame owns update controls | `main` separates `AboutPane` and `UpdatesPane`. | Keep the visual hero/state references, but preserve the current pane split and copy. |
| Flat MISSION/CHAT sidebars in full-app frames | `Sidebar.tsx` ships pinned items, project containers, and root chats/missions over `project`, `tab`, and `mission` nodes. | Port the current node behavior from `main`; use Pencil only for tokens and row-level treatments that still match. |
| No Runner Detail, Crew Editor, Start Chat, Start Project, Create/Edit Runner, Add Slot, command palette, duplicate overlay, toast, or Updates-pane frame | All are current `main` surfaces. | Copy `main` faithfully; record visual QA against the shipping app because Pencil cannot supply a matching frame. |
| Carbon-only canvas variables | `main` ships four app variants and three terminal palettes. | Seed shared token roles from Pencil/Carbon, then port every current value and resolver from `main`. Do not retain native Tokyo Night as an extra product theme. |

## Backend readiness by surface family

| Surface family | Already available | Still required before/with M4 |
|---|---|---|
| Direct chat | Direct session list/get/start/resume/kill/archive/unarchive/delete/rename/pin/project placement; terminal output/input/resize/replay/activity; runtime catalog and overrides; node tab persistence | M3 clipboard file-path paste, auto-resume queue consumer/clear seam, macOS wake bridge, stale unread cleanup; M4 UI persistence and controls; Phase 5 URL/window services |
| Sidebar/projects | Node list/rename/upsert/delete/move/reorder pinned/set pinned/mark viewed/import; project CRUD/reorder/delete; session and mission placement | M3 stale unread cleanup; the complete native tree, drag/drop, menus, attention, and search |
| Runners/crews | Runner CRUD/activity/crew memberships; crew CRUD; slot CRUD/lead/reorder; runtime catalog | M3 page-shaped pagination/search operations; all UI |
| Missions | Mission list/summary/get/start/attach/stop/reset/archive/unarchive/delete/pin/rename/project placement; session list/lifecycle; event replay and human signal posting | M3 router/inbox deferral/reconciliation, human channel-message posting, feed/archive visibility parity, mission grid hint, resume helpers; all mission UI |
| Settings | Runtime status/refresh/override and MCP integration operations; archived mission/session operations | Native settings persistence/apply layer; M3 launch auto-resume seam; Phase 5 Sparkle, logs/reveal, URL opening, and window-state service |
| Multi-window | Shared `WindowRegistry` and focus-map event support in backend | Native window open/focus/report/titlebar adapter, subject reporting, route bootstrap, duplicate overlay, and position restore in Phase 5 |

## Recommended M4 sequence

Keep M4.1–M4.3 in the existing order, then swap the old M4.4 and M4.5 scopes:

1. **M4.1 App shell** establishes native chrome, the real theme/token system, routing, and the frame every comparison uses.
2. **M4.2 Shared widget set** prevents each missing page from inventing its own controls.
3. **M4.3 Direct chat and sidebar** brings the only existing native workflow to full parity and exercises the shell/widgets against live terminals.
4. **M4.4 Runners, crews, and project entry points** should precede missions. These surfaces exercise list, form, modal, drawer, drag, and navigation primitives at lower compositional complexity, and Crew Editor is the natural launch point for Start Mission.
5. **M4.5 Mission surfaces** then builds the highest-complexity workspace on mature components and the remaining M3 router/feed backend.
6. **M4.6 Settings, command palette, and multi-window** closes the global surfaces and parity sweep.

This swap is about UI dependency order, not backend availability: all remaining M3 slices complete before M4 starts, including pagination for the entity lists and router/feed work for missions. The entity pages give the shared list/form/modal/drawer/drag components a lower-complexity proving ground, and Crew Editor gives the mission stage a stable launch entry point before the most compositional workspace is rebuilt.

Some visible surfaces do not fit cleanly inside the six UI-only stages because their service contracts are intentionally Phase 5: Updates and the update prompt need Sparkle; Diagnostics needs native file logging and reveal; About links and terminal hyperlinks need an opener; window restore and duplicate-window actions need the native window adapter. Pull those specific Phase 5 prerequisites forward before the corresponding M4.6 tasks. Do not ship mock buttons or mark the surface parity-complete without its behavior.

The app icon's in-app treatment fits M4.1, but Dock/bundle embedding fits nightly packaging. Pencil-only Settings Chat and Remember window position are not tasks at all. There is no project page and no folder surface to force into M4.4.

## Exhaustiveness audit of `main/src`

Every non-test source file under `main`'s `src/` was assigned above. Tests were read as behavior evidence for their owning surfaces and are not separate product rows.

| Source group not otherwise obvious from filenames | Owning inventory rows |
|---|---|
| `src/hooks/useCurrentWindowFullscreen.ts`, `useResizableWidth.ts` | A2, A8, E1 |
| `src/hooks/useListControls.ts`, `src/lib/listControls.ts` | D1, D2, D6 |
| `src/hooks/useUpdateChecker.ts`, `src/contexts/UpdateContext.tsx` | A12, E8 |
| `src/lib/appZoom.ts`, `settings.ts`, `useStoredBool.ts` | A1, A3, E2–E6 |
| `src/lib/autoResume.ts`, `launchResumeTrace.ts`, `launchDims.ts` | A1, B10, C1, C2, C11, E2 |
| `src/lib/archivingState.ts`, `directChatStatus.ts`, `sessionLifecycle.ts`, `useDelayedFlag.ts` | B1, B2, B9, B11, C1, C2 |
| `src/lib/chatAttention.ts`, `groupPinning.ts`, `sidebarDnd.ts` | A6, A7 |
| `src/lib/chatTabs.ts`, `paneGeometry.ts`, `paneLayout.ts` | B4–B7 |
| `src/lib/deliveryBlocked.ts` | C9 |
| `src/lib/eventFeed.ts`, `missionComposer.ts`, `missionLastTerminal.ts`, `missionResume.ts`, `missionTabNavigation.ts` | C1–C6 |
| `src/lib/frontendLog.ts` | E9 and the Phase 5 logging service |
| `src/lib/keymap.ts` | A1, A8, E5 |
| `src/lib/projectScope.ts` | A6, B10, B11, C11, D10 |
| `src/lib/terminalBlank.ts`, `terminalPaste.ts`, `terminalRegistry.ts`, `terminalResize.ts`, `terminalSizing.ts`, `textureAtlas.ts`, `useTerminalBg.ts`, `windowSettle.ts` | B4, B8, C1, C2, E4 |
| `src/lib/windowFocus.ts` | A10, B1, C1 |
| `src/lib/api.ts`, `types.ts` | All rows; used to check the shipped operation and data contracts against `crates/runner-backend` |
| `src/assets/app-icon.png` | A4 |
| `src/vite-env.d.ts` | Build typing only; no product surface |
