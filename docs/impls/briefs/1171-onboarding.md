# Mission Brief: First-run setup (#1171)

**Tracking issue:** [#1171](https://github.com/yicheng47/runner/issues/1171) | **Existing Draft PR:** [#1335](https://github.com/yicheng47/runner/pull/1335)
**Branch:** `feat/1171-onboarding` | **Base:** `origin/main` (`7f42a9b4`, branch HEAD `85d5cf14`)
**Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1171-onboarding`
**Crew:** `hydrogen trio` (`coder`, `reviewer`, `qa`)

**Parallel isolation:** Sibling worktrees `.worktrees/fix-1251-opencode-v2` and `.worktrees/fix-1201-starting-pill` are active in parallel. Stay strictly in this worktree, never read/edit peer worktrees or assume unmerged changes landed, avoid running workspace-wide `cargo test` concurrently with siblings, and coordinate live QA through `script/qa-lock acquire --timeout 540`.

## Authoritative Spec & Design

- **Behavior spec:** `docs/features/1171-onboarding.md` (already on this branch).
- **Signed-off Pencil design:** `/Users/jason/repos/yeelinks/yeelinks_design/runner/desktop/specs/1171-onboarding.pen` (`9102453`) — frames `WBlrk`, `e839DM`, `t0ki9c`, `SoC3f`, `GVeqR`, `I0YpU`. Frames win on layout/copy; spec wins on behavior. Keep the spec/README/brief clean and fold this brief commit into implementation commits before pushing.

## Read First

1. `crates/runner-daemon/src/db/seed.rs` and `crates/runner-daemon/src/db/app_state.rs` (`SEED_MARKER_KEY`, `SEED_CODER_PROMPT`, `SEED_REVIEWER_PROMPT`, `SEED_CREW_ADDENDUM`, `_app_state`).
2. `crates/runner-daemon/src/ops/runtime_tasks.rs` and `crates/runner-core/src/protocol/runtime.rs` (`RuntimeUpdateTask`, `runtime_task_start`, `runtime/task-changed`, #1165 PTY task monitor).
3. `crates/runner-daemon/src/ops/runtime.rs` & `crates/runner-daemon/src/runtime_status/versions.rs` (catalog order, `npm_package`, login-shell `PATH`, synchronous/recheck probes, existing sign-in/credential detection).
4. `crates/runner-core/src/protocol/api.rs` & `crates/runner-app/src/surfaces/start_chat/launcher.rs` / `render.rs` (launcher surface beside sidebar, Mission entry, prompt prefill).
5. `crates/runner-app/src/surfaces/settings/agents/{mod.rs,presentation.rs,render.rs}` & `crates/runner-app/src/surfaces/background_tasks.rs` (Open setup, Not installed `Install` action, pill/task list).

## Deliverables & Settled Defaults

1. **No seeded crew + `onboarding_completed` (`_app_state` + migration `0031`):**
   - `seed_defaults` stops inserting `Pair coding crew` while keeping `default_crew_seeded` idempotent.
   - Add migration `0031` (+ update `migrations/schema.sql`) setting `onboarding_completed` for existing DBs that have any sessions, missions, or non-seed roles/crews; leave unset on fresh DBs or seed-only DBs.
   - Expose read/set/proposal/starter-crew/install protocol methods in `runnerd`. Any chat or mission start (UI, CLI, remote) marks `onboarding_completed`.
2. **Sign-in status, npm install tasks & `Settings → Agents` parity:**
   - Reuse existing read-only credential/account checks where reliable without launching interactive agents (Claude Code accounts; read-only auth/token checks for Codex/Antigravity if already safe in `usage.rs`); other installed runtimes show `"Installed"` without blocking eligibility or claiming sign-in.
   - Extend `RuntimeUpdateTask` with `kind` (`update` vs `install`, empty `from_version` on install). `npm install -g <npm_package>` runs in a home-folder login-shell PTY when `npm` is on `PATH`; no `Reinstall` check on stopped installs; show `Node.js` link when `npm` is missing and Copy + Guide when `npm_package` is `None`. Add `Install` + inline spinner to `Settings → Agents` Not installed rows and `Open setup` in Settings header (`GVeqR`).
3. **First-run setup surface & Starter crew (`WBlrk`, `e839DM`, `t0ki9c`, `SoC3f`, `I0YpU`):**
   - Replace launcher beside sidebar until `onboarding_completed` is set (and on `Open setup` per window without clearing the key). Show top-4 agents + overflow disclosure, Recheck, Starter crew card (`Will be created` / `Don't create` / `Won't be created` / `Create it` / `Needs an agent`, favoring default + signed-in + two distinct runtimes), and footer (`Skip setup`, `Start your first chat` prefilling a safe read-only sample prompt naming/summarizing the launcher's folder).
   - With zero crews, launcher's Mission entry shows `"No crews yet"` with **Create a crew** + **Open setup**, and `runner mission start` without `--crew` explains how to create one. Keep `en.toml` + `zh-hans.toml` i18n, guide pages (`docs/guide/` + `docs/guide/zh-CN/`), and `make guide-cli` (if CLI help changed) updated.

## Validation & PR Authorization

- **Automated:** Cover all unit/migration/protocol/UI items in `1171-onboarding.md` Verification; run `cargo test` per touched crate (`runner-core`, `runner-daemon`, `runner-app`, `runner-cli`), both `clippy --profile ci` passes (`--features updater`), `cargo fmt --all -- --check`, `git diff --check`, and `python3 script/check-guide-translations.py`.
- **Live QA (`qa` slot):** Follow `1171-onboarding.md` Live checks 1–7 in macOS dev app under `script/qa-lock`, using a **fresh temporary dev DB/data dir or backed-up clean state without touching existing user chats/sessions**, and a scratch npm prefix so real agent installs are untouched. Record results in `docs/tests/runs/1171-onboarding-mac.md` and PR `#1335`. Note OS TCC RemovableVolumes behavior if testing folders under `/Volumes/samsung` vs `/tmp`.
- **PR mode:** Authorized by Jason on 2026-11-15 to fold this brief into implementation commits, push to `feat/1171-onboarding` (updating Draft PR `#1335`), drive CI green, mark PR `#1335` ready for review once `reviewer` adds `reviewed` on head SHA and macOS QA passes (keep `needs-live-test` if Windows/Linux live checks remain unrun), and leave merge to Jason (`core-review` applies).
