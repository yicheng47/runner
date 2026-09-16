# 593 — Provider icons for chats

> Tracking issue: [#593](https://github.com/yicheng47/runner/issues/593)
> Status: shipped 2026-09-15 in [#595](https://github.com/yicheng47/runner/pull/595).
> Priority: P2 — makes mixed Claude/Codex/Trae work easier to scan, without blocking a workflow.
> Platforms: macOS and Windows.
> Design: `design/runner.pen`, frame `Spec — Provider icons for chats (593) · v1` (`Up7D0`), in the spec row beside #587.
> Decision, 2026-09-15: identity is the hue, liveness is the opacity. All three agent runtimes get a mark; Trae keeps its brand green.

## Motivation

Every agent chat is drawn with the same speech bubble today, so a rail of mixed work reads as one undifferentiated stack until you read the titles. Give Claude Code, Codex and Trae their own marks, selected from the runtime already stored on the session.

## Scope

### Provider identity

- `claude-code`: the Claude starburst, in `#D97757`.
- `codex`: the OpenAI knot, in `$text-1` — the theme's own foreground, so it inverts with the ground instead of shipping a second asset.
- `trae`: the Trae screen glyph, in `#32F08C`.
- `shell`: the existing terminal icon, unchanged.
- Other or unknown runtimes: the existing generic chat icon. Never display another provider's logo as a fallback — a wrong logo is worse than a generic one.
- Empty panes: the existing empty-pane icon.

Select the provider from `DirectSessionEntry.agent_runtime`; runner-backed chats resolve through the same field, so a chat started from a runner template and a hand-started chat of the same runtime look identical. Renaming a chat, changing its model, archiving and restoring it, dragging its pane, or resuming it after a relaunch must not change the mark. No terminal-title parsing, process detection, network logo lookup, database migration, or new preference is needed.

### Identity, state, and liveness

The mark is identity and never becomes state. It holds one hue in every row it appears in — selected or not, focused or not, Working, Needs you or Ready. Selection stays with the row's fill, border and semibold label; pane focus stays with the header text and body opacity; status stays with [#347](./347-hook-based-session-status.md)'s glyph, the unread marker and the attention badge. No row may have two things changing colour at once.

Liveness is the exception, and it is carried by opacity rather than hue. `sidebar_icon(path, live)` paints the icon in `theme::accent()` when a member session is `Running` and `theme::muted()` when none is; the parameter is named `active`, but every call site passes `live` — chat tabs, the mission flag, and the project folder. That is the rail's "something is running in here" signal, and a fixed brand tint would spend it. So a provider mark keeps its hue and renders dimmed at `0.45` when no member session is running, beside the label that already drops to `theme::faint()`. This matches the app's existing dimming idiom (`UNFOCUSED_PANE_OPACITY` is `0.7`; disabled controls sit at `0.5`–`0.6`). Shell, layout, mission-flag and project-folder glyphs keep the accent/muted pair exactly as it is.

`SessionStatus` is `Running`, `Stopped` and `Crashed`; there is no separate paused state, so one dimmed state covers pause, exit and crash alike. Archived rows are stopped by definition and render dimmed.

### Surfaces

| Surface | Behavior |
| --- | --- |
| Sidebar, single-pane tab | Replace the generic chat icon with the session's provider mark, including inline rename and drag presentation. |
| Workspace header, single-pane tab | Show the same provider mark. |
| Pane header inside a split tab | Show each pane's own provider mark, including pane-selection menus that already show identity icons. |
| Quick switcher, individual chat result | Show the chat's provider mark. |
| Settings → Archived, individual chat row | Show the archived chat's provider mark, dimmed. |
| Sidebar and workspace header, split tab | Preserve the existing layout icon; a mixed-provider tab represents a group of panes and has no single mark. |

Generic actions and categories such as New chat, Chats, and Settings → Agents keep their existing symbols. This feature covers direct chat identity; runner/crew configuration and mission identity redesigns are outside its scope.

### Presentation

Bundle the three vector marks with the app and render them through the existing asset pipeline. Preserve each mark's original geometry and transparent background; do not substitute a generic sparkle, robot, or letter. Record source URLs and any required attribution alongside the assets.

Keep existing icon slots, spacing, and interaction targets, adjusting the artwork's optical size only as needed for legibility at 12–16 px.

Trae's `#32F08C` sits one hue from Carbon's `#00FF9C` accent, which is also the Ready colour. This was weighed and accepted: the Trae mark is the weakest of the three in monochrome — a rectangle with two dots — so the colour is most of what makes it recognizable, and the mark sits at the head of the row while the status dot sits at its tail, so the two never share an edge.

## Reference implementations

- **Orca:** its shared `AgentIcon` maps Claude to `ClaudeIcon` and Codex to `OpenAIIcon`. The Claude SVG uses `#D97757`; the OpenAI SVG uses `currentColor`. This is the reference for the actual marks and the small shared mapping. Unknown identities receive a neutral fallback. Sources inspected at `c1e15c4008627dbbfb489f5ec44f668f93c20b34`: [agent catalog](https://github.com/stablyai/orca/blob/c1e15c4008627dbbfb489f5ec44f668f93c20b34/src/renderer/src/lib/agent-catalog.tsx#L319), [SVG definitions](https://github.com/stablyai/orca/blob/c1e15c4008627dbbfb489f5ec44f668f93c20b34/src/renderer/src/components/status-bar/icons.tsx).
- **cmux:** `SessionIndexAgentIconImage` resolves a bundled branded asset with a system-symbol fallback. Its Codex asset catalog provides 1×/2×/3× images and explicit dark-appearance variants. Follow the bundled-assets and theme-legibility approach; Runner needs neither an asset catalogue nor a second file per theme, because the mask is tinted at paint time. Sources inspected at `6f118af63b68630f3cf603794c6cb63d15845032`: [icon renderer](https://github.com/manaflow-ai/cmux/blob/6f118af63b68630f3cf603794c6cb63d15845032/Sources/SessionIndexAgentIconImage.swift), [Codex appearance metadata](https://github.com/manaflow-ai/cmux/blob/6f118af63b68630f3cf603794c6cb63d15845032/Assets.xcassets/AgentIcons/Codex.imageset/Contents.json), [provider assets](https://github.com/manaflow-ai/cmux/tree/6f118af63b68630f3cf603794c6cb63d15845032/Assets.xcassets/AgentIcons).
- **Trae mark:** [lobe-icons](https://github.com/lobehub/lobe-icons) `packages/static-svg/icons/trae-color.svg` — a single path in `#32F08C`. Their `Mono` variant is a different, simplified glyph and is not used.

## Implementation Phases

### 1. Design and assets — done

The design is `design/runner.pen`, frame `Spec — Provider icons for chats (593) · v1` (`Up7D0`): the three marks at 12/16/24 px (A), sidebar before/after (B), split-pane headers with the tab's layout icon preserved (C), quick switcher and archived rows (D), the Trae tint decision against Runner's accent (E), the light theme (F), and running versus stopped rows for all three providers (G). The marks are also components: `cmp/MarkClaude`, `cmp/MarkCodex`, `cmp/MarkTrae`.

### 2. Replace chat identity icons

GPUI rasterises an SVG as a mask and paints it in one colour — `window.paint_svg(bounds, path, None, transformation, color)` takes a single tint from `style.text.color` — so a fill baked into the file is discarded. All three marks are single-path, so nothing is lost, but Claude's orange and Trae's green must be written at the call site.

Register the three marks as byte constants in `crates/runner-app/src/assets.rs` with one `ASSETS` row each, beside the lucide glyphs. Nothing is fetched at runtime: the packaged app must draw all three offline on macOS and Windows.

Add a small shared runtime-to-mark mapping in `runner-app` — asset path plus tint — and reuse it at every identity call site: `sidebar/menus.rs::sidebar_tab_icon`, `sidebar/rows_render.rs` with its icon, inline-rename and drag presentation, and `panes.rs::pane_identity_icon`. `workspace_header_icon` takes a `grouped`/`focused_shell` bool pair today and needs the focused pane's runtime instead. Preserve layout-icon precedence for split tabs.

Split tint from liveness in `sidebar_icon`: a provider mark takes a fixed tint plus opacity, while the shell, layout, mission-flag and project-folder glyphs keep today's accent/muted pair.

Carry the already-available runtime into the presentation items in `surfaces/command_palette.rs` (`PaletteKind::Chat` currently supplies the icon) and `surfaces/settings/archived.rs` (`ArchivedKind::Chat` likewise); both discard the runtime and choose an icon from the generic item kind. Keep generic action/category icons separate from individual chat identity.

### 3. Verify

Update the existing icon-selection assertions — `pane_identity_icon` in `panes.rs`, `sidebar_tab_icon` in `sidebar/tests.rs`, both of which expect `message-square.svg` for `claude-code` and `codex` — and add a registration test in `assets.rs` beside the ones already there. Then check the rendered surfaces against the design frame. No status detector or session lifecycle changes are part of this feature.

## Verification

- Claude Code, Codex and Trae chats show distinct provider marks across all scoped surfaces, including chats started from runner templates.
- Rename, resume, archive/restore, switching tabs, and dragging panes preserve the correct provider mark.
- A stopped or crashed chat dims its mark without changing its hue; a running one is at full strength. The mission flag and project folder keep their accent/muted behavior.
- A mixed-provider split shows each pane's provider and retains the tab's layout icon.
- Shell, empty-pane, unsupported-runtime, and generic-action icons remain correct.
- Working, Needs you, Ready, unread, and error indicators remain visible alongside provider identity.
- Check light/dark themes, selected/unselected rows, narrow panes, and display scaling on macOS and native Windows. All three marks must render offline from the packaged app.
- For the implementation, run `cargo test -p runner-app`, workspace Clippy, and formatting checks per `AGENTS.md`; update existing tests where expectations change.
