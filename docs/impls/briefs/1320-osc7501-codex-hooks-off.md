# #1320 — Codex 0.164+ OSC 7501 and hooks-off program status

Jason approved `neon trio` on 2026-11-19. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1320-osc7501-codex-hooks-off` on `fix/1320-osc7501-codex-hooks-off` (base `origin/main` `4c3f06c5`, its own `target/`). The sibling `.worktrees/feat-1213-worktrees-app` belongs to other work; never inspect or edit it.

## Problem and outcome

[#1320](https://github.com/yicheng47/runner/issues/1320) has the full analysis; both gaps are still on `main`:

1. **Codex 0.164+ reports are dropped.** Codex now sends `state=done|error` and `state=blocked:app=codex:kind=permission|question|auth` (source at [rust-v0.170.0 `terminal_program_status.rs`](https://github.com/openai/codex/blob/rust-v0.170.0/codex-rs/tui/src/terminal_program_status.rs): no probe, root record only, no `msg`, ST terminator). Runner's Codex-only parser (`runtimes/codex/program_status.rs`) matches 0.163's exact strings, so every `blocked` with a `kind` is ignored and hooked Codex chats never show **Approval needed** / **Answer needed** or hold crew delivery. `TerminalModel::feed_output` also drops shared program-status reports whenever `codex_native_attention` is set, even when status hooks are off.
2. **Hooks-off chats get no completion badge.** `program_status_authority` is static per adapter (only Antigravity sets it), so managed Claude Code, Codex and pi chats launched with **Status hooks** off never arm or record unread completion from OSC 7501 `done`.

Outcome: Runner reads Codex through the one shared, spec-complete parser (`runner-terminal/src/program_status.rs`, from #861), and any managed agent session that has OSC 7501 and runs without status hooks takes its status, waits and completion badges from it.

## Read first

- Issue #1320; specs `docs/features/archive/861-program-status.md` and `docs/features/archive/1295-antigravity-osc7501.md`; `docs/arch/runtime-integration.md` (program status, Codex input attention and hooks-off paragraphs).
- `crates/runner-daemon/src/runtimes/codex/program_status.rs`, `runtimes/status.rs` (`StatusSemantics::native_attention`, `program_status_authority`), `runtimes/antigravity/mod.rs`.
- `crates/runner-terminal/src/terminal/model.rs` (`feed_output`), `crates/runner-daemon/src/session/manager/{terminal.rs,attach.rs,events.rs,direct_spawn.rs,resume.rs,fork.rs,spawn_spec.rs}` (`codex_native_attention`, `set_program_status_authority`, `status_hooks_enabled`, completion gating).
- `crates/runner-daemon/src/session/state/mod.rs` (`SessionEvent::ProgramStatus`, `SessionEvent::CodexAttention`, `completion_armed`); tests in `session/manager/tests/codex.rs` and `session/state/tests.rs`.

## Deliverables

1. **One parser for Codex.** Retire the bespoke Codex raw-byte parser and feed Codex through the shared parser, including Codex 0.163's unscoped reports. With a live hook bridge, a Codex `blocked` report keeps today's attention-overlay behavior (wait shown, crew delivery held, hook turn state untouched) but now carries `kind` (`permission` → Approval, `question` → Answer, `auth` or absent → Unknown); `working`, `idle`, `done`, `error` or `clear` resolve it, and recognized control bytes never count as output activity. If retiring the parser proves unsafe, extend it to the current contract instead and say why in the review handoff.
2. **Hooks off means OSC 7501 owns status.** For a managed Codex, Claude Code or pi session spawned with status hooks off (chat start, resume and fork), program-status authority is on: Working, Blocked with the mapped wait reason, Done → Completed, Error → Failed and Idle come from OSC 7501, and an unfocused chat records an unread completion on `done`. Agents that never report keep today's terminal-activity fallback. Bare shell tabs and agents started by hand inside them stay quiet (RT-STATUS-14); hooked sessions keep hook precedence (RT-STATUS-15); Antigravity is unchanged.
3. **Docs.** Update `docs/guide/reference/supported-agents.md` and its `zh-CN` page (capabilities table and Codex input-attention text for 0.164+ kinds, done and error), check parity with `python3 script/check-guide-translations.py`, update `docs/arch/runtime-integration.md`, and refresh RT-STATUS-12 in `docs/tests/regression/runtimes.md` from 0.163.0 to the current Codex contract.

Non-goals: making OSC 7501 authoritative while hooks are on, changing the hooks default, Claude Code child-record (`id=`) policy, and OpenCode/Copilot/Grok support; those belong to the follow-up on #1051.

## Validation and live checks

- **Automated:** parser tests for Codex 0.163 and 0.164+ reports (every kind, `done`/`error`, both terminators, split chunks); reducer and manager tests for the hooked Codex overlay with each kind, delivery hold and release, hooks-off authority for Codex/Claude Code/pi (Working, Blocked, Completed badge on `done`, Failed on `error`), shell-tab quiet, and hooked precedence. Existing Codex native-attention tests keep their meaning. Follow the repo's no-race test rules and loop new daemon tests at least 20 times, stating the count. Run `cargo test -p runner-terminal -p runner-daemon`, `cargo test -p runner-app` if app code changes, `make clippy`, `cargo fmt --all --check` and `git diff --check`.
- **Live QA (`@qa` after clean review, macOS, development app):** take the lock with `script/qa-lock acquire --timeout 540`; installed Codex 0.170, Claude Code 2.1.328 and pi 1.5.2; low-cost models, tiny prompts, fresh `/tmp` folders. Back up any development **Status hooks** setting you change and restore it.
  1. RT-STATUS-12 on Codex 0.170, hooks on and off, Bypass off: a command approval shows **Approval needed** and a Plan question **Answer needed** in header and sidebar; resolving clears it.
  2. Hooks off for Codex, Claude Code and pi in turn: Working during a tiny turn; with focus on another chat, the finished turn leaves an unread completion badge.
  3. RT-STATUS-15: a hooked Claude Code chat keeps Working/Idle and its single completion badge.
  4. RT-STATUS-14: Claude Code started by hand in a terminal tab shows status but no unread badge.
  Record results with screenshots in `docs/tests/runs/1320-osc7501-codex-hooks-off-mac.md`.

## Authorization

Approved crew: `neon trio`. Fold this brief into the implementation commit, push the branch and open a PR against `main` with `Fixes #1320`, `risk-medium` (raise it if the change grows) and `reviewed` from `@reviewer` on the final head; `core-review` arrives automatically. Windows native checks are out of scope for this mission; list them as unverified in the PR. Do not merge.

