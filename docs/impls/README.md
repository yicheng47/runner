# Implementation plans

Impl plans are tactical: the feature spec says *what*, the impl plan says how a specific change should land, what it touches, and how to verify it. Architecture truth lives in [`../arch/`](../arch/); fold decisions that outlive the work into it before archiving a plan.

A plan stays here while its work is in flight and moves to [`archive/`](./archive/) when it ships, keeping its filename so links stay stable — the listing answers "what is being built right now". Since 2026-09-01 new plans are named `{number}-{slug}.md` after their feature spec (whose number is the GitHub tracking issue); the archived `0001`–`0045` sequence is closed and keeps its numbers.

## Active

- [`555-mcp-settings.md`](./555-mcp-settings.md) — feature [555](../features/555-mcp-settings.md): the Settings → MCP pane shaped like Skills (runtime dropdown, one toggle per server row, pinned Runner row, click-to-detail, native-text edit with cross-agent sync), with the generalised `ops/mcp.rs` writer behind it.
- [`local-skills/`](./local-skills/README.md) — feature [73](../features/73-runner-skills.md) implementation program: condensed state and binding decisions, milestone plan, dated impl log.
- [`gpui-rewrite/`](./gpui-rewrite/) — the GPUI rewrite record; shipped as `v0.6.0` 2026-08-23. What is still open from the M6 consolidation queue is in [`m6-remainder.md`](./gpui-rewrite/m6-remainder.md); [#445](https://github.com/yicheng47/runner/issues/445) closed 2026-08-27 once the queued items landed.

## Archive

Shipped plans live in [`archive/`](./archive/) in number order; mission briefs live under [`archive/gpui-rewrite/briefs/`](./archive/gpui-rewrite/briefs/).

- [Settings and list scrollbars](./archive/553-settings-scrollbar-alignment.md) — bug [#553](https://github.com/yicheng47/runner/issues/553), shipped 2026-09-11 in [#558](https://github.com/yicheng47/runner/pull/558): the Settings scrollbar keeps the window edge with its track inset at both ends, and the Runners and Crews list scrollbars move off the cards to the page edge.
- [Runner Light](./archive/529-runner-light-theme.md) — feature [529](../features/archive/529-runner-light-theme.md), shipped 2026-09-11 in [#563](https://github.com/yicheng47/runner/pull/563): the first-party light theme, per-mode terminal palettes picked in Appearance with a clickable two-pane preview ([phase 4 brief](./archive/529-appearance-terminal-palettes.md)), Rosé Pine Dawn as the light terminal default, Codex Light and Monokai removed.
- [MCP project tools](./archive/554-mcp-project-management.md) — feature [554](../features/archive/554-mcp-project-management.md), shipped 2026-09-10 in [#556](https://github.com/yicheng47/runner/pull/556): `project_create` / `project_rename` / `project_delete` / `mission_set_project` over the sidebar ops, with a running-member guard on delete.
- [Runtime enum](./archive/538-runtime-enum.md) — issue [#538](https://github.com/yicheng47/runner/issues/538), shipped 2026-09-10 in [#543](https://github.com/yicheng47/runner/pull/543): bare runtime name strings replaced by a `Runtime` enum, string database compatibility kept, no migration.
- [Nightly channel](./archive/504-nightly-channel-unification.md) — features [502](../features/archive/502-unified-nightly.md), [504](../features/archive/504-single-nightly-release.md), [505](../features/archive/505-macos-nightly-replaces-runner.md) and the #507 tag/changelog follow-up, shipped 2026-09-08; the first brief is [502](./archive/502-unified-nightly.md). The contract lives in [arch §14](../arch/arch.md#14-program-state--line-landing-channels).
- [Windows port](./archive/windows-nightly/README.md) — feature [437](../features/archive/437-windows-nightly.md), shipped in 0.8.0. Current build and installer instructions live in [Windows development](../arch/windows.md).
