# Local skills — log

Dated record for the local-skills program ([README](README.md), [plan](plan.md)). Newest entries at the bottom; keep entries short: what landed, deviations, carries, blockers.

## Current state (update with each entry)

- **Landed**: S0 on the canvas and M1 via PR [#531](https://github.com/yicheng47/runner/pull/531) (2026-09-09).
- **Next**: the M2 brief (allowlist in the backend), opening with the `-c skills.config=…` probe that decides codex's per-launch route. Order: M1 pane → M2 allowlist backend → M3 allowlist app.
- **CLI versions verified against**: claude-code 2.1.250 (`skillOverrides` via `--settings` hides skills from the model; leak bugs #54996 / #50631 closed 2026-05-04), codex 0.150.1 (skills present, no per-launch control).

## 2026-08-28 — program opened

Spec 05 rewritten from the MCP + skills catalog framing to visibility-per-runner after verifying `skillOverrides` end-to-end: 51 → 15 skills reported by the model with 34 names `"off"`, `~/.claude/settings.json` byte-identical. Also verified and recorded for phase 3: `--plugin-dir` loads a plugin session-only and namespaced, per-skill symlinks inside it are skipped silently. Listing budget (2.1.129, 1 % of context) noted as the second cause of missed skills. Program directory created with this plan; no code.

## 2026-09-09 — S0 landed, M1 started

Design pass with Jason on the canvas, six frames (ids in [plan.md](plan.md)). Three calls changed the spec: one runtime at a time behind a dropdown instead of stacked cards; two-line rows so the shipped toggle fits (an eye glyph and a hover-only toggle were tried and rejected); the detail is a centered modal whose Edit turns it into a plain-text editor with Save — the first write to a skill file, recorded as decision 1's third write. Spec §Surfaces, plan S0/M1 and README decisions 1 and 6 updated; feature branch `feat/73-skills-pane` cut from `main` (`179c4f9`); the pen file and these docs ride the branch uncommitted until landing. M1 mission `01M22AJERV1ZGP91P1Q21YR52Y` started on `codex peer` from the brief.

## 2026-09-09 — codex correction mid-mission

Jason's smoke test caught the brief's codex half: Codex 0.153.4 reads `~/.agents/skills` (documented user scope, 28 skills here) **and** the legacy `~/.codex/skills` (11 here) — a headless `codex exec "list every skill"` returned all 39 plus 6 system and the plugin skills — and it has a global off switch, `[[skills.config]] path/enabled` in `~/.codex/config.toml`, the setting the Codex app's skill sheet toggles. Spec §Mechanism gained a "What Codex provides" table; the catalog scans both roots into one list; codex rows carry the toggle; `set_global_enabled` writes the TOML array via `toml_edit`; README decisions 1, 3 and 5 amended; `-c skills.config=…` recorded as the M2 question for a codex per-launch route. Canvas: `Settings — Skills · Codex` and the two root-state frames updated. The correction was posted to mission `01M22AJERV1ZGP91P1Q21YR52Y` as a human message.

## 2026-09-09 — M1 landed

Mission `01M22AJERV1ZGP91P1Q21YR52Y` (`codex peer`) ran nine review rounds in one sitting: the pane, modal and editor; a Preview font fix, Cancel returning to Preview and the metadata scrollbar removal steered from the coder's PTY; the codex correction (two roots, `[[skills.config]]`, per-runtime badges) posted mid-mission; one A–Z order across roots; and two round-trip cleanups found by the smoke test — an emptied `[[skills.config]]` no longer leaves a `[skills]` header (Runner writes implicit parents, so a hand-written header with comments is never touched) and an emptied `skillOverrides` map is removed. `make verify` green; PR #531. Deferred, cosmetic: absolute path where the spec writes `~`, the nav icon shared with Diagnostics, modal focus order, 20 px Preview leading from the shared renderer. CLI versions at landing: claude-code 2.1.250, codex 0.153.4.