## Phase 1 addition (#1051 — Codex drops status hooks, targeting 0.20.2)

Scope update from Jason (2026-11-19): deliver Phase 1 of [#1051](https://github.com/yicheng47/runner/issues/1051) as a second commit on `fix/1320-osc7501-codex-hooks-off`, reviewed separately by `@reviewer` before live QA runs on the combined two-commit candidate.

1. **Identity hooks only for Codex.** Keep only `SessionStart` and `UserPromptSubmit` (conversation key, transcript path, `turn_id`) and remove `PreToolUse`, `PostToolUse`, `PreCompact`, `PostCompact`, `Stop`, `Interrupt` and `SessionEnd` for chats and missions.
2. **OSC 7501 primary status + transcript outcomes.** Codex status always comes from OSC 7501 through the Commit 1 path (`working` including tools and compaction, `blocked` with `kind`, `done` → Completed with unread badge when unfocused, `error` → Failed, `idle`). **Interrupted** comes from the transcript's `turn_aborted` (`reason == "interrupted"`) for the `turn_id` reported by `UserPromptSubmit`; API-error turns keep the transcript `task_complete` error path. In missions, `UserPromptSubmit` confirms a delivered turn's start, OSC 7501 drives Working and completion, and `blocked` holds crew delivery.
3. **Remove Codex's Status hooks switch.** Drop `RuntimeLaunchOption::StatusHooks` for Codex in **Settings → Agents → Launch** (matching Antigravity); ignore any legacy saved `codex.status_hooks = false` DB row so Codex's identity/transcript watcher remains active.
4. **Raise Codex `min_version` to `0.164.0`.** Update `crates/runner-core/src/protocol/runtime_metadata.rs` so the existing version gate requires Codex 0.164.0+ (the first version with `done`, `error` and `kind`).
5. **Docs and regression updates.** Update `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md`, `docs/guide/guides/troubleshooting.md`, `docs/guide/zh-CN/guides/troubleshooting.md`, `docs/arch/runtime-integration.md`, `docs/arch/arch.md` and `docs/tests/regression/runtimes.md`.
6. **Combined live QA and PR labels.** `@qa` runs the Commit 1 checks (with Codex in its single mode) plus Codex 0.170 tiny turn (Working → unfocused Completed badge), long tool (stays Working), `/compact`, Esc mid-turn (Interrupted), approval (**Approval needed**), Plan question (**Answer needed**), Settings → Agents Codex row without a Status hooks switch, and a two-member mission preserving Codex's documented mid-turn delivery policy: queue one message during first-turn startup or an open approval/question (held, then delivered once that clears; active drafts stay held), and separately send one mid-turn notice to an already Working Codex member (steered in and confirmed exactly once). API-error turns stay automated-only. PR body uses `Fixes #1320` and `Refs #1051`, labels `risk-high`, `reviewed` (on final head), and `needs-live-test` naming the Windows ConPTY Codex OSC 7501 passthrough check (`core-review` arrives automatically). Do not merge.
