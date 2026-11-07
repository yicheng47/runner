# Implementation plans

An impl plan is tactical: the spec says *what*, the plan says how a change lands, what it touches and how it is verified. Architecture truth lives in [`../arch/`](../arch/); fold a decision that outlives the work into it before archiving the plan.

A plan is written on its feature's branch and lands in the pull request with the code. It stays here while the feature's remaining work is in flight. After shipping, delete small-feature, bug-fix and routine-refactor plans. Keep only important records indexed in [`archive/README.md`](archive/README.md), after preserving unique current decisions and updating references. Validation records and task history are retained separately. Since 2026-09-01 plans are named `{number}-{slug}.md` after their feature spec; the archived `0001`–`0045` sequence is closed. Mission briefs live in [`briefs/`](./briefs/), named `{number}-{slug}.md`, or `{number}-m{n}-{slug}.md` when a feature takes several missions. Shipped briefs are pruned except the references listed in [`briefs/README.md`](./briefs/README.md); the rest are in git history.

## Active

No program is active. File preview ([#634](https://github.com/yicheng47/runner/issues/634)) landed from its umbrella branch in [#1157](https://github.com/yicheng47/runner/pull/1157) and shipped in 0.19.5; its [program record](archive/634-file-preview/README.md), [plan](archive/634-file-preview/plan.md) and [log](archive/634-file-preview/impl_log.md) are archived.

The post-0.17 cleanup ([#977](https://github.com/yicheng47/runner/issues/977)) closed on 2026-10-29; its four plans were deleted in the 0.19.0 sweep, with their outcomes in the [maintenance history](archive/maintenance-history.md#977--post-017-cleanup-outcomes) and their validation records in the [test archive](../tests/README.md).

Remote control's [program record](archive/836-web-remote-control/README.md) shipped in 0.18.0. Linux shipped as a beta in 0.18.2; its [integration record](archive/982-linux-remote-compat.md) is archived, and the [brief](briefs/823-linux.md) remains as the large example.

## Archive

The [archive README](archive/README.md) routes major programs, distinct migration/compatibility plans and preserved task logs. Completed small-change plans are deleted under the [archive policy](../README.md#archive-policy); separate specs and plans stay only when each has distinct lasting context. Test evidence and task/review history are retained, including embedded history extracted before a plan is pruned.
