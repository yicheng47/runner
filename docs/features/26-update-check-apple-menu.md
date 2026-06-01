# 26 - Update check in the Apple menu

> Tracking issue: [#199](https://github.com/yicheng47/runner/issues/199)

## Motivation

Runner currently exposes update controls as a dedicated Settings pane. That makes Settings carry a transient app action instead of only durable preferences, and it diverges from Quill's desktop pattern where manual update checks live in the native app menu next to About.

The desired shape is simpler: Settings should not have an Updates section. Manual update checks should be discoverable through the macOS Apple/app menu, while the existing update toast remains the place where checking, available, downloading, ready, and error states appear.

## Scope

### In scope

- Remove the Updates item from the Settings sidebar and remove the Settings Updates pane UI.
- Add a native "Check for Updates..." menu item.
- On macOS, place "Check for Updates..." in the app menu near About, following Quill's placement pattern.
- On non-macOS, place the same action under Help if a menu is present.
- When the menu item is selected, surface/focus the main window and emit a frontend event such as `menu:check-for-updates`.
- Update the frontend update provider to listen for that event and call the existing manual check path, so the toast shows active feedback for checking, up-to-date, available, downloading, ready, and error states.
- Preserve the launch auto-check and auto-install behavior that already exists in `UpdateContext`; existing stored preferences should continue to be honored even though the Settings pane no longer exposes the toggle.

### Out of scope

- Redesigning the update toast.
- Adding a replacement update preferences pane.
- Changing release, signing, notarization, or updater endpoint behavior.
- Adding localized native menu labels. Runner's current native menu is English at boot; localization can follow separately if needed.

## Implementation Phases

### Phase 1 - Native menu

- Add a "Check for Updates..." menu item in `src-tauri/src/lib.rs`.
- On macOS, insert it into the Runner app menu after About and before the standard service/hide/quit group.
- On non-macOS, append it to Help alongside "Reveal logs in Finder" / the platform equivalent.
- Extend the existing menu event handler so selecting the item shows/focuses the main window and emits `menu:check-for-updates`.

### Phase 2 - Frontend update trigger

- In `src/contexts/UpdateContext.tsx`, listen for `menu:check-for-updates`.
- Route the event to `checkForUpdate({ manual: true })` so the update toast displays active manual-check feedback.
- Keep the existing launch auto-check / auto-install effects intact unless the implementation finds dead coupling to the removed Settings pane.

### Phase 3 - Settings cleanup

- Remove the `updates` pane key and Updates sidebar entry from `src/components/SettingsModal.tsx`.
- Remove `UpdatesPane`, `UpdatesAction`, and imports that become unused.
- Keep Settings focused on General, Appearance, Terminal, Diagnostics, and About.

### Phase 4 - Verification

- Confirm Settings no longer shows an Updates item or update controls.
- Confirm macOS app menu includes "Check for Updates..." near About.
- Confirm selecting "Check for Updates..." triggers the update toast and reports checking / up-to-date / error states.
- Confirm available-update states still allow download/install and restart through the toast.
- Confirm launch auto-check still runs when the stored auto-install preference allows it.
- Run `pnpm exec tsc --noEmit`.
- Run `pnpm run lint`.
- Run the relevant Rust check for `src-tauri/src/lib.rs` menu changes.

## Verification

- [ ] Settings sidebar has no Updates item.
- [ ] Settings content has no update section.
- [ ] macOS app menu contains "Check for Updates..." near About.
- [ ] Non-macOS Help menu contains "Check for Updates..." if the menu is present.
- [ ] Native menu selection emits `menu:check-for-updates`.
- [ ] Manual check uses the existing update toast for feedback.
- [ ] Existing launch auto-check behavior is preserved.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
- [ ] Rust menu code compiles.
