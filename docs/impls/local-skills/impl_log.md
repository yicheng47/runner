# Local skills — log

Dated record for the local-skills program ([README](README.md), [plan](plan.md)). Newest entries at the bottom; keep entries short: what landed, deviations, carries, blockers.

## Current state (update with each entry)

- **Landed**: nothing.
- **Next**: S0 — the Skills pane card and row in `design/runner.pen` (Jason); then the M1 brief (pane: catalog per runtime, global on/off for claude-code via `skillOverrides` in `~/.claude/settings.json`). Order reset 2026-08-28 to see-first-control-later: M1 pane → M2 allowlist backend → M3 allowlist app.
- **CLI versions verified against**: claude-code 2.1.250 (`skillOverrides` via `--settings` hides skills from the model; leak bugs #54996 / #50631 closed 2026-05-04), codex 0.150.1 (skills present, no per-launch control).

## 2026-08-28 — program opened

Spec 05 rewritten from the MCP + skills catalog framing to visibility-per-runner after verifying `skillOverrides` end-to-end: 51 → 15 skills reported by the model with 34 names `"off"`, `~/.claude/settings.json` byte-identical. Also verified and recorded for phase 3: `--plugin-dir` loads a plugin session-only and namespaced, per-skill symlinks inside it are skipped silently. Listing budget (2.1.129, 1 % of context) noted as the second cause of missed skills. Program directory created with this plan; no code.
