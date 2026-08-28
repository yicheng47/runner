# Runtime default model and effort

Tracking issue: [#380](https://github.com/yicheng47/runner/issues/380). Status: shipped on `main` — PR #456 → `491307b`, 2026-08-28, by mission on `codex peer` (`01M136PE1BBGJTBP4DQMNN5N1B`); the smoke test hid the defaults caption on Not-found rows. Ships in the first release after v0.6.6. Picked up on the native GPUI line; the issue's `src-tauri/` pointers map to `crates/runner-backend/src/runtime_status.rs`, `crates/runner-backend/src/ops/mcp.rs`, and `crates/runner-backend/src/router/runtime.rs`.

## Motivation

A runner with no model/effort override runs on whatever the CLI's own config says, and Runner never shows what that is. Settings → Agents lists each runtime's executable, but "default" is opaque: nothing in the app says which model a bare `codex` or `claude` spawn will use, or at what effort. Surfacing it makes the runner form honest (blank means *this*) and shortens the "why is this crew slow/dumb" debug loop.

## Scope

Read-only. Runner reads each runtime's own config on demand and never writes it (the MCP registration posture — only `mcp_servers.runner` is ever touched — stands).

Config sources, one reader per runtime, absent file or key → unknown, malformed file → unknown (never an error):

| runtime | file | model key | effort key |
|---|---|---|---|
| codex | `~/.codex/config.toml` | `model` | `model_reasoning_effort` |
| trae | `~/.trae/traecli.toml` | `model` | `model_reasoning_effort` |
| claude-code | `~/.claude/settings.json` | `model` | `effortLevel` |
| qoder | `~/.qoder/settings.json` | `model` | `effortLevel` if present |

Codex and TRAE honor a top-level `profile = "<name>"`: keys under `[profiles.<name>]` win over the top-level keys, matching the CLI's own precedence. Values are shown verbatim (`claude-fable-5[1m]` stays as written).

- **Backend** — `RuntimeExecutableStatus` and `RuntimeCatalogEntry` each gain `default_model: Option<String>` and `default_effort: Option<String>`, resolved inside `status_list` so every consumer of the status or catalog sees them. Reads happen per call (Settings open, refresh, form open); no file watching.
- **Settings → Agents** — each runtime row gains a caption line under the executable field, in the caption's existing mono/faint styling: `Model: gpt-5.6-sol · Effort: xhigh`. A missing half reads `runtime default` (`Model: claude-fable-5[1m] · Effort: runtime default`). Rows in the Not found state omit the caption.
- **Runner create/edit** — the model field's placeholder becomes `default (gpt-5.6-sol)` when the selected runtime has a known default, `default` otherwise, and follows the runtime select. The effort select's empty option reads `Runtime default (xhigh)` / `Runtime default` the same way.

## Non-Goals

- Environment-variable overrides (`ANTHROPIC_MODEL`, `CODEX_MODEL`, …) and project-level config files — home config only.
- Watching the config files; a reopen or refresh is the update path.
- The crew slot-override drawer and Start Chat model fields (`crews.rs`, `start_chat.rs`) — their "blank inherits runner default" copy already names the runner's own value; runtime resolution there is a follow-up if it earns it.
- Showing the resolved effective model on Runners list cards (left open in #380).
- A Pencil pass: this adds text to existing rows and placeholders with no new layout, so `design/runner.pen` is unchanged.

## Verification

- Backend unit tests over temp config files: codex top-level keys; codex `profile` precedence; missing file; malformed TOML/JSON; claude `model` + `effortLevel`; qoder `model` only.
- `runner-app` tests: the caption line for known / half-known / unknown defaults, hidden for Not found; placeholder and empty-option labels with and without a default.
- Manual: Settings → Agents shows `Model: gpt-5.6-sol · Effort: xhigh` for Codex and `Model: claude-fable-5[1m] · Effort: xhigh` for Claude Code on Jason's machine; TRAE and Qoder (not installed) omit the defaults caption; the runner form's model placeholder and effort default option track the Agent select.
