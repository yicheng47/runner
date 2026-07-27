# WebGL texture atlas invalidation

## Status

Shipped. Tracking issue [#360](https://github.com/yicheng47/runner/issues/360). Wake behavior confirmed by smoke test on 2026-07-27. Phase 1's findings and the answered open questions are recorded below.

## Problem

After the Mac wakes from sleep, terminal panes repaint with glyphs jammed together — characters drawn at the wrong advance width, so `$ pnpm tauri dev` renders closer to `$pnpmtauridev`. Wrapping is correct; only the glyph raster is wrong. The WebGL renderer's texture atlas holds glyphs rasterized under conditions that no longer hold, and nothing tells it so.

Three facts, all verified in the current tree:

1. **`clearTextureAtlas()` has exactly two call sites** — `RunnerTerminal.tsx:1053` and `:1059`, both inside the storage-event handler for terminal font size and font family. Their comment already names this exact symptom: *"a stale cache after a font change can leave a band of pre-change glyphs at the new size until something else evicts them."*
2. **The wake path never calls it.** Every wake trigger funnels into `refreshActiveTerminal`, which does `ensureWebglRenderer()` → `fit.fit()` → `t.refresh(0, t.rows - 1)`. `t.refresh()` only marks lines dirty; the renderer faithfully redraws them from the same stale atlas.
3. **No backing-scale change is detected anywhere.** There is no `devicePixelRatio` listener in `src/` — the only DPR mentions are inside `windowSettle.ts`, which reads it but does not watch it.

### A fourth trigger the issue doesn't cover

`STORAGE_APP_ZOOM` is **absent** from the storage-event handler that clears the atlas on font changes (`RunnerTerminal.tsx:1042-1073` handles font size, font family, cursor style, scrollback, and theme — not zoom). App zoom changes the device-pixel size of every rendered glyph, so it stales the atlas exactly like a font-size change does.

And a DPR watcher would not catch it. `windowSettle.ts:13-14`, written during #363, records the reason: **WebKit does not fold page zoom into `devicePixelRatio`** (webkit#124862). So under app zoom the atlas goes stale while every signal a naive watcher would monitor stays constant.

This matters out of proportion to its size: **app zoom is a deterministic, on-demand reproduction of a bug that otherwise requires sleeping the machine.** Confirm it reproduces first, then develop against it.

> **Corrected after testing (2026-07-27).** It does not reproduce. Jason smoke-tested it: app zoom leaves glyphs correctly spaced. The paragraph above is wrong about the mechanism — page zoom moves nothing the atlas config keys on, so `configEquals` returns true and the same atlas is reused rather than going stale. Zoom was never a repro, and this plan had no deterministic one; the rest of the work was verified at the source and by unit tests instead. Decision 4 still ships, downgraded from "most certain part of the fix" to cheap insurance — see the open questions.

### Why the wake signal is the hard part

`RunEvent::Resumed` → `app/resumed` (`lib.rs:554`) is the only trigger that forces the strong refresh, and it is an app-lifecycle event, not a system sleep/wake one. There is no `NSWorkspace.didWakeNotification` observer. A real system wake typically surfaces only as a window focus change, which routes to the non-forcing `scheduleWakeRefit()`.

Hanging the atlas clear on focus is the obvious shortcut and the wrong answer: focus fires on every alt-tab, and clearing forces every glyph to re-rasterize on the next draw. Cheap once after a wake, wasteful dozens of times an hour.

## Key Decisions

1. **Clear the atlas on the events that actually invalidate it, not on a proxy for them.** Three distinct invalidators, each wired to its own real signal: system wake, backing-scale change, and app zoom. Focus is not one of them — it correlates with wake but fires constantly otherwise, and this codebase has already paid twice (#352, #363) for hanging behavior on a correlated proxy instead of the true signal.
2. **System wake becomes a first-class signal via `NSWorkspace.didWakeNotification`.** Runner is macOS-only (AGENTS.md), so a native observer costs no portability, and `objc2-app-kit` is already a dependency — this needs an added feature, not a new crate. Emit it on the existing `<domain>/<verb>` event convention (e.g. `app/woke`) so the frontend consumes it like any other backend event. This is the precise signal; everything else is inference.
3. **Watch backing scale directly.** A `matchMedia("(resolution: Xdppx)")` listener, re-registered on each change, catches lid-open-on-a-different-monitor and display re-initialization — cases where wake and scale change are independent. Note this is *complementary* to decision 2, not redundant: a GPU texture reset with unchanged scale fires no resolution change, and a monitor move fires no wake.
4. **App zoom joins the existing font-change path.** Add `STORAGE_APP_ZOOM` to the storage-event handler beside font size and family, with the same `clearTextureAtlas()` + `refitAndPush()` treatment. ~~This is the smallest and most certain part of the fix, and the only one with a deterministic repro.~~ **Corrected: it was the *least* certain part.** Zoom does not reproduce the symptom (see above), so this clear is probably inert. Kept regardless — it is one predicate entry on an explicit Cmd +/- press, and "probably inert" is read off the addon source, not measured. Delete it on evidence, not on that reasoning.
5. **Clearing is the whole remedy; do not add a forced repaint.** `clearTextureAtlas()` drops the cache and glyphs re-rasterize lazily on the next draw, which the existing `t.refresh()` on the wake path already triggers. Do **not** reach for the forced resize dance — it fixes geometry, not rasters, and #352 exists precisely because that dance was over-applied.
6. **Do not clear on ordinary activation.** Tab switches and pane activation must stay atlas-preserving. If a case emerges where activation genuinely needs it, that is a separate finding with its own evidence, not an extension of this one.

## Open questions — answered

**Does `RunEvent::Resumed` fire on macOS system wake at all?** No, and worse than the issue assumed: it does not fire on macOS *at all*, wake or otherwise. Three links in the locked dependency tree:

1. `tauri-runtime-wry` 2.10.1 `src/lib.rs:4038-4040` maps `RunEvent::Resumed` from `Event::NewEvents(StartCause::Poll)` — not from tao's `Event::Resumed`.
2. The same function, `src/lib.rs:4029-4031`, forces `*control_flow = ControlFlow::Wait` on every iteration that is not `Exit`.
3. `tao` 0.34.8 `src/platform_impl/macos/app_state.rs:329-347` produces `StartCause::Poll` only under `ControlFlow::Poll`; `ControlFlow::Wait` yields `WaitCancelled` or `ResumeTimeReached` instead.

Independently, tao's own `Event::Resumed` is emitted only from the iOS (`platform_impl/ios/view.rs:615`) and Android backends — `platform_impl/macos/` contains zero occurrences. So decision 2 does **not** collapse; the native observer is required. The existing `app/resumed` handler is left in place untouched as a #352/#363-owned geometry path.

**Does app zoom actually reproduce the symptom?** **No.** Smoke-tested by Jason on 2026-07-27: changing app zoom leaves glyphs correctly spaced. So this plan never had the deterministic repro it was counting on, and phase 1 delivered one empirical answer instead of two.

The addon source agrees, which is worth recording because it explains *why* the plan's reasoning failed. The atlas config is keyed on `devicePixelRatio` plus the CSS-derived char metrics (`CharAtlasUtils.ts:34-53`, `configEquals` at `:55-76`). Page zoom moves neither: WebKit holds `devicePixelRatio` at the display scale (webkit#124862) and CSS char metrics are unchanged, so `configEquals` returns true and the same atlas is reused rather than going stale. The canvas device-pixel box *does* grow — `DevicePixelObserver` sees it and `_setCanvasDevicePixelDimensions` resizes the canvas — but the glyph shader normalizes against the deliberately-retained older `device.canvas` dimensions (`WebglRenderer.ts:680-690`, `GlyphRenderer.ts:324`), so the whole grid scales uniformly. Bigger, not misaligned.

The plan's error was treating "changes the on-screen size of a glyph" as equivalent to "changes what the atlas keys on." They are different questions, and only the second one stales a cache.

Decision 4 ships anyway, on Jason's call, downgraded to insurance: one predicate entry on an explicit Cmd +/- press, against a mechanism argument that is read off dependency source rather than measured. If WebKit rounds a CSS char metric differently at some zoom step the config genuinely differs, and this is the only thing that would notice. Delete it on evidence, not on the reasoning above.

**Is a full atlas clear the right granularity?** Yes — it is the only granularity the addon exposes. `clearTextureAtlas()` is the sole invalidation entry point on the public surface (`typings/addon-webgl.d.ts`), implemented as `_charAtlas?.clearTexture(); _clearModel(true); _requestRedrawViewport()` (`WebglRenderer.ts:332-336`). Something narrower does exist internally — `acquireTextureAtlas` re-keys on a config that includes `devicePixelRatio` and char metrics (`CharAtlasUtils.ts:55-76`) — but it is reachable only from `handleResize` / `handleDevicePixelRatioChange`, not from the addon's API.

**Multi-pane cost.** Acceptable; no staggering. Cheaper than the question assumed, for two independent reasons:

- Mounted is not the same as holding an atlas. `RunnerTerminal` disposes the WebGL addon whenever a pane leaves the foreground (the `[active]` effect, `RunnerTerminal.tsx:1146-1152`), so only the handful of foreground panes have anything to clear. A pane that was its atlas's sole owner drops the cache entry outright on dispose (`CharAtlasCache.ts:85-100`).
- The atlas is shared, not per-pane. `charAtlasCache` is module-level and keyed by config, so panes at the same font and scale hold one `TextureAtlas` between them — the re-rasterization is paid once, not once per pane.

### A fifth finding: xterm already watches backing scale

Fact 3 in the problem statement ("no `devicePixelRatio` listener in `src/`") is true of Runner's own code and false of the bundled dependency. xterm core ships `ScreenDprMonitor` (`CoreBrowserService.ts:65-135`) — the same `(resolution: Xdppx)` query, with the same re-registration-per-change — wired through `RenderService.ts:83` to `WebglRenderer.handleDevicePixelRatioChange()` (`WebglRenderer.ts:180-187`), which re-acquires an atlas keyed on the new ratio.

A plain backing-scale change is therefore already handled upstream. Decision 3's listener still ships, as defense-in-depth for the case upstream misses: a display re-initialization that drops GPU texture contents while landing on a scale the config cache has already seen, where `configEquals` returns true and the stale entry is reused. It does not change the shape of the fix, and the cost is one clear on an event that fires when a monitor changes.

## Goals

- After system wake, panes repaint with correctly spaced glyphs, needing no window resize or font-size toggle.
- Moving the window to a display with a different backing scale repaints correctly.
- Changing app zoom repaints correctly.
- Ordinary tab switching and focus changes do not clear the atlas.

## Non-Goals

- The forced resize dance, geometry, or PTY sizing — this is a raster-only defect (#352, #363 own the geometry paths).
- Reverting the `@xterm/*` beta pin, which fixed a different upstream glyph-corruption bug and stays.
- A DOM-renderer fallback or any change to `onContextLoss` handling for full context loss.
- Cross-platform wake detection.

## Implementation Phases

### Phase 1 — reproduce deterministically

- Confirm app zoom reproduces the overlapped-glyph symptom. If it does, it is the development repro for the rest of the work. → **It does not.** There is no deterministic repro; the rest of the work was verified at the source and by unit tests. See the open questions.
- Verify whether `RunEvent::Resumed` fires on real system wake; record the answer, since it decides Phase 2's size. → It does not fire on macOS at all. See the open questions.

### Phase 2 — wire the three invalidators

All three landed. The invalidation rules moved out of the event handler into `src/lib/textureAtlas.ts`, since #360's shape was an invalidator missing from a list that lived inline in one `if`/`else` chain.

- App zoom → `stalesTextureAtlas()` now owns the storage-key list (font size, font family, app zoom) and the handler consults it, so the list and its use cannot drift apart.
- Backing scale → `observeBackingScale()`, a `(resolution: Xdppx)` listener re-registered on each change.
- System wake → `src-tauri/src/wake.rs`: an `NSWorkspace.didWakeNotification` observer emitting `app/woke`, consumed in `RunnerTerminal`. `app/resumed` is untouched.

### Phase 3 — verification

- Tests where the seam allows: `src/lib/textureAtlas.test.ts` and `wake.rs`'s module test.
- Full check matrix: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

## Verification

Automated:

- [x] An app-zoom storage event clears the atlas and refits — `stalesTextureAtlas` is asserted true for `STORAGE_APP_ZOOM`, and it is the sole gate on the handler's `clearTextureAtlas()` + `refitAndPush()`. Note this verifies the wiring, not a fix: zoom does not reproduce the symptom, so there is nothing here for the clear to repair.
- [x] A resolution-change event clears the atlas — the watcher is asserted to fire on change and, crucially, to re-register at the new scale so the *second* display move is caught too.
- [x] Ordinary activation / tab switch does **not** clear the atlas — held by construction rather than by a test: `clearTextureAtlas()` has exactly three call sites (storage handler, wake listener, backing-scale listener) and none is on the activation path. `stalesTextureAtlas` is asserted false for cursor style, scrollback, theme, unrelated keys, and `null`.
- [x] The wake event reaches the terminal and clears — `wake.rs`'s test posts `NSWorkspaceDidWakeNotification` to the workspace notification center and asserts the observer runs, which pins the registration and the notification name without sleeping the machine. The `app/woke` → `clearTextureAtlas()` hop is a one-liner in the component, outside the unit seam.

Manual (Jason smoke-tests):

- [x] Change app zoom with visible output on screen → glyphs stay correctly spaced. Done 2026-07-27: they do, and they did before this change too — zoom never broke them.
- [x] Sleep the Mac with an active pane, wake, focus Runner → glyphs correct without touching anything. **Confirmed 2026-07-27** — the fix works. This is the one that mattered: with no deterministic repro available, it is the only end-to-end evidence that clearing on `app/woke` repairs the symptom #360 reported.
- [ ] Move the window between displays of different scale → glyphs correct. Not run — decision 3's watcher is unvalidated in the field, and xterm's upstream `ScreenDprMonitor` may be doing the real work here regardless.
- [ ] Alt-tab repeatedly → no visible re-rasterization stutter. Not run — the no-clear-on-focus constraint is held by construction (no call site on the focus path), not by observation.
