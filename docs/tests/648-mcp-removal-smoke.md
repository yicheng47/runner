# 648 — MCP removal smoke

## Coverage boundary

Mission 3 removes Runner's agent-facing MCP registration and `runner-mcp` bridge without changing the app-owned socket server, its tool registry, or `runner call`. Automated tests use only temporary home and app-data directories. They do not read or write any real agent config or either installed Runner data directory.

The live development check below deliberately proves that the development app leaves production registrations alone. Removal of a production registration by the production app belongs to the first nightly carrying this change, because a development build's bridge path must not match it.

## Automated proof

- The once-only upgrade step removes a registered `runner` entry only for a client present in `initializedMcpClients` and only when the command is this app's `<app data>/bin/runner-mcp` path with no arguments.
- A registration pointing elsewhere and a client absent from the initialized set remain byte-for-byte unchanged as final skips; the report names both configs.
- An unparseable config remains byte-for-byte unchanged for that launch, is logged as deferred, leaves `mcpRegistrationsRemoved` false, and is retried on a later pass. A second-read race and a write failure follow the same pending-retry rule.
- A successful pass persists `mcpRegistrationsRemoved`; a reloaded settings file invokes no client operation.
- A settings file containing only `initializedMcpClients` still decodes, and a fresh store over an injected temporary home changes none of the four config canaries.
- The MCP catalog does not seed a Runner row. A configured server named `runner` appears in the ordinary sorted catalog and uses the same toggle, editor, transport badge, and detail modal as any other server. The pane caption no longer mentions Runner's own server.
- Startup deletes a stale bridge from a temporary app-data directory and succeeds when the bridge is absent.
- `cargo build -p runner-cli --bins` produces only `runner-agent-cli`; the removed bridge target and Copilot discovery shim are not compiled or packaged.

## Jason's `make run` checklist

1. Before launch, confirm the `runner` entries in `~/.claude.json`, `~/.codex/config.toml`, and `~/.copilot/mcp-config.json` point at the production bridge.
2. Run `make run` once.
3. Confirm all three production config files are byte-identical to their pre-launch copies.
4. Confirm the development log names each config as left unchanged because its command points to another Runner installation, or because that client was not initialized by the development Runner.
5. Confirm `~/Library/Application Support/com.wycstudios.runner-dev/bin/runner-mcp` is gone.
6. Open Settings → MCP and confirm there is no pinned Runner row. Confirm the production `runner` registration appears as an ordinary server row, points at the production path, and can open the normal detail editor.
7. Confirm `runner-dev mission start` still works through the app-owned socket.

On the first nightly carrying this change, start a Claude Code session before launching the new app, then launch the nightly. Confirm a production `runner` entry pointing at the production bridge is removed once while the already-started Claude Code session keeps its bridge process until that session ends. Relaunch and confirm no second removal pass runs.

## Results

**`make run`, 2026-09-20, on the PR branch.** Items 1 to 7 passed. The Claude Code and Codex entries, which pointed at the production bridge, were left byte-identical; the Codex file had not been touched since 2026-09-18, and the `runner` entry in `~/.claude.json` was intact although Claude Code itself rewrites that file continually. The Copilot entry was removed and its file rewritten to an empty `mcpServers`: the dev app had registered Copilot on 2026-09-16 while #540 was built, a day before 0.10.0 shipped the runtime, so the entry pointed at the dev bridge and production never overwrote it. The stale dev bridge was gone from `bin/`, `mcpRegistrationsRemoved` was recorded with nothing deferred, and a throwaway mission on a codex crew started, spawned its slot, stopped and archived through `runner-dev`, printing `STATUS stopped` on stop.

**First nightly, `a61af39`, 2026-09-20.** Installed over `5b6030a` with ten Claude Code bridge processes alive. On first launch the `runner` entries pointing at the production bridge were removed from `~/.claude.json` (the `quill` and `pencil` servers untouched) and `~/.codex/config.toml`; Copilot's file was already empty; `mcpRegistrationsRemoved` was recorded; the bridge was gone from app data, which held only `runner`; and the ten bridge processes kept running for their sessions. The same launch linked `~/.local/bin/runner` with no click and installed the skill into all three roots, TRAE's by detection. The relaunch check for a second removal pass was not run; the record's early return is unit-tested.

## Release note

Runner 0.11.0 replaces the agent MCP bridge with the `runner` CLI and removes registrations created by this Runner installation on first launch. A hand-written `runner` entry is left unchanged; if it still points at the removed bridge, the agent will show it as a failed server until the user removes or updates that entry.
