# 606 — One liveness rule for rail glyphs, and a folder that opens

> Tracking issue: [#606](https://github.com/yicheng47/runner/issues/606)
> Priority: P2. Platforms: macOS and Windows.
> Shipped 2026-09-16 in [#613](https://github.com/yicheng47/runner/pull/613) (codex peer mission, Jason's macOS smoke passed); the Windows look at the rail rides with the [610](../610-windows-hook-status.md) JASONPC pass.
> Design: `design/runner.pen`, frame `Spec — Rail glyphs (606) · v1` (`Xk8NM`), to the right of the 540 frame. Signed off 2026-09-16: Jason chose the lucide outline pair (`QHByO`).

## Motivation

The sidebar rail runs two rules at once. Folder, shell and mission-flag glyphs say "a session is running in here" with the accent green, while provider marks ([#593](593-provider-chat-icons.md)) say identity with a fixed hue and say "live" with opacity. In a mixed rail the green folders and flag read as a state or a brand, and they are neither. The Trae mark, one hue from the accent, collides with them. With Copilot purple arriving ([540](./540-copilot-cli-runtime.md)) the rail would carry four identity hues plus a fifth colour that means something else.

The folder glyph is a single `folder-code` outline for collapsed and expanded projects alike, so the chevron is the only thing that changes when a project opens, and the `<>` badge says "code" about every project, which is every project.

## Scope

### One rule for every rail glyph

- **Hue is identity, and only provider marks have one.** Claude orange, Trae green, Codex `$text-1`, Copilot purple, as [#593](593-provider-chat-icons.md) decided. Nothing else in a row head carries a colour.
- **Every other glyph is neutral and liveness is opacity**, the rule provider marks already follow. Folder, shell, layout and flag glyphs draw in `$text-1` at full strength when a member session is running and at 0.45 when none is, beside the label that already drops to faint. An inactive row dims as one unit; an active row is full strength.
- **The accent is reserved for state at the tail of a row**: the Working spinner, the ready dot, the unread pill, the attention badge, and the drag-and-drop indicators. The mission flag no longer duplicates the spinner beside it.
- Selection, hover and the semibold label are untouched.

### A folder that opens

- A matched closed/open pair selected by the project's expansion state, next to the chevron that already flips. Collapsed projects show the closed folder; expanded ones the open folder.
- Decided: lucide `folder` / `folder-open`, the pair the M6.21 sidebar frames already drew and the same 2 px stroke as every other rail glyph; `folder-open.svg` is already bundled. Not taken: phosphor `folder-fill` / `folder-open-fill`, stronger at 12 px but the only solid glyph in the rail. Both are on the canvas beside today's `folder-code`.
- The `<>` badge goes.

### What does not change

- Provider marks, their tints, their dimming.
- Tail-state glyphs and their colours.
- The rail's layout, row metrics, and the chevron.

## Implementation Phases

### Phase 1 — design

The frame shows the three folder pairs at 12 px in collapsed and expanded rows and at 16 px enlarged, the rail from Jason's 2026-09-16 screenshot before and after (six projects, a shell, three Claude chats and a running mission), the after state on the light ground, and the rules.

### Phase 2 — `runner-app`

- [x] `sidebar_icon(icon, live)` in `surfaces/sidebar`: a generic icon takes `theme::text()` with opacity 1.0 or 0.45 instead of the accent/muted pair; provider marks keep their existing branch. Callers in `rows_render.rs` (project header at the `folder-code.svg` sites, mission row at the `flag.svg` sites, shell and layout tabs through `sidebar_tab_icon`) pass the same `live` they pass today.
- [x] `assets.rs`: the folder pair; `folder-open.svg` exists, the closed one joins it; `folder-code.svg` is dropped once nothing references it.
- [x] The project header picks the asset from `collapsed`, next to the chevron that already does.
- [x] Tests: the icon-colour table in `chat_icon.rs`, `sidebar_tab_icon`, and a project-header test for the asset by expansion state.

### Phase 3 — smoke

macOS and Windows: a live and a stopped project, shell, mission and chat, in both themes; collapse and expand a project; drag a row and check the accent drop indicator is unchanged.

## Verification

- Every head-of-row glyph other than a provider mark is `$text-1` in both themes; none is accent-coloured.
- A project with a running member session is full strength and one without is 0.45, folder and label together.
- A collapsed project shows the closed folder and an expanded one the open folder; the chevron still flips.
- The spinner, ready dot, unread pill and drop indicator are still accent-coloured.
- `cargo test -p runner-app`, workspace Clippy and formatting per `AGENTS.md`.
