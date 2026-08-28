# Local skills — program record

Implementation program for [feature 05 — local skills management](../../features/05-runner-skills.md) ([#73](https://github.com/yicheng47/runner/issues/73)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [gpui-rewrite](../gpui-rewrite/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) is the milestone plan, [impl_log.md](impl_log.md) is the dated log. Mission briefs go where every brief lives, [`docs/impls/archive/gpui-rewrite/briefs/`](../archive/gpui-rewrite/briefs/), named `local-skills-s1-backend.md` and so on.

## Status (2026-08-28)

Specced, not scheduled. Nothing has landed. The mechanism is verified (claude-code 2.1.250, 2026-08-28): `--settings '{"skillOverrides":{…:"off"}}'` on a headless launch cut the model's own reported skill list from 51 names to 15 with `~/.claude/settings.json` untouched. Order decided the same day: **see first, control later** — M1 is the Settings → Skills pane (catalog per runtime, global on/off for claude-code), M2 the allowlist in the backend, M3 the allowlist in the app. Next: S0's pane nodes in `design/runner.pen`, then the M1 brief.

## Why a program, not one mission

The core is one function on the spawn path, but the feature crosses every layer once: a filesystem catalog with frontmatter parsing (backend), a schema column threaded through repo → ops → MCP (backend), a `--settings` merge that changes an existing contract (spawn), a new Settings pane and a new form control with a popup (app), and a design pass first. Four crates' conventions, one required check, and a smoke test that needs two runners and a model call. Sequenced as milestones so each lands green on its own: the pane first because it is useful by itself and settles the catalog's shape against real directories before any behaviour depends on it; then the backend allowlist, invisible until used; then the app control.

## End state

Settings → Skills shows each runtime's global skills and switches claude-code ones on or off globally. A runner declares which of the on ones it wants; every session spawned from it — direct chat or mission slot — sees only those (plus Claude Code's bundled skills and the project's own, which Runner does not touch), and nothing on disk changed to make that happen. The pane also shows which runners restrict to what. Codex sessions are unchanged and say so.

## Decisions that bind

1. **Visibility, not installation.** Runner never copies, symlinks, moves, installs or edits a skill. Its two writes are the `--settings` argv at spawn (per session) and the `skillOverrides` key of `~/.claude/settings.json` (global on/off — the key Claude Code's own `/skills` menu writes), edited as a structured read-modify-write of that one key. Consistent with MCP registration (`mcp_servers.runner` only) and feature 65 (config reads). Codex has no such key, so codex rows are view-only.
2. **Allowlist stored, denylist computed.** `runners.skills_json` holds the picks (`NULL` = all). The `"off"` map is `known − picks` computed at spawn from the live filesystem plus the bundled cache, never from the form's last view.
3. **One global directory per runtime; project and bundled skills are outside the model.** A runner picks from its runtime's global set (`~/.claude/skills`, `~/.codex/skills`) and nothing else is catalogued. Project skills are the project's decision and bundled skills are Claude Code's; both always load. No probe, no version cache, no project picker.
4. **`--settings` merges; it no longer short-circuits.** A runner whose `args` carry `--settings` gets Runner's keys deep-merged on top, one combined flag emitted, the runner's own flag removed. Unreadable file → Runner's payload alone plus a warning on the runner card.
5. **claude-code only for control.** Codex is catalogued and captioned; the managed-`CODEX_HOME` mirror stays declined (as for feature 52) until codex ships a per-launch control. qoder/trae are not scanned.
6. **Pencil-first for the app milestone.** M1 does not start until the Skills pane card and list row exist in `design/runner.pen`; M3 not until the runner-form field, its checklist popup and the chip summary do. M2 has no UI and needs no design.
7. **Behaviour-preserving by default.** Every existing runner has `skills_json = NULL`; M2 alone changes nothing visible. The emitted `--settings` for an unrestricted runner is byte-identical to today's.
8. **Plugins are phase 3.** `--plugin-dir` kits are the route for skills outside `~/.claude/skills` only; nothing in M1–M3 touches `~/.claude/plugins`. Per-skill symlinks inside a kit are skipped silently by Claude Code — copy, never link, when that phase comes.

## Landing rule

Each milestone is one `codex peer` mission on a task branch off `main`, working-tree review first, Jason smoke-tests, then PR → the one required check `Rust / macOS` → merge → a `docs(local-skills)` landing commit that moves the milestone from [plan.md](plan.md) into [impl_log.md](impl_log.md) and updates the status above. Migration `0021` is allocated to M2. Standing GPUI rules from the [gpui-rewrite record](../gpui-rewrite/README.md#standing-gpui-rules-every-brief-cites-them) apply to M1 and M3; crews do not launch the app.

## History

The Tauri-era attempt was impl [0008 — runner skills and MCPs](../archive/0008-runner-skills-and-mcps.md) against the original spec 05 (per-runner skills injected through a synthetic agent home built from symlink overlays); it was superseded by the 2026-07-15 catalog rewrite of the spec, which in turn was replaced on 2026-08-28 by the visibility design once `skillOverrides` existed. The catalog framing's MCP half is deferred, not dropped — it gets its own spec when felt.
