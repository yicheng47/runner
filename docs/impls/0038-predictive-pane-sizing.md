# Launch estimate: the fallback returns the wrong box

## Status

Planned, and **substantially narrowed**. Tracking issue [#366](https://github.com/yicheng47/runner/issues/366).

## Correction — the original premise was wrong

The first draft of this document (committed as `20979b8`, never implemented) claimed that `chatPaneAreaBox()` composes a `<main>` measurement taken on the `/runners` boot route with chrome subtractions that only apply to the chat surface — "a measurement of one surface, plus chrome corrections for another, describes neither." That is false, and [#366's comment](https://github.com/yicheng47/runner/issues/366#issuecomment-5101422726) is correct to reject it.

There is exactly one shell `<main>` — `AppShell.tsx:120` — a flex sibling of `<Sidebar>` with `flex-1`, with `PersistentSurfaces` mounted inside it. Its width is `window − sidebar` and it is **route-independent**: same element, same box, on `/runners` and on a chat route. What is currently rendered inside it does not change its width. `terminalSizing.ts:142-144` says this outright — *"Mounted from boot, so this reads true even with no chat or mission surface on screen"* — and the original draft failed to weigh it.

So subtracting the stored chat-panel width from that box is not a mis-applied measurement. It is a deliberate prediction of the destination pane area, which is exactly what a launch estimate must be.

The localStorage-per-origin argument fails on its own terms too. `storedSideWidth` and `RunnerChat.tsx:272` read the **same key** with the **same** absent-means-open default, so within one origin the estimate and the surface the session returns into agree. Dev and packaged profiles do have separate stores, but both halves of the computation live in the same origin, so that divergence cannot produce an asymmetry between them.

### The original fix would have made things worse

The draft proposed replacing the `<main>` read with a DOM-free reconstruction from `window.innerWidth` minus stored chrome. That would discard information the measurement gets for free:

`Sidebar.tsx:2004-2010` sets `width: sidebarVisible ? width : 0` on a `shrink-0` flex item, and the collapsed hover-peek switches it to an absolute overlay. Measuring `<main>` therefore already accounts for collapsed (0), expanded (the dragged width), mid-animation (the interpolated width), and peek-overlay (0) — correctly, and without knowing any of those rules. A reconstruction would have to re-derive all of it from localStorage and would get the overlay and animation cases wrong.

Reading the live layout is the right call here. The draft's own principle — prefer a value that described a real box over a computed one — argued against its own conclusion.

## What actually survives

One defect, in `shellContentBox` (`terminalSizing.ts:145-152`):

```js
return {
  width: rect && rect.width > 0 ? rect.width : window.innerWidth,
  height: rect && rect.height > 0 ? rect.height : window.innerHeight,
};
```

`window.innerWidth` is the full window **including** the sidebar, so the fallback silently substitutes a box roughly a sidebar-width too wide. It is a fallback that answers a different question rather than declining to answer.

A concrete reachable path: `SettingsPage.tsx:233` renders a **second** `<main>` while AppShell's is `display:none` under the Settings takeover (`AppShell.tsx:102-105`). `document.querySelector("main")` returns the first in document order — AppShell's hidden one — whose rect is zero, so the fallback fires and over-estimates.

Note the sign: this makes the estimate too **wide**, the opposite of #366's reported symptom. It is a real defect and worth fixing, but it does not explain the report.

## The reported symptom is #367

The screenshot behind "0.4.6 still didn't fix the restart width problem" shows a correctly-wrapped full-width frame with **no scrollback above it**. That is not a width bug — it is [#367](https://github.com/yicheng47/runner/issues/367): MCP-started missions fork slots at 80×24, and the first slot visit trips the cols-gate purge and discards the ring. See [impl 0039](0039-backend-initiated-spawn-width.md).

Absent a reproduction that survives the analysis above, there is no confirmed launch-estimate width bug in the shipped tree.

## Key Decisions

1. **Keep the `<main>` measurement.** It is route-independent, and it captures sidebar collapse, drag width, animation, and peek-overlay state for free. Do not replace it with a reconstruction from stored values.
2. **Fail null instead of substituting a different box.** When `<main>` is missing or zero-width, return null and let the precedence chain fall to the persisted rung. A persisted `last_cols` described a real pane once; `window.innerWidth` describes a box that includes chrome the terminal never gets.
3. **Fix the multiple-`<main>` selector while here.** `querySelector("main")` picking a hidden element is a latent trap independent of the fallback. Scope the lookup to the shell's own element rather than relying on document order.
4. **No change to the precedence chain.** 0036's measured → estimated → persisted → default ordering stands, and its decision 2 was right after all — no correction note is owed to it.

## Non-Goals

- Replacing, rewriting, or DOM-freeing the estimate. That was the original draft's proposal and it is withdrawn.
- Anything in #367 / impl 0039.
- Changing the window-settle gate, the 300ms stagger, or #320's auto-resume semantics.

## Implementation Phases

### Phase 1 — fallback and selector

- `shellContentBox` returns null when it cannot measure the shell's `<main>`; callers propagate null rather than substituting.
- Scope the element lookup so a Settings-route `<main>` cannot shadow the shell's.

### Phase 2 — tests and validation

- Unit-test: absent `<main>` → null; zero-rect `<main>` → null; two `<main>`s with the first hidden → the shell's box, not the other one.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

## Verification

Automated:

- [ ] No measurable `<main>` yields null, and the launch path defers to the persisted rung rather than forking on `window.innerWidth`.
- [ ] A hidden first `<main>` does not shadow the shell's.
- [ ] Estimates with a normally-laid-out `<main>` are unchanged, panel open and closed.

Manual (Jason smoke-tests, **packaged build**):

- [ ] Auto-resume with the Settings surface open at quit → restored panes wrap correctly.
- [ ] Ordinary quit/relaunch with the panel open → unchanged from today.
