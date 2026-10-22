# Role and personal avatar customization

Tracking issue: [#996](https://github.com/yicheng47/runner/issues/996).

## Motivation

Make roles easier to distinguish with a readable initial-based default, retain the existing pixel-art option, and let people control the image/color representing a role or their own Human messages.

## Scope

- Save Initials / Pixel art, optional theme color and optional image on the existing role model and form flow.
- Offer Automatic, eight named theme colors and a validated custom hex color.
- Use the installed Codex CLI for AI generation. A role-derived default image brief is editable in a modal; only explicit Generate starts the request. A generated image remains a preview until role save. Preserve prior data on cancel, generation failure or stale results.
- Resolve role appearance by canonical role identity across lists, chats, crew aliases and mission views.
- Add Interface / My avatar local tabs under Settings → Appearance, not General. Personal styles, colors and copied PNG imports use existing application settings and Human event identity.
- Validate and own imported/generated files; clear, replacement, failed save and late async results must not delete user source files or images still referenced by roles.

The issue contains a text layout sketch. Native screenshots and design sign-off remain open review gates, not completed acceptance. Three requested generated default-image assets are not included; real image generation has not delivered usable files locally. No image API fallback, global provider/auth changes, release, production operation or event-identity change is in scope.

## Implementation phases

1. Extend the shared role protocol, SQLite migrations and role repository with default-compatible appearance fields.
2. Extend the existing avatar component and role forms with local-style/color selection, prompt modal and owned image preview generation.
3. Connect the same role appearance to all role-backed surfaces and add personal appearance to the existing settings/store path.
4. Cover serialization, migrations, process supervision, image validation/ownership, save/cancel/staleness and GPUI interactions; publish a draft PR with remaining live evidence explicit.

## Verification

The selected feature checks and exact automated evidence are in [the validation record](../tests/2026-10-22-role-avatars.md). Native macOS/Windows visual checks, successful real image generation and GitHub CI results must not be inferred from local tests. The development app was explicitly restarted for the user; startup/process/hash checks do not establish a native visual pass.

Affected user-guide pages: [Roles](../guide/concepts/roles.md) and [Appearance](../guide/guides/appearance.md). Implementation detail: [role-avatar plan](../impls/2026-10-22-role-avatars.md).
