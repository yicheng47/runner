# Predictive pane sizing for launch resume

## Status

Planned. Tracking issue [#366](https://github.com/yicheng47/runner/issues/366). **Corrects [0036](0036-launch-resume-fork-width.md)**, whose precedence chain is sound but whose estimated rung was wired to the wrong helpers — see Problem.

## Problem

0036 shipped and the symptom survived in packaged builds. The root cause is a category error:

**The estimate answers a question about a layout that is not rendered by measuring the layout that is.**

At cold launch the main window boots on `/runners` (`App.tsx:117`), so no chat surface exists and the *measured* rung is unreachable — `launchDims.ts:8-9` documents this. Every auto-resumed session therefore falls to the *estimated* rung, which does:

```
chatPaneAreaBox()                  // terminalSizing.ts:210
  → shellContentBox()              // :145 — document.querySelector("main")
  → width − storedSideWidth("runner.chat.panel.open", …, default 320)
```

`<main>` on `/runners` holds the runners list and no chat side panel, but the panel width is subtracted anyway. A measurement of one surface, plus chrome corrections for another, describes neither. With the panel open — which is the default, since `RunnerChat.tsx:272` treats a missing key as open — the estimate is `CHAT_PANEL_DEFAULT_WIDTH_PX` (320) too narrow, roughly 30–35 columns.

`shellContentBox()` carries the same error one level down: when `<main>` is missing or zero-width it falls back to `window.innerWidth`, the full window *including* the sidebar. Opposite sign, identical mistake — substituting a different box rather than declining to answer.

Structurally: these helpers were written for the **live** case, where the chat surface is mounted and measure-then-subtract is exactly right. 0036's decision 2 named them for the **predictive** case without noticing that their correctness depends on the surface being on screen. They are correct where they came from and wrong where they were reused. That instruction was in the impl doc, not an implementation slip.

### Why dev looked fixed

Two stores diverge between builds, which is why this reproduced only in production:

1. **localStorage is per-origin.** Dev serves from `http://localhost:1420`; a packaged build uses the Tauri asset protocol. `runner.chat.panel.open` / `.width` are not shared. A dev profile with the panel closed subtracts 0 and looks correct.
2. **Database and window state are separate** — `lib.rs:126-134` gives debug builds a sibling `<identifier>-dev` data dir. Different sessions, tab layouts, and split shapes.

## Key Decisions

1. **Predictive sizing is its own computation, with no DOM input.** Add a distinct path for "how wide will this pane be once its surface exists," derived from window geometry plus stored chrome. It must not call `document.querySelector` — reading the current DOM is precisely what makes the answer describe the wrong surface. The live helpers stay as they are for the mounted case; this is a sibling, not a replacement.
2. **Inputs are all available without rendering anything.** `window.innerWidth` / `innerHeight`; sidebar from `runner.sidebar.collapsed` and `runner.sidebar.width` (`Sidebar.tsx:155` — it is draggable, so the width is stored, and collapsed renders as an overlay costing no layout width — confirm); chat side panel from `runner.chat.panel.open` / `.width`; `CHAT_HEADER_HEIGHT_PX`; and the pane's share of its tab from the persisted layout via `paneBoxForSession`. Mission sessions take the mission chrome constants the same way.
3. **Fail null, never approximate.** When any input is missing or implausible, return null and let the backend's persisted rung take over. A persisted `last_cols` described a real pane once; a confidently wrong estimate never did. This is the principle that would have prevented both defects in this family, and it applies especially to the `window.innerWidth` fallback — delete it rather than repair it.
4. **Keep the measured rung.** It is dead at cold launch today, but it is correct whenever it does resolve, costs nothing, and becomes live if the boot route changes or a second window ever resumes. Document its reachability where it is defined so the next reader does not mistake it for the working path.
5. **Verify against a packaged build.** `pnpm tauri dev` cannot reproduce this class — separate origin, separate database. Any check that only passes under `tauri dev` proves nothing here. Unit tests should drive the computation directly with injected geometry and stored values rather than through a rendered tree.

## Open questions

- **Collapsed sidebar width.** Confirm the collapsed sidebar is an overlay contributing 0 to layout width rather than a persistent thin strip; the peek overlay behavior suggests overlay, but the predictive path needs the exact number.
- **Zoom.** `window.innerWidth` is in CSS pixels and app zoom scales them, so the stored chrome constants and the window measurement should already be in the same space — worth confirming rather than assuming, given #360's WebKit zoom surprise.
- **Whether the live helpers should also stop falling back.** `shellContentBox`'s `window.innerWidth` fallback is wrong in the live case too, just less visibly. Fixing it there is in scope if it does not disturb mounted-pane behavior; if it does, that is a separate finding.

## Goals

- A session auto-resumed at launch in a **packaged** build forks at the width it will actually be displayed at, with the chat side panel open and several tabs present.
- The width estimate never silently substitutes a different box; unknown inputs produce null and defer to the persisted value.
- Live, mounted-pane sizing is unchanged.

## Non-Goals

- Changing 0036's precedence order — measured → estimated → persisted → default is right; only the estimated rung's implementation is wrong.
- Changing the window-settle gate, the 300ms stagger, or anything in #320's auto-resume semantics.
- Repairing scrollback already mis-wrapped by earlier versions.
- Reworking live terminal sizing for mounted panes beyond the fallback noted in open questions.

## Implementation Phases

### Phase 1 — predictive sizing path

- Add the DOM-free computation described in decisions 1–2, for both chat and mission destinations.
- Point `launchDims.ts`'s estimate rung at it; leave the measured rung intact with a reachability comment.
- Remove the `window.innerWidth` fallback from the predictive path entirely (decision 3).

### Phase 2 — tests

- Drive the computation directly with injected window dimensions and stored chrome: panel open vs closed, sidebar expanded vs collapsed, single pane vs 2- and 3-pane splits, missing keys, implausible values.
- Assert the null-not-approximate contract: any missing input yields null rather than a number.

### Phase 3 — verification and record

- Full check matrix: `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Append a correction note to 0036 decision 2 pointing at 0038 and #366.

## Verification

Automated:

- [ ] Predictive width with the panel open matches panel-closed width minus the stored panel width — and neither reads the DOM.
- [ ] Collapsed vs expanded sidebar changes the result by the stored sidebar width.
- [ ] A session in a 2-pane split gets roughly half the pane area; a 3-pane `cols-3` and a `main-2` with the same pane count give different boxes.
- [ ] Any missing or implausible input yields null.
- [ ] Mounted-pane (live) sizing is unchanged.

Manual (Jason smoke-tests, **packaged build only**):

- [ ] Panel open, auto-resume on, quit and relaunch → restored pane wraps at the correct width.
- [ ] Same with the panel closed.
- [ ] Same with the sidebar collapsed, and with a widened sidebar.
- [ ] Same for a session in a 2-pane split tab.
- [ ] Resize the window between quit and relaunch → still correct (the #363 case stays fixed).
