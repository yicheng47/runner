# Implementation plans

An impl plan is tactical: the spec says *what*, the plan says how a change lands, what it touches and how it is verified. Architecture truth lives in [`../arch/`](../arch/); fold a decision that outlives the work into it before archiving the plan.

A plan stays here while its work is in flight and moves to [`archive/`](./archive/) when it ships, keeping its filename so links stay stable. Since 2026-09-01 plans are named `{number}-{slug}.md` after their feature spec; the archived `0001`–`0045` sequence is closed. Mission briefs live in [`briefs/`](./briefs/), named `{number}-m{n}-{slug}.md`, and stay there after the mission ships.

## Active

- [539 — pi runtime](./539-pi-runtime.md) — feature [539](../features/539-pi-runtime.md) ([#539](https://github.com/yicheng47/runner/issues/539), 0.11): missions 1 and 2 shipped in [#646](https://github.com/yicheng47/runner/pull/646) and [#649](https://github.com/yicheng47/runner/pull/649); mission 3 (fixture, smoke test, JASONPC, archive) remains.

## Archive

Shipped plans are in [`archive/`](./archive/) in number order, and each names its PR. Multi-mission programs keep a folder with a condensed README:

- [`347-hook-status/`](./archive/347-hook-status/README.md) — hook-based agent status on macOS ([#347](https://github.com/yicheng47/runner/issues/347), closed 2026-09-16); Windows followed as [610](./archive/610-windows-hook-status.md), shipped in 0.10.0.
- [`604-role-rename/`](./archive/604-role-rename/README.md) — the runner entity became role ([#604](https://github.com/yicheng47/runner/issues/604), shipped 2026-09-16 in [#618](https://github.com/yicheng47/runner/pull/618)).
- [`local-skills/`](./archive/local-skills/README.md) — the Skills pane program ([#73](https://github.com/yicheng47/runner/issues/73), closed 2026-09-13; per-role picks continue as [#577](https://github.com/yicheng47/runner/issues/577)).
- [`gpui-rewrite/`](./archive/gpui-rewrite/README.md) — the GPUI rewrite, shipped as `v0.6.0` on 2026-08-23; its dated log, surface inventory and mission briefs were pruned on 2026-09-18 and live in git history.
- [`windows-nightly/`](./archive/windows-nightly/README.md) — the Windows port ([#437](https://github.com/yicheng47/runner/issues/437)), shipped in 0.8.0; current build instructions are in [`../arch/windows.md`](../arch/windows.md).
