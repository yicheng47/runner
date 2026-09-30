# 768 — New role and New crew open their pages

> Tracking issue: [#768](https://github.com/yicheng47/runner/issues/768)
> Priority: P2, 0.12. Platforms: macOS and Windows.
> History: filed 2026-09-30 as "New role and New crew modals were left out of the role/crew redesign". Jason chose the page approach over a redesigned modal the same day, and signed off the design.
> Smoke-test changes: on 2026-10-01, Jason requested a generated avatar before a handle is entered and effort selection while Model is set to default. These updates supersede the initial empty role tile and default-model effort restriction; the supplied PNGs show the original design.

## Motivation

The role page (#393) and crew page (#699) moved to the new visual language, and so did the Start a chat modal (#735). The New role and New crew modals did not: New role is a stacked column of generic fields (`render_create_role_modal` in `surfaces/roles/create.rs`), and New crew is a bare Name field (`render_create_crew_modal` in `surfaces/crews/create.rs`). Creating a role or crew looks unlike viewing, editing or chatting with one.

A redesigned modal would be a third copy of the same fields, next to the role page and the crew page. Instead, New role and New crew open the page itself in a creating state: one page, two states, create a new one or edit an existing one.

## Design

`design/specs/768-new-role-crew.pen`: New role `jXUZc` and New crew `L5MbU`. Each is the page's in-place edit layout (`runner.pen` `eOJby` and `VFMMV`) with the changes below.

## Scope

**Entry points.** The Roles list's New role button and its empty state, the Crews list's New crew button and its empty state, and the Add slot modal's Create role link (`create_role_from_add_slot`, which today goes to Roles and opens the modal) open the creating page instead of a modal.

**Routing.** The creating pages are routes of their own (for example `AppRoute::NewRole` and `AppRoute::NewCrew`), so the sidebar highlights role or crew and back and forward work. The breadcrumb reads `Roles › New role` or `Crews › New crew` with a **NEW** tag in the editing tag's style; the first crumb returns to the list.

**New role** (`jXUZc`), the role page's edit layout (`render_role_edit_page`):

- **Avatar**: `RoleAvatar` at the page's size. Before a handle is entered, show a generated preview that stays stable for that draft. Seed the avatar with the handle as it is typed; clearing the handle restores the same draft preview. The saved role's avatar remains seeded by its handle.
- **Display name**: the edit page's large name field.
- **Handle**: an editable mono field with a faint `@` prefix under the name, and under it the hint "Lowercase letters, digits, - and _. The handle can't change later." A handle error (today's `handle_error`) replaces the hint in the danger colour.
- **Actions**: **Create role**, primary, full width, with a `plus` icon, then Cancel. Create is disabled until the form can submit (today's `create_role_can_submit`) and reads "Creating…" while submitting. The unsaved-changes note does not appear.
- **Setup**: Runtime, Model, Effort, Speed (Codex only, with "Fast uses more credits."), Command, Args and Working directory with Browse, the same controls and behaviour as the edit page. **Effort is new to creating a role**: today the modal always sends `effort: None`. Changing the runtime resets the command, model placeholder, effort options and speed as the create form does today.
- **Default model and effort**, on both the creating and edit pages: Effort can be selected while Model is set to default. Use the runtime's configured default model for capability filtering when known; otherwise offer the runtime's effort levels. An explicit model keeps its own supported-effort filtering. Saving an effort override with default Model sends the effort without a model override. Changing runtime resets the effort selection.
- **Prompt**: the system prompt editor card (Markdown and Preview, the line and size meta updating as it is typed), empty with a placeholder, and the edit page's caption under it.
- **Hidden** while creating: Crews using this role, and the activity and id lines.
- **Create** saves (`CreateRoleInput`, now with `effort`) and lands on the new role's page in view mode, as today. **Cancel** returns to where New role was opened. A backend error shows as today's error banner, at the top of the profile column.

**New crew** (`L5MbU`), the crew page's edit layout (`render_crew_editor`):

- **Picture**: an empty tile with a `users` glyph.
- **Name**: the edit page's large name field; the summary under it reads "No slots yet".
- **Actions**: **Create crew**, primary, full width, with a `plus` icon, then Cancel. Create is disabled while the name is blank and reads "Creating…" while submitting.
- **Slots**: the header reads "Slots · 0" with Add slot shown disabled, over "Add roles as slots once the crew exists." and "The first slot leads."
- **Conventions**: the Team conventions editor card, empty with a placeholder, and its caption. `CreateCrewInput.system_prompt_addendum` already takes it.
- **Hidden** while creating: the Missions card and the created and updated lines.
- **Create** saves and lands on the crew's page in view mode, where Add slot works. **Cancel** returns to where New crew was opened.

**Removed**: both create modals, their overlay hooks, and any form state the pages do not reuse.

**Behaviour kept**: today's validation, disabled and error states, IME-safe Enter handling, and a keyboard order that walks the page top to bottom. Leaving a creating page with typed content behaves as leaving the edit page does. The pages hold at the 640 × 480 minimum window as the edit pages do.

## Non-goals

- Adding slots before the crew exists: slots save one at a time against a crew, so the page would have to hold unsaved slots in memory; creating first costs one click, since only the name is required.
- Backend changes beyond sending `effort`, which `CreateRoleInput` already accepts.
- The Start a chat modal, the role and crew list pages, and `runner.pen` (applied after this ships).

## Verification

- `runner-app` tests pass, extended for the creating states, the routes, and the removal of the modals; workspace clippy is clean.
- Manual pass: New role from the list, from the empty state and from Add slot's Create role; the initial avatar preview, live handle changes and clearing back to the preview; a handle error; a Codex role with Speed and an effort, including default Model in create and edit; Create landing on the role page; Cancel. New crew with conventions, Create landing on the crew page and adding a slot there; Cancel. Back and forward through both; the 640 × 480 window; both themes.
- macOS and Windows.
