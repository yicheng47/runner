# Role initials and optional Codex-generated avatars

> **For agentic workers:** Use subagent-driven development for the isolated backend and UI slices. Preserve existing work and do not launch the app as a coder or reviewer.

**Goal:** Let each role choose saved initials or pixel art, with optional Codex CLI-generated image avatars in the existing role forms.

**Approved design:** Confirmed in chat on 2026-10-22. A colored rounded tile displays either the uppercase first character of the role handle (`?` while blank) or the original mirrored pixel pattern. Initials is the default. The new/edit form offers both base styles plus AI generation and regeneration. Generation uses role identity and responsibilities, runs without blocking the UI, and never overwrites the saved avatar before the role is saved. Cancel and failure preserve saved data. All role-backed surfaces use the saved image, including renamed crew slots. Existing presence indicators remain unchanged.

**Architecture:** Extend the existing Role, create/update protocol, SQLite migration/repository, and daemon API; do not add a second role-save protocol or a direct image API. The daemon resolves the configured Codex CLI, runs a bounded isolated `codex exec` request with image generation enabled, validates and stages the image in Runner data, and returns an immutable image path. Existing role save owns adoption of that path. Temporary generation outputs are discarded when stale/cancelled. The shared RoleAvatar widget supports an optional image path and initials fallback.

**Tech stack:** Rust, GPUI, existing daemon protocol, rusqlite, Codex CLI.

## Contract and scope

- `Role.avatar_path: Option<String>`; missing old serialized fields default to None. `Role.avatar_style: AvatarStyle` defaults to `Initials` and is omitted on the wire when default; `Pixel` is serialized as `pixel`.
- `CreateRoleInput.avatar_path: Option<String>` defaults to None; `avatar_style: AvatarStyle` defaults to initials.
- `UpdateRoleInput.avatar_path: Option<Option<String>>` uses the existing omitted/preserve, null/clear, value/replace semantics. `avatar_style: Option<AvatarStyle>` preserves the stored style when omitted.
- `GenerateRoleAvatarInput { handle: String, display_name: String, system_prompt: Option<String> }`.
- `GeneratedRoleAvatar { path: String }`.
- `role_avatar_generate(input: GenerateRoleAvatarInput) -> GeneratedRoleAvatar` and `role_avatar_discard(path: String) -> ()` use the existing daemon API, with a request-specific generation timeout.
- Images stay in Runner-managed storage; reject arbitrary outside paths. A discard must not remove an image referenced by a role. Output is bounded and validated; failed generation, replacement, and rollback preserve the old image.
- No changes to agent global configuration, unrelated chats/missions, installed Runner data, releases, or remote Git state.
- Initial implementation had no GitHub CLI login. For the authorized publication follow-up, the configured Git credential is reused without printing or saving its secret; tracking issue #996 records the scope. Publish via the current user's fork and leave the PR open, not merged.

## Task 1: Persistence and generation (backend)

Files: runner-core protocol role/model/api/socket modules, runner-daemon role ops/repository/migrations, a focused avatar generation module, affected backend/CLI fixtures.

- [x] Add tests for optional avatar serialization, migration/read/write, patch clear/preserve semantics, managed-path validation and safe discard.
- [x] Observe relevant tests failing; implement minimal persistence changes in existing create/update transactions.
- [x] Add a fake-Codex process fixture to cover successful output, invalid/missing output, failure and timeout without a paid network request.
- [x] Resolve the Codex binary via existing settings and runtime-path handling. Use command arguments, a private temporary working directory, bounded process lifetime/output, and do not invoke a shell or load the project directory.
- [x] Validate and copy successful image output atomically into immutable managed storage; clean temporary resources.
- [x] Run relevant runner-core/runner-daemon/runner-cli tests.

## Task 2: Initials, form workflow, and consumers (UI)

Files: shared ui/avatar widget; role create/edit/form state, save and detail rendering; role-backed consumers in chat/crew/mission surfaces; i18n English/Chinese.

- [x] Add failing initials and fallback tests, then replace the pixel grid in the existing widget.
- [x] Add optional image rendering with initials fallback and preserve presence sizing/position.
- [x] Add form-local generation state, one request per form, generation/regeneration and restore controls, explicit error/loading text, dirty-state integration.
- [x] Capture form identity across async completion; discard stale results and unsaved images on cancellation; never write a generated path before save.
- [x] Pass avatar_path through existing role_create/role_update and all role-backed consumers by role ID (or immutable canonical role handle in crew summaries), never by slot alias.
- [x] Update relevant UI fixture Role/CreateRoleInput literals and role form tests.

## Task 3: Docs, review and verification

- [x] Update docs/guide/concepts/roles.md for default initials, optional Codex prerequisite and persistence/cancel/failure behavior.
- [x] Run runner-app tests, backend focused tests, workspace clippy, and formatting checks. Data/protocol changes justify core/daemon tests beyond normal UI checks.
- [x] Review final diff and perform independent review of cancellation, storage ownership, image loading and process boundaries.
- [x] Record actual test outputs and remaining gaps. Native live QA is separate and must not be claimed from unit tests; coders and reviewers do not launch the app.
- [x] No commit/push/PR unless explicitly requested.

