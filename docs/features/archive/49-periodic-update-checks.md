# 49 — Periodic update checks for long-running sessions

> Tracking issue: [#333](https://github.com/yicheng47/runner/issues/333)

## Motivation

The updater checks exactly once per process: `UpdateContext` fires a single `checkForUpdate` ~3s after mount and never again. Runner is a long-running app — a cockpit left open for days or weeks never learns a release shipped, and the user only updates when something else forces a restart. Everything downstream of the check already handles the long-running case: `checkForUpdate` is re-entrant-guarded and resting-state-only, auto-install advances available → downloaded → ready, and the sidebar prompt card + Settings → About surface "ready" with a manual restart. Only the re-check triggers are missing.

## Scope

- **Interval re-check** — while the app runs, call the existing `checkForUpdate` on a timer (default ~6h). No new state machine; the resting-state guard makes repeated calls free.
- **Focus-triggered stale check** — track `lastCheckAt`; on window focus, re-check when the last check is older than the interval. This is the half that actually matters on a laptop: WKWebView throttles/suspends timers during sleep, so the overnight-lid-closed case is caught at the moment the user returns, not whenever a starved timer fires.
- **Never auto-relaunch.** Runner hosts live PTYs with running agents — an automatic restart is data loss by design. Restart stays a manual action on the existing surfaces; a staged update also applies on the next natural quit/launch, so even a user who ignores the card converges.
- **Decouple checking from the auto-install toggle.** Today the launch check is gated on `STORAGE_AUTO_INSTALL_UPDATES` (`UpdateContext.tsx:37`) — toggle off means *no check at all*. New semantics: checks always run (launch + interval + focus); the toggle governs auto-download only. Toggle-off becomes notify-only ("available" shows in the update surfaces) instead of never-knowing.
- **Dedicated Settings → Updates pane.** The update ladder (check / available / download progress / ready–restart), the auto-install toggle, and the current version move out of Settings → About into their own `UpdatesPane` in the settings nav. About keeps identity content (version line, credits, links) and loses the interactive updater.
- **macOS app-menu entry.** Add "Check for Updates…" to the application menu in `build_menu` (`lib.rs`), conventionally right below About. The menu handler emits an event to the webview; the frontend listener triggers `checkForUpdate` and navigates to Settings → Updates so the result is visible immediately. This is the standard macOS discoverability path for users who never open Settings.

Storage: none. The interval and staleness threshold are module constants; `lastCheckAt` is an in-memory ref (every launch re-checks anyway, so persisting it would only create a stale-suppression bug surface). Zero new persisted state; the existing toggle key narrows in meaning to download-only.

## Out of scope

- Any auto-restart, restart nagging, or install-on-quit hooks.
- Backend/Rust changes beyond the single app-menu item + event emit in `build_menu`.
- Changing the sidebar prompt card.
- A configurable interval — constant only; promote to a `settings.ts` key later if ever actually wanted.

## To be decided

- Interval length (default 6h; anything 4–12h is defensible — this is a freshness net, not a delivery SLA).
- Whether "error" from a background re-check should surface anywhere or stay silent until the next attempt (leaning silent — a transient network failure at 3am shouldn't leave a red badge). A menu-initiated check is explicit and DOES surface its error in the Updates pane.

## Implementation phases

1. **Re-check triggers** — interval timer + `lastCheckAt` + focus listener in `UpdateContext` (or the hook); launch check un-gated from the toggle; toggle governs the available → download transition only.
2. **Updates pane + menu item** — new `UpdatesPane` in the settings nav with the ladder/toggle/version moved from About; "Check for Updates…" item in `build_menu` emitting to the webview; frontend listener checks + navigates to the pane.
3. **Validation** — `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo check` for the menu wiring; unit-test the trigger policy (stale-on-focus fires, fresh-on-focus doesn't, toggle-off checks but doesn't download) with the dev status override / fake timers; manual smoke via the existing `runner.dev.updateStatus` escape hatch.

## Verification

- [ ] With the app left running past the interval, a staged release is detected without a restart (interval path).
- [ ] Sleep/wake past the staleness threshold triggers a check on focus (focus path).
- [ ] Auto-install ON: background check quietly reaches "ready"; prompt card appears; no automatic relaunch ever.
- [ ] Auto-install OFF: background check reaches "available" and surfaces it; no download starts until the user clicks.
- [ ] A background check error stays silent and the next trigger retries; a menu-initiated check surfaces its error in the Updates pane.
- [ ] Settings shows a dedicated Updates pane with the ladder, toggle, and version; About no longer hosts the updater.
- [ ] App menu → "Check for Updates…" opens Settings → Updates with a check in flight.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, and `cargo check` pass.
