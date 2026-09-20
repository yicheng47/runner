# 648 — General CLI, mission 1 smoke

## Coverage boundary

Mission 1 covers the one-shot command tree over the app socket, the unchanged direct event-log path inside a mission, reference resolution, output modes, exit codes 0–3, caller identity, the five backend additions, and the temporary `runner-mcp` bridge over the shared client. It does not cover feed following or filters, `help agents`, the embedded skill, Settings/install actions, MCP removal, #562 coordinator verbs or seats, or shell completions.

## Automated proof

- The backend registry and the set of tools actually reached by the CLI's exhaustive recorder test compare against `runner_core::RUNNER_TOOL_NAMES`; a registered tool without a command path fails tests.
- CLI unit tests run every remote command against an in-memory recorder and assert its tool sequence and final argument JSON without a socket. They also cover role-handle, exact-name and unique-prefix resolution plus ambiguous name/prefix exit-2 errors, uniform full archived-mission and full mission-session-id pass-through, lexically normalized default/relative paths, empty-string update clears, person-versus-handle identity, own-mission prefix routing, exact JSON/quiet output, every command-aware default view with multiline and nested fixture data, refused-tool decoding, and a shared-client call against an rmcp stub over `tokio::io::duplex`.
- CLI process tests cover unchanged in-mission message/signal/read appends, the removed `status busy|idle` alias, partial and off-bus usage failures, and the exact exit-3 not-running diagnostic. On Unix, the exit-3 tests isolate `HOME` and `XDG_DATA_HOME` from a running development app and cover both a plain shell and the direct-chat `RUNNER_HANDLE`-only environment; Windows uses a fixed pipe name, so that assertion is manual/CI-environment-dependent there.
- Backend tests cover absent `from` as `human`, roster-handle attribution, unknown handles, all four handle-forbidden signal types, mission resume's running-session and concurrent-resume skips plus partial-failure report, direct-start source validation including role-plus-runtime override, direct-chat-only list semantics in the existing repo/ops suite, the registry names, and strict input-property schemas.
- Repository verification is recorded in the mission handoff; no build is pointed at the production app socket.

## Jason's macOS checklist

Run `make run`, then use the debug sidecar at `~/Library/Application Support/com.wycstudios.runner-dev/bin/runner` from a plain Terminal.app shell in a repository:

- [ ] `runner status` shows the CLI version, app version, dev socket, sidecar presence, and outside mode.
- [ ] `runner project list`, `role list`, `crew list`, `mission list`, and `session list` print the curated columns from the spec; multiline values stay on one row, long cells end in `…`, and null values print as `-`.
- [ ] `runner role list --json` prints the tool JSON unchanged.
- [ ] `runner project create cwd-check` and `runner mission start --crew <name> --goal "…" -q` store the repository cwd without a trailing `/.`; `--cwd .` does the same and `--cwd ./x/../y` stores the lexically normalized `y` path.
- [ ] `runner mission show <unique-prefix>` resolves the id prefix and prints key values, sessions, copyable pending question ids, and recent warnings; `runner mission feed` prints the oldest event first and ends with `next_offset`.
- [ ] `runner msg post --mission <id> --to <lead> "…"` appears on the feed from `human`.
- [ ] The same command with `--as <worker-handle>` appears from that handle.
- [ ] The same command with a handle outside the mission roster exits 1 and names the invalid handle.
- [ ] Create an ask card, then `runner mission answer <id> <question_id> <choice>` resolves it.
- [ ] `runner mission stop <id>`, `runner mission resume <id>`, and `runner mission archive <id>` work; `runner mission show <full-id>` still opens the archived mission, `runner msg post --mission <archived-full-id> "…"` reaches the tool and exits 1 with its lifecycle refusal, and `runner mission unarchive <full-id>` restores it. A missing archived prefix exits 2.
- [ ] `runner session list` contains direct chats and no mission sessions.
- [ ] `runner session restart <full-mission-session-id>` passes the id through and restarts that mission slot; a prefix is intentionally insufficient because `session list` excludes mission sessions.
- [ ] `runner chat start --runtime codex -q` starts a role-free direct chat and prints its session id.
- [ ] Quit Runner; `runner status` exits 3 and prints exactly `Runner is not running. Open Runner and retry.` on stderr.
- [ ] Inside a mission PTY, `runner msg post "…"` appends directly and `runner mission show` defaults to that mission.
- [ ] Reconnect an MCP client: it sees `mission_post`, `mission_signal`, `mission_resume`, and `session_list`; it does not see either old `_human_` tool.

## Manual-only boundary

The live dev-app handshake, real socket calls, UI feed attribution, runtime-only PTY spawn, mission-wide resume against live PTYs, and macOS sidecar path are intentionally manual because the crew is not authorized to launch or restart Runner. Windows is compile/CI-covered here; its fixed named pipe prevents deterministic local isolation of the not-running process test, and the later release smoke proves the installed command on JASONPC.
