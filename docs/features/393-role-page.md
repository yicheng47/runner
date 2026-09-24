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

Redesign the page in Pencil first, in `design/specs/393-role-page.pen`, which also holds the crew page ([699](./699-crew-page.md)), then implement to match. The page shows the same data and runs the same commands, with no backend changes.

- **Profile split**: a left column holds the role's pixel avatar, display name and handle, Chat now and Edit, then the setup (runtime, model and effort, permissions, command, working directory), the crews using the role and its activity. The system prompt fills the right column. The avatar is the mission's `RoleAvatar` seeded with the role's handle, so it needs no new field, and a slot that keeps its role's handle looks the same on the crew page and in a mission.
- **Edit in place**: Edit turns the page into its own form instead of opening the edit drawer. The left column's values become controls, the prompt card becomes the editor at full column height, and Save and Cancel replace Chat now and Edit. The create form keeps its drawer.
- **Clamped prompt**: the system prompt shows a few lines with expand and collapse instead of the full text, and is collapsed by default.
- **Chat now card removed**: it folds into the header action, and the working directory becomes a row in the setup.
- **Crews using this role** stays one glance away.
- **Override hint**: the stale hint is corrected.
- **Room for what is coming, without building it**: the layout leaves a place for per-run usage from #630 in Activity, and for per-role skills and MCP picks from #577, so neither lands as another card at the bottom.

## Non-goals

- New fields or capabilities.
- The role list page.
- The create and edit forms (`roles/create.rs`, `roles/edit.rs`), beyond what the new layout needs for visual consistency.
- Building the #630 or #577 sections.
- The crew page ([699](./699-crew-page.md)).

## Implementation phases

1. **Design**: a Pencil frame for the role page, reviewed before any code. Its layout language carries over to the crew page.
2. **Role page**: the setup hero, the clamped prompt, and the Chat now card folded into the header and Details.

## Verification

- `runner-app` tests pass, extended where the restructure moves behavior (`surfaces/roles/tests.rs`); workspace clippy is clean.
- Manual pass with a role whose system prompt runs to several hundred lines: the prompt is collapsed by default, and "Crews using this role" is visible without scrolling.
- On macOS and Windows, the page keeps its layout at the minimum window width.
