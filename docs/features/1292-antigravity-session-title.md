# Antigravity CLI Session Title Auto-Rename (#1292)

Tracking issue: [#1292](https://github.com/yicheng47/runner/issues/1292)

## Problem

Runner's tab and pane live-title pipeline listens to terminal `OSC 2` / `OSC 0` title escapes emitted by the child runtime inside its PTY (`alacritty_terminal` `Event::Title -> provider_title -> Host::set_live_title`). Codex, Claude Code, Gemini CLI, and pi emit meaningful session titles onto `OSC 2`, whereas Antigravity CLI (`agy`) runs a silent no-op for its built-in `"title"` hook unless `title` is explicitly configured in settings. As a result, Antigravity CLI tabs stay on `Antigravity CLI` (the default label) instead of reflecting `agy`'s auto-generated topic or `/rename` changes.

Probing installed `agy` (`1.5.8`) inside a real PTY confirmed the exact runtime behavior:

- `agy` resolves `"title"` command hooks from `~/.gemini/antigravity-cli/settings.json` overridden per top-level key by a CLI `--settings <json>` argument, without modifying `settings.json` on disk.
- Multiple `--settings` flags on `agy`'s CLI are **last-flag-wins across the entire JSON object** (not merged across flags), and `agy` uses Go `flag` parsing supporting `-settings`, `--settings`, `-settings=...`, and `--settings=...`.
- When `"title"` is configured with `{"type":"command","command":"<cmd>","enabled":true}`, `agy` runs `<cmd>` via `sh -c` on Unix and `cmd /c` on Windows with a JSON payload on stdin (`{"cwd":..., "session_id":..., "conversation_id":..., "conversation_title":..., "product":"antigravity", ...}`, omitting `conversation_title` until titled).
- Any trimmed non-empty stdout from `<cmd>` is emitted by `agy` as `\x1b]2;<title>\x07`; empty stdout, missing binary, or non-zero exit produces zero `OSC 2` output and no stderr noise in the PTY. Whitespace-only stdout emits `\x1b]2;   \x07`, so `<cmd>` must trim and suppress whitespace-only output.
- In `agy` `1.5.8`, background first-turn title generation (`cortex.(*CascadeManager).maybeGenerateConversationTitle.func1`) writes the generated title into `~/.gemini/antigravity-cli/conversation_summaries.db` and broadcasts on `SubscribeToSummaries`, but `cli/store`'s `(*Manager).applySubagentSummaries` (`subagent_manager.go:55`) skips `summaryID == currentConversationID` without updating `Store.Conversation.Title` in memory or calling `(*Manager).notify()`. As a result, `agy` `1.5.8` invokes the `--settings` `"title"` hook immediately on `/rename` and on `--conversation` resume (`StreamConversation`), while first-turn automatic titles remain only in `conversation_summaries.db` until `/rename` or resume unless Runner also reads `conversation_summaries.db`.

## Settled Design Decisions

These decisions were confirmed with Jason:

1. **Always override `settings.json` `"title"` via `--settings`.** Runner passes `--settings '{"title":{"type":"command","command":"<cmd>","enabled":true}}'` unconditionally on fresh, resume, and mission Antigravity launches, overriding any `"title"` entry in `~/.gemini/antigravity-cli/settings.json` (matching Codex `#860` / `CODEX_TERMINAL_TITLE`). `preferred_title` already preserves `@handle` for role-backed chats and manual `Rename...` beats `live_title`.
2. **Skip `--settings` injection and summary-DB polling when role args already supply `--settings`.** Because `agy` applies last-flag-wins to the whole `--settings` JSON object, Runner skips both `--settings` injection and `conversation_summaries.db` title polling whenever the role's custom `args` already contain `--settings`, `-settings`, `--settings=...`, or `-settings=...` (matching Claude Code's `inject_claude_settings`).
3. **Hidden `runner hook title` subcommand.** Invoke Runner's own bundled `<app_data_dir>/bin/runner hook title` binary (matching how status hooks use `hook_executable(app_data_dir)`) rather than depending on system `jq`, `python3`, or inline shell scripts.
4. **Read-only `conversation_summaries.db` poll in `agy_capture` for Turn-1 auto-titles (`agy` `1.5.8`).** Alongside `--settings` `"title"`, `agy_capture` performs short-lived strictly read-only SQLite reads (`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` plus `PRAGMA query_only = true`, never creating or mutating `agy`'s database) of `conversation_summaries.title` for the session's active `conversation_id`, filtering through `provider_title` and guarding writes with `expected_row_started_at`.
5. **Rebind reused `TerminalModel` session generation across `session_stop` + `session_resume`.** Stopping and resuming a session in place reuses its `Arc<TerminalModel>` while `resume_in_place` advances `sessions.started_at`; `install_handle` queues a FIFO `SyncSessionTitle` fence on `TerminalModel` so pre-resume `OSC 2` events stay bound to the prior generation while post-resume `OSC 2` events persist under the new `started_at` (and fresh-fallback resumes clear the cached terminal title so identical later titles are not suppressed).
6. **No UI redesign.** Uses Runner's existing live-title pipeline and existing Settings → Agents → Launch → Command preview (`Note::NamesChat` = `"always, names the chat"`), requiring no `.pen` changes.

## Solution Design

### 1. CLI helper: `runner hook title`

- File targets: `crates/runner-cli/src/hook.rs`, `crates/runner-cli/src/main.rs` (verify `session_cli` exemption for `args.get(1) == "hook"` already lets `runner hook title` run directly without session-handoff interference).
- Add `Command::Title` under the existing hidden `runner hook` subcommand (`#[command(hide = true)]` in `crates/runner-cli/src/command/cli.rs`, so public CLI help and `make guide-cli` stay clean).
- `Command::Title` reads stdin up to a bounded limit (reusing `MAX_ENVELOPE_BYTES` or an equivalent constant), parses JSON with `serde_json::from_slice`, looks up `.get("conversation_title").and_then(Value::as_str)`, trims leading/trailing whitespace, and writes the trimmed title to stdout only when non-empty. Always exits `0` on empty stdin, malformed JSON, oversized/unparseable payloads, or missing/null/non-string/whitespace-only `conversation_title`, writing nothing to stdout or stderr so `agy` stays silent and clean.

### 2. Antigravity runtime adapter (`crates/runner-daemon/src/runtimes/antigravity/mod.rs`)

- Extract a helper (similar to Claude Code's flag helper) to detect whether `ctx.role_args` already passes `--settings`, `-settings`, `--settings=...`, or `-settings=...`.
- When no settings flag is present in `ctx.role_args`, emit `--settings` followed by a compact JSON string `{"title":{"type":"command","command":"<cmd>","enabled":true}}` generated via `serde_json` (not hand-rolled string concatenation of JSON values).
- Build `<cmd>` from `hook_executable(ctx.app_data_dir)`:
  - On Unix (`cfg!(not(windows))`): format with `crate::session::launch::shell_quote(&exe)` + `" hook title"` (`'<path>/runner' hook title`).
  - On Windows (`cfg!(windows)`): format cleanly for `cmd /c` as `"<path>\runner.exe" hook title` (single leading+trailing double quotes strictly around the executable path, never surrounding the whole command line, so `cmd /c` handles spaces in `%LOCALAPPDATA%` / `app_data_dir` cleanly without stripping quotes unexpectedly).
- Apply on fresh chats, resumed chats, and mission launches alike (matching `#860`), preserving existing trailing `--session-log`, `--interactive`, permission flags, and status hooks behavior.
- In `agy_capture` (`crates/runner-daemon/src/runtimes/antigravity/agy_capture.rs`), track the active `conversation_id` (seeded from `plan.assigned_key` on resume and updated on each `Streaming conversation <uuid>` log line) and, whenever `inject_antigravity_settings(&role.args)` is true, run short-lived read-only lookups against `~/.gemini/antigravity-cli/conversation_summaries.db` (`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` + `PRAGMA query_only = true`), passing any non-empty title through `provider_title` and routing updates through `SessionManager::apply_provider_title` (`ProxyEvent::ApplySessionTitle` + `repo::session::set_live_title` with `expected_row_started_at`) so both `sessions.live_title` and the attached `TerminalModel` title/metadata cache update together, while deduplicating per `(conversation_id, cleaned_title)` so unchanged summary rows never overwrite a newer `OSC 2` `/rename`.

### 3. Idle brand title filtering & resumed terminal rebinding (`crates/runner-core/src/protocol/session_title.rs`, `crates/runner-terminal`, `crates/runner-daemon/src/session/manager`)

- Add `"antigravity"`, `"antigravity cli"`, and `"agy"` (case-insensitive via existing lowercasing) to `decoration(...)` so idle/startup brand titles emitted by `agy` cannot overwrite a custom or default chat label, while genuine `conversation_title` titles pass through `provider_title`.
- Update `Host` (`session/manager/terminal.rs`), `install_handle` (`session/manager/attach.rs`), and `TerminalModel` (`runner-terminal/src/terminal/model.rs`) so reattaching a retained `TerminalModel` across `session_stop` + `session_resume` flushes any unfinished synchronized-update (`ESC[?2026h`), stacked-title, or partial VTE parser state under the prior generation and enqueues `ProxyEvent::SyncSessionTitle` ahead of new PTY output, updating `Host.started_at` to the resumed row's `started_at` and syncing `TerminalModel.title` to the row's persisted `live_title`.

### 4. Launch preview & command redaction (`crates/runner-daemon/src/session/manager/spawn_spec.rs`)

- Currently `command_lines(...)` matches `if arg == "--settings"` unconditionally and rewrites every `--settings` value into Claude's `{"tui":"fullscreen","theme":"auto","hooks":{…}}` summary with `Note::StatusHooks` / `Note::Identity`.
- Update `command_lines(...)` so Antigravity's `--settings` injection formats accurately (for example compactly displaying the title command payload) and is annotated with `Note::NamesChat` (`"always, names the chat"`, matching Codex's `-c tui.terminal_title=...`), without disturbing Claude Code's `--settings` formatting or note.
- Verify `format_command_summary` in `crates/runner-daemon/src/session/pty_runtime.rs` already masks `--settings` / `--settings=` arguments safely.

### 5. Documentation & goldens

- Update affected Antigravity entries in `crates/runner-daemon/src/session/manager/tests/expectations/spawns.json`, `launch_gate.rs`, and `runtimes/antigravity/tests.rs`.
- Inspect `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md` to confirm whether Antigravity capability descriptions or Launch preview notes mention session titles / `--settings`, and update both languages together if needed. Confirm `make guide-cli` stays clean.

## Out of Scope

- Merging Runner's `--settings` JSON with user-supplied `--settings` JSON or editing `~/.gemini/antigravity-cli/settings.json` on disk.
- Adding a new toggle setting in Preferences to disable Antigravity live titles.
- Any UI layout redesign or `.pen` modifications.

## Validation & Live Test Scope

### Automated Tests

- `crates/runner-cli`: unit and subprocess stdin tests for `runner hook title` covering valid ASCII and CJK/Unicode `conversation_title`, whitespace trimming, whitespace-only/empty/null/missing/non-string `conversation_title`, malformed JSON, empty stdin, and oversized input.
- `crates/runner-core` & `crates/runner-terminal`: `protocol::session_title` tests confirming `"Antigravity"`, `"Antigravity CLI"`, `"agy"` are filtered as decorations while real session titles pass `provider_title`; `TerminalModel` delivery tests confirming `ProxyEvent::SyncSessionTitle` drains unfinished `ESC[?2026h` synchronized updates, stacked titles, and partial VTE sequences under the prior generation before rebinding and resets cached titles on fresh-fallback resumes.
- `crates/runner-daemon`: `runtimes::antigravity::tests` and `agy_capture` tests covering `--settings` injection (fresh, resume, mission, spaces in `app_data_dir`, Windows `cmd /c` formatting helper), skipping `--settings` injection and `conversation_summaries.db` polling when `role_args` contains `--settings`, `-settings`, `--settings=...`, or `-settings=...`, and read-only WAL `conversation_summaries.db` polling with decoration filtering and generation guards; `session::manager::tests` (`archive` and `antigravity`) covering reused-terminal rebind across `session_stop` + `session_resume`, end-to-end Turn-1 summary-title sync, and attached `TerminalMetadata` / `TerminalMirror` / `preferred_title` updates when switching from an OSC-titled conversation to a native-summary-titled conversation; `spawn_spec::launch_preview_tests` confirming Antigravity's `--settings` preview shows accurately with `Note::NamesChat` and Claude's `--settings` preview stays intact; updated `launch_gate` and `spawns.json` goldens.
- Repo checks: `make fmt` / formatting check, `make clippy` (CI profile including `--features updater` on macOS), and full `cargo nextest run --locked --workspace --cargo-profile ci` (plus `make guide-cli` check).

### Live QA Checks (macOS development app)

Selected per `AGENTS.md` Test Scope from `docs/tests/regression/runtimes.md` and feature-specific behaviors changed by the diff (run by `qa` only after clean code review, acquiring `script/qa-lock`, using bounded low-cost prompts in a fresh `/tmp` directory, preserving existing dev sessions):

- **AGY-TITLE-01 — Fresh direct chat auto-rename, `/rename`, & switched conversation summary update:** Create a fresh Antigravity CLI direct chat in `/tmp/...` (with no custom `"title"` in `~/.gemini/antigravity-cli/settings.json`); confirm initial label `Antigravity CLI`, send one tiny prompt that triggers `conversation_title`, confirm pane header and sidebar tab auto-rename cleanly (and `live_title` is populated in `runner-dev session show <id> --json`), issue `/rename <topic>` inside `agy` and confirm the tab/pane updates to `<topic>`, then start a fresh conversation inside `agy` (`/new`) with a tiny topical prompt and confirm the attached pane/sidebar update from `<topic>` to the new conversation's asynchronous summary title. Capture screenshot evidence.
- **AGY-TITLE-02 — Manual rename precedence & stop/resume stability:** Manually rename the direct chat via Runner's UI (`Rename…`), issue another `/rename` inside `agy`, and confirm the manual Runner title wins; also stop/resume (or reload) an Antigravity direct chat and verify the title stays clean without flickering to `"Antigravity"` or `"agy"` brand strings.
- **AGY-TITLE-03 — Settings Launch preview & role `--settings` precedence:** Open Settings → Agents → Antigravity CLI → Launch → Command preview and confirm the Antigravity `--settings` title injection displays accurately with the `always, names the chat` note (not Claude's `{"tui":"fullscreen"...}` preview), and verify an Antigravity role passing `--settings {"toolPermission":"request-review"}` skips Runner's `--settings` injection rather than overriding the role's flags (`RT-START-04` parity).

Platform boundary: native Windows runtime execution of `agy` with `cmd /c` cannot run on macOS `qa` and must be disclosed explicitly in `docs/tests/runs/YYYY-MM-DD-mac.md` and the PR description (`needs-live-test` handled strictly per `AGENTS.md` Test Scope if a required platform check remains unverified).