## Selected behavior checks and rationale

- Initials: lowercase/whitespace/non-ASCII/blank handles and small/large sizes replace seed-dependent pixels.
- Form generation: loading/duplicate prevention, success preview, failure retains prior avatar, restore, dirty/save/cancel, late completion after leaving.
- Persistence: migration of an existing role, omitted versus cleared versus replaced avatar, restart/readback, unsafe paths and invalid images.
- Consumers: role list/detail, new-chat picker, chat panel, crew slots/composite, start-mission and mission feed/rail use the same saved avatar even when slot handles differ.
- CLI process: fake executable validates argument boundaries, output/error/timeout, no project/global-config writes, no orphaned temporary files.
- Live regression selection: role create/edit and role-backed chat/crew identity rows only; IME, authentication and unrelated session lifecycle are outside scope.

## Final evidence and limits

See `docs/tests/2026-10-22-role-avatars.md`. Core 66, app 807 (3 existing ignored), avatar/migration 18, existing role ops 38 and shell discovery 13 tests passed. Schema snapshot and preceding migration tests passed. Workspace Clippy and formatting passed. The optional Role image field is omitted when absent to preserve existing CLI output; ten affected golden cases pass after that compatibility fix. One unrelated runtime-start golden error-precedence mismatch remains, with both outputs recorded. Normal Metal build, native UI/Windows acceptance and real successful Codex image generation are not claimed. The real current Codex session reports native image generation unavailable. No commit, push, PR, release or global agent-configuration change was performed.

## Follow-up: selectable pixel and initials styles

User requested both generated pixel art and initials as saved choices. Keep initials as the default from the approved design. Extend the existing Role/create/update/database shape with `avatar_style: AvatarStyle` (`initials` or `pixel`); default/legacy initials omit the extra wire field, and an omitted update preserves the saved style. Add migration 0030 so a previously-created migration-0029 database also upgrades safely. Restore the original mirrored pixel algorithm inside RoleAvatar and select it through `.style(AvatarStyle)`; saved AI images still take precedence and fall back to the chosen style when missing/unreadable.

The form always offers Initials and Pixel buttons. Selecting a base style clears the draft image using the existing preview cleanup and save/cancel lifecycle; applying an AI result does not discard the saved fallback style. Persist style and path together through the existing create/update call. Propagate both through all role-backed avatars, including renamed crew aliases and mission role mappings. No new image-generation requests, provider changes, agent settings, commit, push or PR are authorized by this follow-up.

Validation: unit checks for the original pixel symmetry/determinism and both missing-image fallbacks; GPUI checks for selecting, previewing, saving/reopening and cancelling a style change plus AI-to-base-style transitions; serialization/default/patch semantics and migration 0029-to-0030; existing role operations and app suite, all-target workspace Clippy and formatting. Do not repeat unrelated CLI lifecycle/golden cases or the real paid image probe.


### Follow-up verification completed

The updated app suite passed 815 tests (3 existing ignored). Core passed 66; avatar process/storage tests passed 17; role operations passed 38; role repository tests passed 6; the 0029-to-0030 migration and schema snapshot each passed. All-target workspace Clippy with updater/runtime shaders, formatting and diff whitespace checks passed. Both direct widget fallback rendering and actual form mouse/keyboard save/cancel flows are covered. Existing native-image capability, Metal compiler, native-window and Windows verification limits remain as recorded. No new real image-generation request was made.

## Follow-up: role theme colors and editable AI prompt dialog

The supplied reference is a visual palette, not execution instructions. Apply it to each role's avatar/identity color, not the global application theme. Add the eight named presets (orange, blue, emerald, purple, rose, cyan, amber, slate), a custom `#RRGGBB` field and an Automatic option preserving existing seed-derived colors. Use the existing form draft/create/update flow, with migration 0031 for an optional canonical lowercase color. Invalid hex never reaches persistence; existing roles without a color keep their current appearance. Both local styles, image fallbacks, crew aliases, current mission identities and role color indicators use the saved role color. AI bitmaps are not recolored retroactively.

Clicking Generate/Regenerate opens the existing Modal primitive with an editable multiline image brief, prefilled from role name, handle, responsibilities and selected color. Offer Reset to default, Cancel and Generate. Opening/cancelling/resetting never launches Codex or consumes generation usage. Confirm sends the edited brief through the existing generation request (a new optional prompt field), while preserving the native-image-only execution and output/path validation boundaries. Keep an edited brief for regeneration in the current form only; it is not a second persisted role prompt or a global agent setting. The role's system prompt remains separate and unchanged. Default prompts are localized; callers omitting the new field keep a backend default for compatibility. Empty/oversized prompts are rejected before process launch. Late-result source matching also includes the image brief and selected color.

