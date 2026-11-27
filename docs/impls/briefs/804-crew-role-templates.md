# Brief: #804 — Crew and role template import and export (Stage 1)

## Starting point

- **Issue / PR:** [#804](https://github.com/yicheng47/runner/issues/804) (folds #1334, supersedes #1002); existing draft PR [#1474](https://github.com/yicheng47/runner/pull/1474) on `feat/804-crew-role-templates`.
- **Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/feat-804-crew-role-templates` (keep root on `main`; use this worktree's own `target/`, never share `CARGO_TARGET_DIR`).
- **Base revision:** Rebase `feat/804-crew-role-templates` onto `origin/main` (`c972f67a`) before coding, preserving `docs/features/804-crew-role-templates.md` and its `docs/features/README.md` entry cleanly.
- **Sibling worktrees:** `.worktrees/feat-1489-kitty-graphics-protocol` is a separate checkout — do not inspect, edit, or assume anything from it.

## Problem and outcome

Deliver offline `.crew.toml` / `.role.toml` template export and import across `runner-core`, `runner-daemon`, `runner-cli`, and `runner-app`, plus converting `examples/` into `.crew.toml` files.

- **Authoritative spec:** `docs/features/804-crew-role-templates.md` (signed off 2026-11-27; wins on behavior, TOML contract, protocol, CLI, and test scope).
- **Authoritative design:** `runner/desktop/specs/804-crew-role-templates.pen` in `yeelinks_design`, revision `cd7c9f6` (wins on visual hierarchy, layout, copy, and states across Boards `KUm7t`, `vIBqb`, `g8ycMu`).

## Read first

- `docs/features/804-crew-role-templates.md` — full signed-off spec.
- `crates/runner-core/src/protocol/{model.rs,api.rs}` — `Crew`, `Role`, `Slot`, and client protocol pattern.
- `crates/runner-daemon/src/ops/{crew.rs,role.rs,slot.rs,onboarding.rs,runtime.rs}` — immediate transactions, change events, handle checks, and installed built-in runtime detection.
- `crates/runner-cli/src/command/{crew.rs,role.rs}` — crew/role CLI subcommands and JSON output.
- `crates/runner-app/src/surfaces/{crews/,roles/}` and `crates/runner-app/src/ui/file_drop.rs` — list/detail pages, menus, `ExternalPaths` drop highlight, and `Select` / modal patterns.
- `examples/` and `docs/guide/{guides/build-a-crew.md,zh-CN/guides/build-a-crew.md}`.

## Deliverables and boundaries

1. **TOML v1 contract (`runner-core`):** New `crates/runner-core/src/template/` using `toml_edit`; move `MAX_SYSTEM_PROMPT_BYTES` (16 KiB) into `runner-core`. Shared `RoleTemplate` schema for `[role]` and `[[roles]]`; `version = 1`, `kind = "crew" | "role"`, optional `[meta]`, `[crew]`, ordered `[[slots]]`. Reject unknown keys everywhere except `[meta]`, reject excluded fields (`id`, timestamps, `working_dir`, `env`, `command`, `args`, `lead`, `permission_mode`), reject non-v1, `shell`/unknown runtimes, invalid `codex_speed`, files > 256 KiB, and prompts > 16 KiB. Export self-validates and emits readable multiline TOML.
2. **Daemon protocol & atomic import (`runner-daemon`, `core-review`):** Add `template_export_crew`, `template_export_role`, `template_preview`, and `template_import` (zero network code). Always recreate imported roles fresh (no conflict/reuse detection); auto-suffix colliding handles (`-2`, `-3`, … within 32 chars) and crew names (`(2)`). Check each slot's effective runtime (`slot.runtime` or `role.runtime`) and standalone role's `runtime` against installed built-in runtimes: preserve overrides when installed; when switched to another runtime, clear `model`, `effort`, and `codex_speed` on that slot/role to defaults (shared roles with differing slot runtimes put the first chosen runtime on the role and `runtime_override` on differing slots). Run `template_import` in one immediate SQLite transaction and emit `role/changed`, `crew/changed`, and `slot/changed` after commit.
3. **CLI & reference (`runner-cli`):** `runner crew export/import` (`-o`, `--name`, `--map-runtime <from>=<to>`, `--dry-run`, `--json`) and `runner role export/import` (`-o`, `--handle`, `--runtime`, `--dry-run`, `--json`). Regenerate CLI docs via `make guide-cli`.
4. **Desktop UI & guides (`runner-app`, `docs/guide/`):**
   - Entry points (`KUm7t`): **Import…** on Crews/Roles headers and Crews empty state; **Browse sample crews ↗** (`https://runnersh.dev/crews`) in Crews subtitle, empty state, and Step 1 modal; **Export…** in row `⋯` menus and Crew/Role detail page `⋯` menus. All hidden on remote hosts. Export uses `cx.prompt_for_new_path` + toast.
   - Centered 3-step modal (`vIBqb`, `g8ycMu`): Step 1 drag-and-drop (`ExternalPaths`) + **Choose file…** (`cx.prompt_for_paths`) + `runnersh.dev/crews` link; Step 2 Preview with **Change file**, editable crew name / role name + handle, fresh-role badges, missing-runtime picker + bulk switch banner (2+ slots sharing a missing runtime) + `Keep <runtime> (not installed)` option, Team conventions row, broad-access note; Step 3 Prompt Reader (Source default with line numbers & visible invisible/bidi markers + Compact Rendered toggle, Previous/Next/Back to import, Esc).
   - Recoverable validation error with filename/line, drop retry hint, and **Choose another file…**. Post-import toast, list refresh, and navigation to the new Crew/Role page. Full Dark + Runner Light themes, `en.toml` + `zh-hans.toml` i18n, and EN + `zh-CN` guide pages (`concepts/crews.md`, `concepts/roles.md`, `guides/build-a-crew.md`, `reference/cli.md`).
5. **Repo examples (`examples/`):** Convert all 7 folders (`codex-trio`, `dev-crew`, `docs-crew`, `pair-coding`, `tic-tac-toe`, `tomb-raid`, `werewolf`) into `examples/<slug>/<slug>.crew.toml` (removing redundant per-role `.md` and `team-conventions.md`, keeping `goal.md`), add a `runner-core` validation test for every `examples/**/*.crew.toml`, and update `build-a-crew.md` links (EN + `zh-CN`).
6. **Non-goals:** No `runner://` links, no URL/HTTP fetching, no role conflict/reuse detection, no permission modes in templates, no remote-host import/export, no auto-starting missions.

## Validation and handoffs

- **`@coder`:** Implement in clean layered commits (folding this brief out of standalone history before final push). Run the spec's automated suites (`runner-core`, `runner-daemon`, `runner-cli`, `runner-app` with `h.wait(…)` and `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'` for async UI tests), `make guide-cli`, and `make verify`. Do not run the live dev app.
- **`@reviewer`:** Review code and tests only (do not run the app). Check TOML contract strictness, protocol/transaction safety, test race-freedom, and design alignment against `cd7c9f6`. Apply `reviewed` on the clean head SHA and confirm the live QA checks for `@qa`.
- **`@qa`:** After clean review, acquire `script/qa-lock acquire --timeout 540`, run the 5 feature live checks + `MIS-CREW-01` and `CLI-READ-01` from the spec in the dev app using disposable crews/roles (preserving pre-existing user sessions untouched), archive only QA-created sessions/missions, release `qa-lock`, record results in `docs/tests/runs/804-crew-role-templates-mac.md` and PR `#1474`, and remove `needs-live-test` once all checks pass.

## Authorization

- Approved crew: **`neon trio`** (`coder`, `reviewer`, `qa`; 3 agents), authorized by Jason on 2026-11-27.
- Authorized to rebase `feat/804-crew-role-templates` onto `origin/main`, commit, push (including `--force-with-lease` after rebase/fixups), mark PR `#1474` ready for review (`Fixes #804`) with `risk-medium` (keeping `core-review`), manage `reviewed` / `needs-live-test`, and drive CI green. Do not merge; stop at an open, reviewed, QA-verified, CI-green PR for Jason's `core-review` and final merge.
