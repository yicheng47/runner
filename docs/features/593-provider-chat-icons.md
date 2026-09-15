# 593 — Provider icons for chats

> Tracking issue: [#593](https://github.com/yicheng47/runner/issues/593)
> Priority: P2 — makes mixed Claude/Codex work easier to scan, without blocking a workflow.
> Platforms: macOS and Windows.

## Motivation

Claude Code and Codex chats currently share a generic speech-bubble icon. When several chats are open, their provider should be recognizable before reading a title. Use Claude's starburst and the OpenAI knot for Codex, selected from the chat's stored runtime.

## Scope

### Provider identity

- `claude-code`: Claude starburst, in its orange brand color.
- `codex`: OpenAI knot, with a neutral foreground that remains legible in light and dark themes.
- `shell`: existing terminal icon.
- Other or unknown runtimes: existing generic chat icon until their own assets are added. Never display another provider's logo as a fallback.
- Empty panes: existing empty-pane icon.

Select the provider from `DirectSessionEntry.agent_runtime`; runner-backed chats use the session's runtime too. Renaming a chat, changing its model, restoring it, or changing its status must not change its provider identity. No terminal-title parsing, process detection, network logo lookup, database migration, or new preference is needed.

### Surfaces

| Surface | Behavior |
| --- | --- |
| Sidebar, single-pane tab | Replace the generic chat icon with the session's provider mark, including inline rename and drag presentation. |
| Workspace header, single-pane tab | Show the same provider mark. |
| Pane header inside a split tab | Show each pane's own provider mark, including pane-selection menus that already show identity icons. |
| Quick switcher, individual chat result | Show the chat's provider mark. |
| Settings → Archived, individual chat row | Show the archived chat's provider mark. |
| Sidebar and workspace header, split tab | Preserve the existing layout icon; a mixed-provider tab represents a group of panes. |

Generic actions and categories such as New chat, Chats, and Settings → Agents keep their existing symbols. This feature covers direct chat identity; runner/crew configuration and mission identity redesigns are outside its scope.

### Presentation

Bundle the two vector marks with the app and render them through the existing asset pipeline. Preserve the marks' original geometry and transparent backgrounds; do not substitute a generic sparkle, robot, or letter. Record source URLs and any required attribution alongside the assets.

Keep existing icon slots, spacing, and interaction targets, adjusting the artwork's optical size only as needed for legibility at 12–16 px. Claude stays orange and Codex uses a neutral light/dark foreground; active rows and focus remain apparent through the existing selection treatment. Provider icons do not take on Working/Needs you/Ready colors or replace status indicators, unread markers, or attention badges.

## Reference implementations

- **Orca:** its shared `AgentIcon` maps Claude to `ClaudeIcon` and Codex to `OpenAIIcon`. The Claude SVG uses `#D97757`; the OpenAI SVG uses `currentColor`. This is the reference for the actual marks and the small shared mapping. Unknown identities receive a neutral fallback. Sources inspected at `c1e15c4008627dbbfb489f5ec44f668f93c20b34`: [agent catalog](https://github.com/stablyai/orca/blob/c1e15c4008627dbbfb489f5ec44f668f93c20b34/src/renderer/src/lib/agent-catalog.tsx#L319), [SVG definitions](https://github.com/stablyai/orca/blob/c1e15c4008627dbbfb489f5ec44f668f93c20b34/src/renderer/src/components/status-bar/icons.tsx).
- **cmux:** `SessionIndexAgentIconImage` resolves a bundled branded asset with a system-symbol fallback. Its Codex asset catalog provides 1×/2×/3× images and explicit dark-appearance variants. Follow the bundled-assets and theme-legibility approach; Runner can use vector marks with explicit tint instead of adopting an AppKit asset catalog. Sources inspected at `6f118af63b68630f3cf603794c6cb63d15845032`: [icon renderer](https://github.com/manaflow-ai/cmux/blob/6f118af63b68630f3cf603794c6cb63d15845032/Sources/SessionIndexAgentIconImage.swift), [Codex appearance metadata](https://github.com/manaflow-ai/cmux/blob/6f118af63b68630f3cf603794c6cb63d15845032/Assets.xcassets/AgentIcons/Codex.imageset/Contents.json), [provider assets](https://github.com/manaflow-ai/cmux/tree/6f118af63b68630f3cf603794c6cb63d15845032/Assets.xcassets/AgentIcons).

## Implementation Phases

### 1. Design and assets

Create a small feature-scoped Pencil file at `design/chat-provider-icons.pen` before implementation. Show Claude and Codex single-pane tabs, a mixed-provider split, the switcher, and archived rows in light/dark themes with active, inactive, and attention states. Record the relevant frame/node references in this spec. Source and bundle the two SVG marks.

### 2. Replace chat identity icons

Use a small shared runtime-to-icon mapping in `runner-app`, including the appropriate tint, and reuse it at the existing identity call sites. Register the assets in `crates/runner-app/src/assets.rs`. GPUI's SVG element receives a paint tint, so preserving the Claude color requires an explicit tint rather than relying only on the SVG's embedded fill.

Update `sidebar/menus.rs::sidebar_tab_icon`, `sidebar/rows_render.rs` and its icon/rename/drag presentation, plus `panes.rs::pane_identity_icon` and `workspace_header_icon`. The single-pane workspace header needs the actual runtime instead of only a shell/non-shell boolean. Preserve layout-icon precedence for split tabs.

Carry the already-available runtime into the presentation items in `surfaces/command_palette.rs` and `surfaces/settings/archived.rs`; those currently discard it and choose an icon from the generic item kind. Keep generic action/category icons separate from individual chat identity.

### 3. Verify

Update the existing icon-selection assertions, then check the rendered surfaces against the Pencil design. No status detector or session lifecycle changes are part of this feature.

## Verification

- Claude Code and Codex chats show distinct provider marks across all scoped surfaces, including chats started from runner templates.
- Rename, resume, archive/restore, switching tabs, and dragging panes preserve the correct provider mark.
- A mixed Claude/Codex split shows each pane's provider and retains the tab's layout icon.
- Shell, empty-pane, unsupported-runtime, and generic-action icons remain correct.
- Working, Needs you, Ready, unread, and error indicators remain visible alongside provider identity.
- Check light/dark themes, selected/unselected rows, narrow panes, and display scaling on macOS and native Windows. Both marks must render offline from the packaged app.
- For the implementation, run `cargo test -p runner-app`, workspace Clippy, and formatting checks per `AGENTS.md`; update existing tests where expectations change.