Selected checks: hex normalization/validation, default/omitted/clear/replace color persistence and migration 0030-to-0031; both local renders/fallbacks with explicit color; preset/custom/invalid/reset/cancel/save UI and current-role alias propagation; modal default/edit/reset/cancel/confirm/blank/key handling without real generation, exact edited prompt captured by a fake transport, unchanged system prompt; bounded fake-Codex tests remain in scope. Full app tests, focused core/daemon tests, all-target workspace Clippy and formatting are the automated gates. No real image request, provider/login/global-config change, unrelated CLI regression sweep, commit, push or release is part of this refinement.


### Color / prompt refinement evidence

The updated app suite passed 823 tests (3 existing ignored); core passed 67; daemon avatar/brief/storage tests passed 19; role operations 38; repository 6; the migration-to-0031 and schema snapshot each passed. All-target workspace Clippy passed. The role palette and editable modal are implemented on the same worktree, with no real image request or global configuration change. The running user-acceptance dev window is intentionally left alone until a requested refresh, so no unsaved form is discarded.


### Color / editable prompt refinement final verification

Final checks passed: app 823 (3 existing ignored), core 67, avatar backend 19, role operations 38, role repository 6, migration-to-0031 1, schema 1, all-target workspace Clippy, formatting and diff whitespace. A development candidate was built successfully with the already-used runtime-shader feature. It has not replaced the running user's dev window; restart only after their current form is saved or discarded deliberately. Native/Windows acceptance and actual successful image generation remain separate, unclaimed gates.

## Timeout follow-up

User reported a Codex CLI call timeout. Read-only runtime identity checks showed the acceptance app/daemon still running the 13:21 candidate, not the newer palette/prompt build. There was no per-generation diagnostic record for reconstructing the user's completed failed call, so its exact upstream cause is not claimed.

No-inference probes confirmed fast CLI/MCP initialization and exposed a capability mismatch: `modelProvider/capabilities/read` returned imageGeneration=true, while a locally redirected custom-provider request contained neither the native image tool nor an image tool source in deferred discovery. This is not proof that all API-key providers are unsupported, and no provider/login/model/global configuration was changed. No real model turn or image generation was requested in these probes.

Fix the confirmed Runner-owned bug in the existing subprocess path: stdout JSON was previously buffered until process exit, so a terminal turn failure could be hidden by a later generic timeout. Parse only bounded stdout event lines, record fixed stage/error categories without raw messages or secrets, stop/reap on terminal `turn.failed`, retain nonterminal retry context, and include the last stage/category in a genuine deadline error. Keep the existing 180-second lifetime, 210-second IPC deadline, output/image validation and no-retry ownership. Warn-log the safe failure so the next user-controlled attempt is diagnosable.

Selected checks: a fake CLI that emits authentication failure but keeps running must return before its deadline; a fake reconnecting CLI must retain provider/network context on timeout; oversized progress events remain bounded and do not leak secrets; all earlier image/process/storage cases remain passing. Avatar UI checks, workspace Clippy, formatting and a development candidate build cover integration. Native successful image generation is still not claimed.

## Personal avatar in Appearance (2026-10-22)

The user refined the personal-avatar location to **Settings → Appearance**, with a separate subsection/tab rather than a General setting. Extend the existing Appearance renderer and `AppSettings`/store update path; do not introduce a new top-level settings route, human AI-role record, event identity, or parallel profile persistence protocol. Preserve all current theme controls.

Implementation steps:
1. Add default-compatible personal avatar settings (initials/pixel style, optional validated theme color, optional owned image path); test old JSON defaults and round trips before implementation.
2. Add Appearance-local navigation between interface settings and My avatar. Reuse the existing avatar renderer, palette and native image selection patterns. Copy chosen images into personal app-data storage; never delete external source images or leave personal image ownership dependent on the daemon's role-reference cleanup.
3. Render human-authored mission feed entries from personal settings without affecting role/system authorship or role appearance resolution.
4. Update the Appearance guide. Verify focused personal-avatar, settings, human/role avatar and localization tests, then runner-app tests, workspace Clippy and formatting. Native UI acceptance remains separate.

Three generated cyberpunk/anime/superhero-comic presets remain pending: no callable image generator has produced files. Do not add fabricated placeholders, change the user's provider/login/global configuration, or silently fall back to a separately billed Images API. A capability flag and an installed skill do not prove tool availability in a concrete model turn.

Personal-avatar completion: implemented Interface / My avatar local Appearance tabs, saved local styles/colors and PNG import with a copied personal file. Settings candidates persist atomically before notifying the shared store. Superseded owned copies are reclaimed only after saving; stale cross-window imports are invalidated with a shared transient edit epoch. The app's existing human author identity and role/system avatar routing remain unchanged. Final app suite: 834 passed / 3 existing ignored, workspace Clippy and formatting passed. Actual native image calls returned empty results and no asset paths; three requested generated defaults remain pending, rather than being represented by fake assets.
