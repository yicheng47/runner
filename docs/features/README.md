# Feature specs

Specs of features in development. A feature is filed as an issue only; its spec is written when development starts, on the feature's branch, and lands in the same pull request as the code. At a release sweep, default to deleting completed specs. Retain only major features or consequential migration/compatibility records under the [archive policy](../README.md#archive-policy), preserving unique current decisions and fixing references before pruning. Priorities and milestones live on the issues; [`../roadmap.md`](../roadmap.md) mirrors them.

Since 2026-09-01 a spec's number **is** its tracking issue number: file the issue first, then name the spec after it, so gaps in the sequence belong to bugs and PRs. Specs 01–64 predate the alignment and keep their numbers in `archive/`; the then-active ones were renumbered to their issues (05→73, 58→393, 61→403), and 60 (fork a chat) kept its number and shipped 2026-09-01.

## Active

- [748 — Mission updates reach the starting chat](./748-mission-watch-delivery.md) — the return path for delegation; on hold after #826 since 2026-10-17, because Jason rejected Runner typing updates into a normal chat, and the spec keeps the refreshed proposal for the redesign ([#748](https://github.com/yicheng47/runner/issues/748)).
- [634 — File preview and code review](./634-file-preview-and-review.md) — a project tree, read-only file/diff tabs, a base picker and review comments returned to the agent; design before implementation ([#634](https://github.com/yicheng47/runner/issues/634)).
- [559 — Command palette](./559-command-palette.md) — command mode over the existing keymap and quick-switcher overlay ([#559](https://github.com/yicheng47/runner/issues/559)).
- [836 — Web remote control](./836-web-remote-control.md) — Phase 1 connects a host to an account: device-code enrollment from Settings → Remote access or `runner remote login`, who may connect a host, secrets in the OS secret store, the outbound relay connection and its states; the companion protocol, encrypted channel and terminals follow, then a Tailscale route released together with the hosted service ([#836](https://github.com/yicheng47/runner/issues/836)); Phase 1's desktop side merged into the `nightly-remote` umbrella on 2026-10-18 ([#945](https://github.com/yicheng47/runner/pull/945)); see the [program record](../impls/836-web-remote-control/README.md).

Linux support ([#823](https://github.com/yicheng47/runner/issues/823)) is active in [PR #982](https://github.com/yicheng47/runner/pull/982); its [brief](../impls/briefs/823-linux.md) remains in this checkout while the implementation and platform docs are on the PR branch.

Future work, including [iOS follow-through](https://github.com/yicheng47/runner/issues/912), [shell/agent separation](https://github.com/yicheng47/runner/issues/795) and [pi MCP management](https://github.com/yicheng47/runner/issues/764), stays in its issue until development starts. Milestones and priorities are listed in the [roadmap](../roadmap.md), not duplicated in this index.

## Archive

The [archive README](archive/README.md) indexes only important features and consequential decisions, with a retention reason for each topic. Completed small-feature and bug-fix specs are deleted under the [archive policy](../README.md#archive-policy); git history keeps their original text. Release contents and dates belong in the [roadmap](../roadmap.md), and validation evidence remains in [tests](../tests/README.md).
