# 09 — Persistent auto-update toast

> Tracking issue: [#111](https://github.com/yicheng47/runner/issues/111)

## Motivation

Runner's auto-update is on by default. Today the lifecycle is:

1. `UpdateProvider` waits ~3s after mount, then calls `check()`.
2. If a new release exists, `status` becomes `"available"`.
3. Because `STORAGE_AUTO_INSTALL_UPDATES` is true by default, the
   provider immediately calls `downloadAndInstall()`, which moves
   `status` to `"downloading"` on the next tick.
4. After the download completes, `status` becomes `"ready"` and the
   user is expected to restart from Settings → Updates.

`UpdateToast` only renders when `status === "available"`. With
auto-install on (the default), that window lasts a few milliseconds —
the toast's enter transition is 200ms, but the status flips before the
toast can paint. The code comment in `UpdateContext.tsx` says the
toast "still shows the available pill briefly so it doesn't feel like
the update happened in secret," but in practice it does happen in
secret: a user opens the app, sees nothing, closes it, and on next
launch is on a new version with no idea anything changed.

This is the gap. The toast component already exists; it just doesn't
cover the rest of the auto-update lifecycle.

## Scope

### In scope (v1)

- **Persistent toast across the full auto-update lifecycle.** Render
  for `"available"`, `"downloading"`, and `"ready"` instead of only
  `"available"`. Each state has its own copy and primary action:
  - `available` (manual-install mode only): "Runner v{x} is
    available" — primary action `Update` opens Settings → Updates.
  - `downloading`: "Downloading Runner v{x}… {n}%" — no primary
    action; the dismiss `×` stays. Optional inline progress bar
    underneath the text row.
  - `ready`: "Runner v{x} is ready to install" — primary action
    `Restart` calls `restart()` directly (skips Settings).
- **Auto-install vs manual paths converge on the toast.** With
  auto-install on (default), the user sees `downloading → ready`. With
  it off, they see `available → (click) → downloading → ready`. Either
  way, a toast is visible until the user dismisses or restarts.
- **No auto-dismiss while a download or ready state is active.** The
  existing 30s auto-dismiss applies only to `available` in
  manual-install mode (a missed prompt; comes back next launch).
  Dismissing `downloading` or `ready` hides the toast for the rest of
  the session but does not abort the install.
- **Re-show on next launch if `ready` was dismissed.** If the app
  restarts and a downloaded update is still pending (Tauri's updater
  exposes this via the next `check()` returning the same version), the
  toast surfaces again with the `Restart` action.

### Out of scope (deferred)

- **Release notes in the toast.** Click-through to a release-notes
  panel is a separate feature. The toast is a notification, not a
  reader.
- **Per-update opt-out / "skip this version."** Useful but introduces a
  storage shape (skipped versions list) we don't need yet. v1 just
  improves visibility.
- **A dock badge or system notification.** macOS dock badge / native
  notifications when the app is backgrounded would be nice, but the
  toast is the in-app surface; OS-level surfaces are their own
  feature.
- **Download retry UI.** `status === "error"` already exists in the
  hook but is not surfaced. Surfacing errors is a related but separate
  concern; this spec does not change error behavior.

### Key decisions

1. **One toast, multiple states — not three toasts.** The component
   already owns its `dismissed` and `visible` state; extending the
   `shouldShow` predicate to include `downloading` and `ready` keeps
   the surface coherent. Stacking three toasts would feel noisy and
   require positioning logic.
2. **The `Restart` action lives in the toast, not only in Settings.**
   Auto-install's whole point is the user shouldn't have to dig
   through settings to finish an update. If we trust them enough to
   download in the background, the restart prompt should be one click
   away.
3. **Don't change the auto-install default.** This spec is purely
   about visibility. Whether auto-install stays on by default is a
   separate product question.
4. **Dismissal does not abort.** A user can dismiss the toast at any
   stage; the install continues. This matches every other
   background-task notification (Slack file uploads, Linear sync,
   etc.) and avoids a surprising "I clicked × and now my update is
   gone" failure mode.

## Implementation phases

### Phase 1 — toast state coverage

- `UpdateToast.tsx`: widen `shouldShow` to `status === "available" ||
  status === "downloading" || status === "ready"`.
- Branch the rendered copy and primary action on `status`. Pull
  `progress` from `useUpdate()` for the downloading state.
- The 30s auto-dismiss timer becomes conditional — only armed when
  `status === "available"` *and* auto-install is off (read
  `STORAGE_AUTO_INSTALL_UPDATES`).

### Phase 2 — restart action

- `ready` state's primary button calls `restart()` from the update
  context. Confirm the button is disabled-styled (or removed) for
  `downloading` — only the `×` is interactive while downloading.
- Add a thin progress bar inside the toast for `downloading` (already
  have `progress` 0–100). Width animates with `transition-all`.

### Phase 3 — design + edge cases

- Mock the three states in `design/runners-design.pen` against the
  current dark palette. Reference the existing `To8GR` node Bryce used
  for the available-state pill.
- Verify the toast position doesn't fight any modal that opens
  top-center.
- Confirm `error` state still cleanly returns to `idle` and hides the
  toast (no broken intermediate UI).
- Smoke test the full path with `pnpm tauri dev` against a fake
  endpoint: open the app, watch the toast persist `downloading → ready`,
  click `Restart`, confirm relaunch.

## Verification

- [ ] With auto-install on (default), opening the app surfaces a
      toast that stays visible through `downloading` and `ready`.
- [ ] With auto-install off, the toast appears with an `Update`
      button; clicking opens Settings → Updates; toast then advances
      through `downloading` and `ready` once download starts.
- [ ] Progress bar in the `downloading` toast advances 0 → 100.
- [ ] `Restart` button in the `ready` toast relaunches the app on the
      new version.
- [ ] Dismissing the toast during `downloading` does not abort the
      install; on next launch the `ready` toast reappears.
- [ ] No toast is shown when `status === "idle"` or `status ===
      "error"`.
- [ ] `pnpm exec tsc --noEmit` and `pnpm run lint` clean.
