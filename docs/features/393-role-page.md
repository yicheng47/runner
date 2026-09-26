# 393 — Role page redesign

> Tracking issue: [#393](https://github.com/yicheng47/runner/issues/393)
> Priority: P1, 0.12 release blocker. Platforms: macOS and Windows.
> History: filed as spec 58 for both detail pages in the Tauri era and renumbered to 393 on 2026-09-01. On 2026-09-22 it was split: this spec covers the role page, which goes first, and the crew page follows as [699](./699-crew-page.md).

## Motivation

The role page is still the MVP draft (frame `ocAFJ` in `design/runner-mvp-design.pen`). It was carried over as-is through the GPUI cutover and the #604 rename. `render_role_detail_body` (`crates/runner-app/src/surfaces/roles/detail.rs`) buries the page's primary content under prompt prose:

- **Prompt first**: "Default system prompt" is the first card in the main column and renders in full, so with any real prompt "Crews using this role", Activity and Details sit far below the fold.
- **Duplicate Chat now card**: a "Chat now" card repeats the header's Chat now button, mainly to show the working directory.
- **Stale hint**: the prompt card's hint still reads "Override per crew/mission slot later (v0.x)", though slot overrides for runtime, model and effort have shipped.

The positioning settled on 2026-09-16 raises the bar. A role is a setup that jobs run on: a runtime, a model, an effort level and a brief. Each run will show what it cost (#630). The page should lead with that setup, not with the prose.

## Scope

Redesign the role page and the role list in Pencil first, in `design/specs/393-role-page.pen`, which also holds the crew page ([699](./699-crew-page.md)), then implement to match. Both pages show the same data and run the same commands, with no backend changes. The list joined the scope on 2026-09-26.

- **Profile split**: a left column holds the role's pixel avatar, display name and handle, Chat now and Edit, then the setup (runtime, model and effort, permissions, command, working directory), the crews using the role and its activity. The system prompt fills the right column. The avatar is the mission's `RoleAvatar` seeded with the role's handle, so it needs no new field, and a slot that keeps its role's handle looks the same on the crew page and in a mission.
- **Edit in place**: Edit turns the page into its own form instead of opening the edit drawer. The left column's values become controls, the prompt card becomes the editor at full column height, and Save and Cancel replace Chat now and Edit. The editor shows the prompt as markdown text in the mono font, with a Markdown | Preview switch in its header that opens on Markdown. In edit mode the card has a neutral border, not the accent colour. The create form keeps its drawer.
- **Markdown prompt, clamped**: the system prompt renders as markdown (headings, lists, code blocks) through the renderer the mission feed already uses (`mission_markdown::render_markdown`). It shows a few lines with expand and collapse instead of the full text, and is collapsed by default.
- **Chat now card removed**: it folds into the header action, and the working directory becomes a row in the setup.
- **Crews using this role** stays one glance away.
- **Override hint**: the stale hint is corrected.
- **Room for what is coming, without building it**: the layout leaves a place for per-run usage from #630 in Activity, and for per-role skills and MCP picks from #577, so neither lands as another card at the bottom.
- **Role list as a table** (frame `Mq7E4`): the stacked cards become a table with a row per role, set directly on the page with no panel around it: a rule under the header and hairlines between rows. Its columns are Role (the `RoleAvatar`, display name and handle), Runtime (the provider mark and name), Model, Effort, Crews and Last active. An unset model or effort reads `default`, and a role in no crew shows a dash. A live role shows its session and mission count in the accent colour under Last active instead of the date of its last start. Clicking a row opens the role; hovering a row turns its Chat icon into a Chat button, and ⋯ keeps Edit details and Delete role. The title, New role and search stay, and the subtitle becomes "The setups your chats and crews run on: a runtime, a model, an effort and a brief." The paging keeps `PaginatedListPage`'s layout: eight rows a page, and the rows scroll above a pager pinned to the bottom of the page, centred under a rule. Two changes: the count moves from beside search to the left end of the pager row, and it reads as a total (`9 roles`), switching to matches of total (`3 of 9 roles`) only while a search is active; today it always reads `9 of 9 roles`. The component is shared, so the crew list's count changes with it. Everything shown comes from `role_list_with_activity` already.

## Non-goals

- New fields or capabilities.
- The create and edit forms (`roles/create.rs`, `roles/edit.rs`), beyond what the new layout needs for visual consistency.
- Building the #630 or #577 sections.
- The crew page ([699](./699-crew-page.md)).

## Implementation phases

1. **Design**: Pencil frames for the role page and the role list, reviewed before any code. The page's layout language carries over to the crew page.
2. **Role page**: the setup hero, the clamped prompt, and the Chat now card folded into the header and Details.
3. **Role list**: the table and its hover Chat, inside the existing paginated list page.

## Verification

- `runner-app` tests pass, extended where the restructure moves behavior (`surfaces/roles/tests.rs`); workspace clippy is clean.
- Manual pass with a role whose system prompt runs to several hundred lines: the prompt is collapsed by default, and "Crews using this role" is visible without scrolling.
- Manual pass with more than eight roles, some live, some with no model or effort, and one in no crew: the table pages at eight, live rows show their counts, and search narrows the rows and the count.
- On macOS and Windows, both pages keep their layout at the minimum window width.
