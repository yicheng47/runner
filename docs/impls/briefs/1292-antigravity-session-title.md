# Mission Brief — #1292 Antigravity CLI Session Title Auto-Rename

- **Issue:** https://github.com/yicheng47/runner/issues/1292
- **Feature spec:** `docs/features/1292-antigravity-session-title.md`
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1292-antigravity-session-title`, branch `fix/1292-antigravity-session-title`, off `origin/main` (`95d74efa`). Keep the root checkout on `main` and work strictly in this worktree (with its own `target/`, never a shared `CARGO_TARGET_DIR`).

## Problem And Settled Decisions

Antigravity CLI (`agy`) tabs stay on `Antigravity CLI` because `agy` leaves its built-in `"title"` hook empty unless `title` is configured in settings, unlike Codex/Claude/Gemini/pi which emit `OSC 2` titles into the PTY.

The design in `docs/features/1292-antigravity-session-title.md` is settled (`needs-design` cleared):

1. Inject `--settings '{"title":{"type":"command","command":"<cmd>","enabled":true}}'` from `Antigravity::launch_args` on fresh, resume, and mission launches, overriding any `"title"` in `~/.gemini/antigravity-cli/settings.json`.
2. Skip `--settings` injection whenever `ctx.role_args` already passes `--settings`, `-settings`, `--settings=...`, or `-settings=...` (because `agy` applies whole-object last-flag-wins across multiple `--settings` flags).
3. Implement a hidden `runner hook title` subcommand invoked via `hook_executable(app_data_dir)` (`<app_data_dir>/bin/runner hook title`). It reads bounded stdin JSON, trims `conversation_title`, writes trimmed non-empty text to stdout (nothing on empty/whitespace/malformed/missing), and always exits `0` cleanly.

## Read First

1. `docs/features/1292-antigravity-session-title.md` — full spec, edge cases, and live QA checks.
2. `crates/runner-daemon/src/runtimes/antigravity/mod.rs`, `tests.rs` — `Antigravity::launch_args`.
3. `crates/runner-cli/src/hook.rs`, `command/cli.rs`, `main.rs` — hidden `hook` subcommand & `session_cli` exemption (`args.get(1) == "hook"`).
4. `crates/runner-daemon/src/runtimes/codex/mod.rs`, `crates/runner-daemon/src/runtimes/claude_code/mod.rs` — existing `#860` title injection, `hook_executable`, `shell_quote`, and `--settings` flag skip pattern.
5. `crates/runner-daemon/src/session/manager/spawn_spec.rs` (especially `command_lines` around `--settings`) & `crates/runner-app/src/surfaces/settings/agents/launch.rs` — Launch Command preview (`Note::NamesChat`).
6. `crates/runner-core/src/protocol/session_title.rs` — `decoration(...)` filter and `provider_title`.

## Deliverables And Boundaries

- Add `Command::Title` (`runner hook title`) in `runner-cli` with bounded stdin JSON reading, whitespace trimming, zero stdout/stderr on missing/whitespace/malformed input, and exit `0`; cover thoroughly with unit and subprocess stdin tests.
- Update `Antigravity::launch_args` to emit `--settings` (built via `serde_json`, quoting `hook_executable(app_data_dir)` via `shell_quote` + `" hook title"` on Unix and `"<exe>" hook title` for `cmd /c` on Windows), skipping injection when `ctx.role_args` already sets `--settings`/`-settings`.
- Update `crates/runner-core/src/protocol/session_title.rs` `decoration(...)` to ignore `"antigravity"`, `"antigravity cli"`, and `"agy"` chrome strings.
- Update `command_lines(...)` in `spawn_spec.rs` so Antigravity's `--settings` injection renders accurately (instead of hitting Claude's `--settings` formatter) and carries `Note::NamesChat`, while preserving Claude's `--settings` preview. Confirm `pty_runtime::format_command_summary` masks `--settings` safely.
- Update affected Antigravity expectations in `spawns.json`, `launch_gate.rs`, `runtimes/antigravity/tests.rs`, `session_title.rs`, and `launch_preview_tests`. Check `docs/guide/reference/supported-agents.md` (+ `zh-CN`) and update together only if needed; check `make guide-cli`.
- Out of scope: merging `--settings` JSON with role args, writing `~/.gemini/antigravity-cli/settings.json`, adding a toggle preference, or `.pen` UI layout edits.

## Validation And Handoffs

1. **Coder:** implement cleanly, pass `make fmt`, workspace Clippy (CI profile + `--features updater`), `cargo nextest run --locked --workspace --cargo-profile ci` (or `make test`), and `make guide-cli` check. Do NOT launch `make run`. Fold this brief (`docs/impls/briefs/1292-antigravity-session-title.md`) into an implementation commit (never leave it as a standalone brief commit in final history) while keeping `docs/features/1292-antigravity-session-title.md`. Hand candidate commit SHA to Reviewer.
2. **Reviewer:** inspect diff and tests against the spec and `AGENTS.md` without launching the app; verify `Note::NamesChat` preview, `role_args` opt-out, `cmd /c` / `sh -c` quoting, and clean commit history. Pass clean candidate commit to `qa` (naming `AGY-TITLE-01`..`AGY-TITLE-03`), and add `reviewed` to the PR once opened/verified.
3. **QA (`qa`):** after clean review, acquire `script/qa-lock acquire --timeout 540`, run `make run` in this worktree, verify `AGY-TITLE-01`, `AGY-TITLE-02`, and `AGY-TITLE-03` from `docs/features/1292-antigravity-session-title.md` with bounded tiny prompts in `/tmp/...` using existing sign-ins, preserving existing dev sessions and archiving only test chats created by QA, capture screenshots, release `script/qa-lock release` after app + dev daemon exit, and commit/record results in `docs/tests/runs/YYYY-MM-DD-mac.md` and the PR description. Report macOS PASS honestly while disclosing that native Windows `cmd /c` runtime execution is only covered by unit tests + Windows CI.

## Authorization

`agy trio` is authorized for issue `#1292`: once clean review and macOS QA pass, fold the brief into the implementation commit, push branch `fix/1292-antigravity-session-title`, open a PR against `main` referencing `#1292` (with label `risk-medium`, plus `needs-live-test` only if a required check remains unverified per `AGENTS.md`), drive GitHub CI green on both platforms, ensure `reviewed` covers the final head commit, and stop at the open PR without merging.
