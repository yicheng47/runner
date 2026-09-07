# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Since 2026-09-01 a spec's number **is** its tracking issue number — file the issue first, then name the spec after it — so gaps in the doc sequence belong to bugs and PRs, never to skipped specs. Specs numbered 01–64 predate the alignment and keep their numbers in `archive/` and in the Dropped list; the then-active specs were renumbered to their issues (05→73, 58→393, 61→403). Spec 60 (fork a chat) is mid-mission and keeps its pre-alignment number; it archives as 60.

Since the GPUI rewrite shipped as `v0.6.0` (2026-08-23) there is one line of work on `main`; new features are Pencil-first in `design/runner.pen` and land as nightlies. The post-GA consolidation queue (M6) is [`../impls/gpui-rewrite/m6-remainder.md`](../impls/gpui-rewrite/m6-remainder.md), tracked in [#445](https://github.com/yicheng47/runner/issues/445).

## Index

- [497 — Windows code signing](./497-windows-code-signing.md) — sign the app, both CLI sidecars, the installer, and the uninstaller in both Windows packaging paths through a cloud signing provider (SignPath OSS first, Azure Artifact Signing or a commercial OV certificate otherwise), verify signatures in CI, and retire the "More info → Run anyway" wording; #493 consumes the signature for update verification ([#497](https://github.com/yicheng47/runner/issues/497)).
- [493 — Automatic updates on Windows](./493-windows-auto-update.md) — in-app downloads, verified installer handoff, and user-controlled restart while preserving data and protecting running work; stable and nightly channels stay separate ([#493](https://github.com/yicheng47/runner/issues/493)).
- [494 — macOS header navigation](./494-macos-header-navigation.md) — add Previous page and Next page arrows beside the sidebar toggle, matching Windows icons, tooltips, history navigation, and disabled states while preserving macOS window chrome ([#494](https://github.com/yicheng47/runner/issues/494)).
- [73 — Local skills management](./73-runner-skills.md) — a runner declares which skills it wants and Runner hides the rest at spawn via claude-code's `skillOverrides` riding the existing `--settings` (allowlist computed at spawn, nothing written into `~/.claude`); a read-only Settings → Skills catalog (personal / project / bundled, who-uses-it), a Skills picker on the runner form with a listing-budget hint; claude-code first, codex listed only; rewritten 2026-08-28 from the MCP + skills catalog framing, which is deferred ([#73](https://github.com/yicheng47/runner/issues/73)).
- [52 — Hook-based session status](./52-hook-based-session-status.md) — **record only**: #347 closed won't-do 2026-08-27, and the grid-scraping variant #455 closed 2026-08-28; busy/idle stays on the byte-flow `IdleDetector` (threshold raised to 2 s). As specced: authoritative `working`/`waiting`/`done` status from agent CLI hooks injected per spawn (claude `--settings`, codex hooks.json — never the user's config), with the byte-flow IdleDetector demoted to a universal fallback tier; adds the needs-you attention state.
- [393 — Runner & crew detail redesign](./393-runner-crew-detail-redesign.md) — Pencil-first redesign of both MVP-draft detail pages: crew detail puts the slot roster above the prose config sections instead of below them, runner detail clamps the system-prompt dump and consolidates redundant cards ([#393](https://github.com/yicheng47/runner/issues/393)).
- [403 — Opt-in worktree isolation per mission](./403-mission-worktree-isolation.md) — mission-start decorator: opted-in missions get `git worktree add <repo>/.worktrees/… -b mission/<short-id>-<slug>` and `mission.cwd` points into it, so concurrent crews and the human's checkout never collide; default stays the project checkout, all slots share the tree, archive offers safe (`git worktree remove`, dirty-refusing) cleanup ([#403](https://github.com/yicheng47/runner/issues/403)).
- [468 — Landing page for Runner](./468-landing-page.md) — one dark, responsive page that teaches runner → crew → mission before listing features, following the herdr/onorca category shape; designed first in `design/landing.pen`, hand-written HTML/CSS/JS in `site/`, deployed by a GitHub Pages workflow to `yicheng47.github.io/runner` and then `runner.wycstudios.com` ([#468](https://github.com/yicheng47/runner/issues/468)).
- [491 — Confirm quit while work is still running](./491-confirm-quit-running-work.md) — every quit path (⌘Q / menu, Dock → Quit and logout via a new `applicationShouldTerminate:` hook, last-window close on Windows) first asks the backend what is still running — agents the byte-flow detector reports busy, shell terminals with a foreground process — and, only if there is any, shows a **Quit Runner?** confirm naming them with Cancel / Quit anyway; idle chats never trigger it since auto-resume covers them; the follow-up #466's decision named when it declined the session daemon ([#491](https://github.com/yicheng47/runner/issues/491)).
- [466 — Sessions outlive the app process](./466-sessions-outlive-the-app.md) — **record only**: #466 closed won't-do 2026-09-02; PTYs stay in-process and auto-resume remains the relaunch story (vision §4.2, arch decision 12). As specced: session PTYs move into a small background host so quitting, updating, or crashing Runner.app no longer stops running agents, with reattach on relaunch and a leave-running vs stop-everything quit choice ([#466](https://github.com/yicheng47/runner/issues/466)).

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.

## Dropped

Considered and deliberately not built. Spec kept in-repo as the record.

- [19 — Mission split view](./19-mission-split-view.md)
  — closed as won't-do ([#255](https://github.com/yicheng47/runner/issues/255)):
  crew missions coordinate turn-based, so side-by-side slot PTYs mostly
  show one busy terminal next to an idle one; the feed + per-runner tabs
  cover monitoring, and split view already exists for direct chats.
- [21 — Import native agent sessions into a project](./21-import-native-sessions.md)
  — closed as won't-do ([#176](https://github.com/yicheng47/runner/issues/176)):
  the CLIs' own resume pickers (`claude --resume` / `codex resume` from a
  pane in the project cwd) cover the core need, so the native-store import
  machinery wasn't worth its maintenance surface.
- [24 — Cronjobs](./24-cronjobs.md)
  — closed as won't-do ([#193](https://github.com/yicheng47/runner/issues/193)):
  a resident scheduler (overlap, catch-up, timeouts, wake correctness)
  is always-on machinery inside an app whose identity is a cockpit you
  open to work in, and `mission_start` over MCP/CLI already lets any
  external scheduler fire missions on cron with zero app code. Revisit
  only if the same mission goal keeps getting launched manually on a
  rhythm.
- [53 — Session fork](./53-session-fork.md)
  — closed as won't-do ([#348](https://github.com/yicheng47/runner/issues/348)):
  months of daily use produced zero fork reaches, and the generic
  transcript-handoff tier was exactly the lossy per-runtime capture
  machinery the simplicity budget keeps refusing. The revisit trigger
  fired in 2026-08; the native tier returns, with destinations, as
  spec 60 ([#398](https://github.com/yicheng47/runner/issues/398)).
