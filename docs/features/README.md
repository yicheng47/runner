# Feature specs

Active feature specs, one per open tracking issue. A spec moves to [`archive/`](./archive/) once its issue closes, shipped or dropped: the implementation is the source of truth and the archived spec is the "what we were going for" record. Priorities and milestones live on the issues; [`../roadmap.md`](../roadmap.md) mirrors them.

Since 2026-09-01 a spec's number **is** its tracking issue number: file the issue first, then name the spec after it, so gaps in the sequence belong to bugs and PRs. Specs 01–64 predate the alignment and keep their numbers in `archive/`; the then-active ones were renumbered to their issues (05→73, 58→393, 61→403), and 60 (fork a chat) kept its number and shipped 2026-09-01.

## Active

- [562 — Missions as containers](./562-mission-spawn.md) — the mission owns its roster, a mission starts from a crew or a role, and the lead or an outside seat spawns, lists, waits on and stops slots ([#562](https://github.com/yicheng47/runner/issues/562), P3 on the issue, 0.12; plan under review).
- [403 — Opt-in worktree isolation per mission](./403-mission-worktree-isolation.md) — a mission runs in its own `git worktree` so crews and the human's checkout never collide ([#403](https://github.com/yicheng47/runner/issues/403), P1, 0.14).
- [393 — Role and crew detail redesign](./393-runner-crew-detail-redesign.md) — Pencil-first redesign of both detail pages ([#393](https://github.com/yicheng47/runner/issues/393), P1, 0.12 release blocker).
- [559 — Command palette on ⌘⇧P](./559-command-palette.md) — the ⌘K overlay in command mode over the keymap ([#559](https://github.com/yicheng47/runner/issues/559), P2, 0.12).
- [565 — i18n, 简体中文 first](./565-i18n.md) — a language setting, a compile-time catalog, a live switch ([#565](https://github.com/yicheng47/runner/issues/565), P2, 0.14).
- [586 — Shell status: process detection first](./586-shell-status-detection.md) — foreground-process detection for shell panes before semantic shell integration ([#586](https://github.com/yicheng47/runner/issues/586), P2, 0.13).
- [533 — Update agent CLIs from Settings → Agents](./533-agent-cli-updates.md) — installed versions, an Update button that runs the CLI's own updater in a pane, an update-available badge ([#533](https://github.com/yicheng47/runner/issues/533), P2, 0.11.x).
- [468 — Landing page](./468-landing-page.md) — one dark responsive page in `site/`, deployed by GitHub Pages ([#468](https://github.com/yicheng47/runner/issues/468), P2, 0.12).

## Dropped

Considered and deliberately not built; the spec stays in `archive/` as the record.

- [19 — Mission split view](./archive/19-mission-split-view.md) ([#255](https://github.com/yicheng47/runner/issues/255)): missions coordinate turn-based, so side-by-side slot PTYs mostly show one busy terminal next to an idle one.
- [21 — Import native agent sessions](./archive/21-import-native-sessions.md) ([#176](https://github.com/yicheng47/runner/issues/176)): the CLIs' own resume pickers from a pane cover the need.
- [24 — Cronjobs](./archive/24-cronjobs.md) ([#193](https://github.com/yicheng47/runner/issues/193)): a resident scheduler is always-on machinery inside a cockpit; `mission start` from an external scheduler covers it.
- [53 — Session fork](./archive/53-session-fork.md) ([#348](https://github.com/yicheng47/runner/issues/348)): zero reaches in months of use; the native tier returned as spec 60 ([#398](https://github.com/yicheng47/runner/issues/398)).
- [466 — Sessions outlive the app](./archive/466-sessions-outlive-the-app.md) ([#466](https://github.com/yicheng47/runner/issues/466), 2026-09-02): PTYs stay in-process; the direction returned as the session host ([#645](https://github.com/yicheng47/runner/issues/645), 0.13).
- [491 — Confirm quit while work is running](./archive/491-confirm-quit-running-work.md) ([#491](https://github.com/yicheng47/runner/issues/491), 2026-09-07): a stopgap the session host makes obsolete.
- [511 — Ask about a selection](./archive/511-ask-about-selection.md) ([#511](https://github.com/yicheng47/runner/issues/511), 2026-09-10): no side thread forked from a selection.
- [557 — Translucent window backdrop](./archive/557-window-backdrop.md) ([#557](https://github.com/yicheng47/runner/issues/557), 2026-09-11): not important enough to carry.
