# Implementation plans

Impl plans are tactical: the feature spec says *what*, the impl plan says how a specific change should land, what it touches, and how to verify it. Architecture truth lives in [`../arch/`](../arch/); fold decisions that outlive the work into it before archiving a plan.

A plan stays here while its work is in flight and moves to [`archive/`](./archive/) when it ships, keeping its filename so links stay stable — the listing answers "what is being built right now". Since 2026-09-01 new plans are named `{number}-{slug}.md` after their feature spec (whose number is the GitHub tracking issue); the archived `0001`–`0045` sequence is closed and keeps its numbers.

## Active

- [`474-mission-terminal-drawer.md`](./474-mission-terminal-drawer.md) — feature [474](../features/474-mission-terminal-drawer.md) build plan ([#474](https://github.com/yicheng47/runner/issues/474)): the mission terminal drawer — the drawer model lifted out of `PaneLayout`, a serde-defaulted mission-node layout, shells attached by the workspace, the action / palette / ⌘W routed by surface. Planned 2026-09-04.
- [`fork-chat.md`](./fork-chat.md) — feature [60](../features/60-fork-chat-to-pane-or-tab.md) build plan ([#398](https://github.com/yicheng47/runner/issues/398)): native fork of a chat into a new tab behind two fork surfaces. In flight 2026-09-01; predates the naming rule and archives as-is.
- [`local-skills/`](./local-skills/README.md) — feature [73](../features/73-runner-skills.md) implementation program: condensed state and binding decisions, milestone plan, dated impl log.
- [`gpui-rewrite/`](./gpui-rewrite/) — the GPUI rewrite record; shipped as `v0.6.0` 2026-08-23. What remains is the M6 consolidation queue in [`m6-remainder.md`](./gpui-rewrite/m6-remainder.md), tracked in [#445](https://github.com/yicheng47/runner/issues/445).

## Archive

Shipped plans live in [`archive/`](./archive/) in number order; mission briefs live under [`archive/gpui-rewrite/briefs/`](./archive/gpui-rewrite/briefs/).
