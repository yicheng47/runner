# 592 — OpenCode smoke checklist

Branch: `feat/592-opencode-runtime`, stacked on #644. Spec: [592](../features/592-opencode-runtime.md). Everything in the spec's v1 is implemented and unit-tested; nothing on this branch ran against a signed-in OpenCode, because a live session uses Jason's providers and writes under `~/.config/opencode` and `~/.local/share/opencode`. The probes ran OpenCode 1.18.30 in a throwaway `HOME`/XDG/`OPENCODE_DB` environment against a local fake model, and the shipped plugin was run there too (below). This file is for Jason to run by hand. Installed CLI at the time of writing: `opencode --version` printed `1.18.30`.

## Coverage boundary

Every OpenCode spawn loads Runner's plugin, `<app data>/opencode/runner-session-key.js`, through `OPENCODE_CONFIG_CONTENT` (appended to the role's or Runner's own value when that is a JSON object), and sets `RUNNER_OPENCODE_REKEY_PATH` to the row's drop file under `<app data>/session-keys/`. On every top-level `session.created` the plugin writes `{"session_id":"ses_…"}` there, and the shared drop watcher rekeys the row. In the throwaway environment the shipped plugin source, under an app-data path containing a space, reported each of two TUIs started 50 ms apart in one folder with its own session, reported a fork's new id, and reported nothing on a plain resume. Unit tests cover the plugin under Node, the drop watcher's OpenCode-only acceptance of `ses_` ids, the environment merge, the spawn and resume argv, and the database check.

A resume passes `--session <key>` when `SELECT 1 FROM session WHERE id = ?` finds the key in OpenCode's database (`OPENCODE_DB`, else `~/.local/share/opencode/opencode.db`), and otherwise starts fresh with the persona as the first turn. Every spawn sets `OPENCODE_DISABLE_AUTOUPDATE=1`. Bypass is `--auto`; Default passes nothing; Accept edits and Auto are not offered. Hook status is out of v1, so OpenCode sessions show the terminal-activity baseline. The provider mark is a placeholder, the generic chat bubble.

## Jason's macOS checklist

1. **Direct chat with a persona.** Open an OpenCode chat with a role that has a persona. Confirm the persona arrives once as the first turn, the pane paints on the alternate screen, and the tab keeps the role's name. For a runtime-only chat, the tab shows OpenCode's session title without its `OC | ` prefix, and nothing while OpenCode shows `OpenCode`.
2. **Key capture.** Within a few seconds of the first turn, confirm the row has a key (`runner` CLI or the DB) equal to the session OpenCode lists in `opencode session list` for that folder.
3. **Blank chat.** Start a runtime-only OpenCode chat and type nothing. Confirm there is no key; send a message and confirm the key appears. Run `/new` in the TUI, send a message, and confirm the key moves to the new session.
4. **Relaunch.** Quit and relaunch Runner. Confirm the chat resumes the same session (`--session <key>`, history visible) and no first turn is replayed.
5. **Missing session.** Stop the chat, delete its session with `opencode session delete <key>`, and resume. Confirm a fresh start with the persona as the first turn and a new key on the row.
6. **Fork.** Fork a keyed OpenCode chat to a new tab. Confirm the fork opens with the source's history under OpenCode's `(fork #1)` title, gets its own key, and the source keeps its key. Fork the fork once and confirm the same.
7. **Model.** Set a role's model to one of your `provider/model` ids: the footer shows it. Leave it on Default: OpenCode uses `model` from `~/.config/opencode/opencode.json`, which the role form shows as the placeholder. The effort picker offers only Default.
8. **Permission modes.** The dropdown offers Default and Bypass only. With a `permission` rule set to `ask` in your config, Default asks and Bypass approves without asking (`--auto`); a rule set to `deny` still denies under Bypass.
9. **Crew.** Start a mission with two OpenCode slots in one worktree, in Bypass mission mode. Confirm each launch prompt lands once, each slot gets its own key (never the other's), and `runner msg read`, `runner msg post` and `runner signal ask_human` work from the bash tool unattended.
10. **Your own config survives.** With your own plugins listed in `opencode.json`, confirm they still load in a Runner-started OpenCode (for example the herdr plugin still reports). Nothing under `~/.config/opencode` should change except MCP entries you copy in check 12.
11. **Settings → Agents and Skills.** The OpenCode row shows the bare `opencode --version`. With an older version installed than npm's `opencode-ai`, the Update button runs `opencode upgrade` in the modal. Settings → Skills lists `~/.config/opencode/skills`, `~/.claude/skills` and `~/.agents/skills` with no toggle, and the Runner skill is present in `~/.agents/skills`.
12. **Settings → MCP.** OpenCode appears as a client reading `~/.config/opencode/opencode.json` (label JSONC). Copy a stdio and an HTTP server from Claude Code: the file gains `{"type":"local","command":[…],"environment":{…}}` and `{"type":"remote","url":…,"headers":{…}}` under `mcp`, and every comment and other key outside those entries is unchanged. `opencode mcp list` shows both. Remove one and confirm the file still parses in OpenCode.
13. **Fixture and wheel.** Record `crates/runner-terminal/fixtures/opencode-first-turn.ndjson` from a live first turn by launching the dev app with `RUNNER_RECORD_INPUT_FIXTURE=<prefix>`, trim it to the first turn and its reply, and bless its snapshot with `UPDATE_SNAPSHOTS=1 cargo test -p runner-terminal`. OpenCode turns on SGR mouse reporting (`1000`/`1002`/`1003`/`1006`), so the wheel over an OpenCode pane should scroll OpenCode's own transcript; confirm it does, and that the prompt line is untouched.

## JASONPC checklist

OpenCode is off by default on Windows; enable it in Settings → Agents first.

1. Confirm how OpenCode is installed (npm puts `opencode.cmd` first; scoop and choco install `opencode.exe`) and that Runner detects it.
2. Run checks 1–6, 8 and 9 above under ConPTY. With the npm shim the first turn is pasted after the TUI is ready rather than passed on `--prompt`.
3. Confirm the plugin loads from its Windows path and reports keys, and that the database check finds sessions in `%USERPROFILE%\.local\share\opencode\opencode.db`.
4. If it all passes, turn `default_enabled` on for Windows in `ops/runtime.rs`, README footnote ⁴ in both languages and `docs/arch/windows.md`.

## Automated verification

Recorded in the pull request with each gate's exit code: `runner-backend`, `runner-app`, `runner-terminal` and `runner-cli` tests at `--profile ci`, workspace Clippy with `-D warnings`, `cargo fmt --all --check` and `git diff --check`.
