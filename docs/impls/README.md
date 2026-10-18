# Implementation plans

An impl plan is tactical: the spec says *what*, the plan says how a change lands, what it touches and how it is verified. Architecture truth lives in [`../arch/`](../arch/); fold a decision that outlives the work into it before archiving the plan.

A plan is written on its feature's branch and lands in the pull request with the code. It stays here while the feature's remaining work is in flight. After shipping, delete small-feature, bug-fix and routine-refactor plans. Keep only important records indexed in [`archive/README.md`](archive/README.md), after preserving unique current decisions and updating references. Validation records and task history are retained separately. Since 2026-09-01 plans are named `{number}-{slug}.md` after their feature spec; the archived `0001`–`0045` sequence is closed. Mission briefs live in [`briefs/`](./briefs/), named `{number}-{slug}.md`, or `{number}-m{n}-{slug}.md` when a feature takes several missions. Shipped briefs are pruned except the references listed in [`briefs/README.md`](./briefs/README.md); the rest are in git history.

## Active

- [`836-web-remote-control.md`](./836-web-remote-control.md) — web remote control ([#836](https://github.com/yicheng47/runner/issues/836)): the program record for the `nightly-remote` umbrella, which missions branch from and target, follows `main` by rebase, ships installed builds as the separate `remote-nightly` release and reaches `main` as one PR; Phase 1's desktop side merged into it in [#945](https://github.com/yicheng47/runner/pull/945).

The [Linux brief](briefs/823-linux.md) remains active in [PR #982](https://github.com/yicheng47/runner/pull/982).

## Archive

The [archive README](archive/README.md) routes major programs, distinct migration/compatibility plans and preserved task logs. Completed small-change plans are deleted under the [archive policy](../README.md#archive-policy); separate specs and plans stay only when each has distinct lasting context. Test evidence and task/review history are retained, including embedded history extracted before a plan is pruned.
