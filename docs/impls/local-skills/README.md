# Local skills — program record

Implementation program for [feature 05 — local skills management](../../features/05-runner-skills.md) ([#73](https://github.com/yicheng47/runner/issues/73)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [gpui-rewrite](../gpui-rewrite/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) is the milestone plan, [impl_log.md](impl_log.md) is the dated log. Mission briefs go where every brief lives, [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/), named `local-skills-s1-backend.md` and so on.

## Status (2026-08-28)

Specced, not scheduled. Nothing has landed. The mechanism is verified (claude-code 2.1.250, 2026-08-28): `--settings '{"skillOverrides":{…:"off"}}'` on a headless launch cut the model's own reported skill list from 51 names to 15 with `~/.claude/settings.json` untouched. Design (S0) is the next step and is Jason's call; S1 can start without it.

## Why a program, not one mission

The core is one function on the spawn path, but the feature crosses every layer once: a filesystem catalog with frontmatter parsing and a cached headless probe (backend), a schema column threaded through repo → ops → MCP (backend), a `--settings` merge that changes an existing contract (spawn), a new Settings pane and a new form control with a popup (app), and a design pass first. Four crates' conventions, one required check, and a smoke test that needs two runners and a model call. Sequenced as milestones so each lands green on its own and the app work never blocks on the backend being re-reviewed.

## End state

A runner declares which skills it wants; every session spawned from it — direct chat or mission slot — sees only those plus Claude Code's bundled skills, and nothing on disk changed to make that happen. Settings → Skills shows what each runtime can see and which runners restrict to what. Codex sessions are unchanged and say so.

## Decisions that bind

1. **Visibility, not installation.** Runner never copies, symlinks, installs or edits a skill. The only write is the `--settings` argv at spawn. Consistent with MCP registration (`mcp_servers.runner` only) and feature 65 (config reads).
2. **Allowlist stored, denylist computed.** `runners.skills_json` holds the picks (`NULL` = all). The `"off"` map is `known − picks` computed at spawn from the live filesystem plus the bundled cache, never from the form's last view.
3. **Bundled skills are probed, not hard-coded, and never hidden by accident.** One explicit headless probe from the Skills pane, cached per `claude --version`; without a cache they stay `"on"`.
4. **`--settings` merges; it no longer short-circuits.** A runner whose `args` carry `--settings` gets Runner's keys deep-merged on top, one combined flag emitted, the runner's own flag removed. Unreadable file → Runner's payload alone plus a warning on the runner card.
5. **claude-code only for control.** Codex is catalogued and captioned; the managed-`CODEX_HOME` mirror stays declined (as for feature 52) until codex ships a per-launch control. qoder/trae are not scanned.
6. **Pencil-first for the app milestone.** S2 does not start until the Skills pane card, the list row, the runner-form field, its checklist popup and the chip summary exist in `design/runner.pen`. S1 has no UI and needs no design.
7. **Behaviour-preserving by default.** Every existing runner has `skills_json = NULL`; S1 alone changes nothing visible. The emitted `--settings` for an unrestricted runner is byte-identical to today's.
8. **Plugins are phase 3.** `--plugin-dir` kits are the route for skills outside `~/.claude/skills` only; nothing in S1/S2 touches `~/.claude/plugins`. Per-skill symlinks inside a kit are skipped silently by Claude Code — copy, never link, when that phase comes.

## Landing rule

Each milestone is one `codex peer` mission on a task branch off `main`, working-tree review first, Jason smoke-tests, then PR → the one required check `Rust / macOS` → merge → a `docs(local-skills)` landing commit that moves the milestone from [plan.md](plan.md) into [impl_log.md](impl_log.md) and updates the status above. Migration `0021` is allocated to S1. Standing GPUI rules from the [gpui-rewrite record](../gpui-rewrite/README.md#standing-gpui-rules-every-brief-cites-them) apply to S2; crews do not launch the app.

## History

The Tauri-era attempt was impl [0008 — runner skills and MCPs](../archive/0008-runner-skills-and-mcps.md) against the original spec 05 (per-runner skills injected through a synthetic agent home built from symlink overlays); it was superseded by the 2026-07-15 catalog rewrite of the spec, which in turn was replaced on 2026-08-28 by the visibility design once `skillOverrides` existed. The catalog framing's MCP half is deferred, not dropped — it gets its own spec when felt.
