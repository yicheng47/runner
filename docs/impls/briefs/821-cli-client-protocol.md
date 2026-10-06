# 821 — The runner CLI moves from MCP to the runnerd client protocol

Implement [#821](https://github.com/yicheng47/runner/issues/821). Jason requested this mission on 2026-10-07. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-821-cli-client-protocol`, on the existing branch `refactor/821-cli-client-protocol`, created from `origin/main` at `872a4cb5` (0.13.0). The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. No other crew is running.

## Why

Since 0.13.0, `runnerd` serves two protocols over one core: MCP on `mcp.sock` for the `runner` CLI, and the client protocol on `runnerd.sock` for the app. The CLI is MCP's only client, so every operation is registered twice, and MCP lacks what comes next: pushed events, the caller's identity on the connection, and a byte stream that #808's phase 3 can carry over ssh. This mission leaves one protocol.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #821, and `docs/impls/645-runnerd/plan.md` from "After phase 1 — the CLI moves to the client protocol" (about line 326). `docs/features/645-session-host.md`, "The client protocol" and "What does not change".
- `crates/runner-cli/src/client.rs`: `SocketClient` over rmcp, `connect`/`connect_or_start` (start the daemon when not running, refuse over ssh), `connect_failure` (about line 52, the exit 3 versus exit 5 rule).
- `crates/runner-cli/src/command.rs`: the `ToolCaller` seam (about line 515) and `call` (about 2245), `run_connected` (about 945), `daemon_command` (about 852, already on `SocketTransport`), the `status` JSON (about 1000), `follow_feed`/`follow_call` (about 1661) with the poll constants at the top, the recorder test that checks `RUNNER_TOOL_NAMES` (about 3840) and `every_command_leaf_parses_without_a_socket`. Also `output.rs` and `help.rs`.
- `crates/runner-daemon/src/mcp/` (server, listener, `tools/*.rs`): the 46 tools, their argument parsing and the daemon-side caller-handle checks. `runner_core::RUNNER_TOOL_NAMES` in `crates/runner-core/src/lib.rs`.
- `crates/runner-core/src/protocol/{api.rs,client.rs,socket.rs,wire.rs}` and `crates/runner-daemon/src/daemon/{mod.rs,server.rs}`: the request table, `dispatch`, the per-connection loop, and the endpoint ownership check (about server.rs:199).
- `crates/runner-core/src/{daemon_process.rs,app_paths.rs}`, `crates/runner-cli/tests/{daemon_process.rs,roundtrip.rs}` (how tests run an isolated daemon), and `crates/runner-core/src/protocol/agent_skill.rs` (the shipped agent skill text).

## What must not change

- The CLI's output, including every `--json` shape, stays byte-identical: agents' skills and scripts parse it. The only intended difference is that `runner status` reports the `runnerd.sock` endpoint in `socket`, plus the follow wording in phase 2.
- Exit codes: 3 not running, 5 blocked by a sandbox (keep `connect_failure`'s semantics), 4 reserved for #562's `wait`, 2 usage.
- Caller identity (arch §9.3): every `from`/handle check an MCP tool makes stays daemon-side and behaves the same.
- Inside a mission, `msg post`, `msg read`, `signal` and `ask` keep appending to the event log directly, with no socket.
- The CLI still starts `runnerd` when it is not running and refuses to over ssh.
- Settings → MCP, the catalog of the user's own MCP servers, stays: `ops/mcp.rs`, `protocol/mcp.rs`, `surfaces/settings/mcp.rs` and the client registrations are not Runner's MCP server.

## Phase 0 — goldens on today's MCP path, then a checkpoint

Before changing the transport, add a golden test that runs the built `runner` binary against an isolated daemon (temporary app data, test pipe names on Windows, as `tests/daemon_process.rs` does) seeded with fixed fixture data, and records each case's argv, exit code, stdout and non-empty stderr. Cover every leaf in `every_command_leaf_parses_without_a_socket`, in human and `--json` form where both exist, plus not-found, usage and not-running errors and a non-follow `mission feed`.

Format: one text file, one block per case: `$ runner <argv>`, `exit <n>`, then the output verbatim. Normalize only what cannot be fixed (ULIDs, timestamps, PIDs, temp paths) with named placeholders, and document the rule at the top of the file. Budget about 2,000 lines in total; check size at the checkpoint. Do not use pretty-printed JSON snapshots of internal values: the golden is what the user sees.

Hand off to the reviewer at this point. The reviewer checks coverage against the command list, readability and determinism (run it three times), and posts `GOLDENS OK` before any transport code changes. Phase 1 must leave the file unchanged except for the `status` socket line.

## Phase 1 — the switch

1. **Typed calls over `runnerd.sock`.** The CLI reaches the daemon through `DaemonClient` over `SocketTransport` instead of rmcp. Every MCP tool the CLI uses that has no request yet (for example `role_get_by_handle`, `mission_list_summary`, `mission_feed`, `mission_post`, `mission_signal`, the project tools) gets a row in `api.rs` whose handler calls the same daemon code the tool called, so results are unchanged. Where a tool's JSON differs from the existing request's result type, the CLI keeps the tool's shape: the goldens decide.
2. **The recorder test** checks that the requests the CLI reaches cover what the CLI needs, against the request table, in place of `RUNNER_TOOL_NAMES`.
3. **Version skew.** A CLI and a daemon from different builds must not show up as "not running". When a request cannot be decoded, the CLI prints one clear line telling the user to restart Runner; name the exit code in the handoff. Keep the check cheap: no hashing of the binary on every invocation.
4. **`runner status`.** `socket` names the runnerd endpoint, and `app_version` still reports the daemon's version. The client protocol does not carry a version today: add it to the handshake or as a `fast` request.
5. **Removal.** Delete the daemon's MCP server and tool registry (`crates/runner-daemon/src/mcp/`), `RUNNER_TOOL_NAMES`, the `socket-schemas` golden and tests that exist only for the MCP layer, and all three `rmcp` entries (the CLI's client, its dev-dependency server and the daemon's server). `Cargo.lock` must end with no `rmcp` crate.
6. **Keep the older-app guard.** A 0.12 app does not know `runnerd.sock`; today runnerd notices one only because the old app replaces `mcp.sock`, and then it stops so two processes never own one database. Keep that: runnerd still binds the MCP endpoint (Unix socket and Windows pipe) as a sentinel, serves nothing on it (accept and close), and keeps its ownership check. One short comment says why. This is the only place `mcp.sock` survives.
7. **Docs.** Update `docs/arch/arch.md`, `concurrency.md`, `process-model.md`, `docs/product/vision.md` and the 645 spec and plan where they describe the CLI on `mcp.sock`. Leave archived records alone. Check both READMEs and change them together only if they mention it.

## Phase 2 — `mission feed --follow` as a subscription

The follower stops polling: the daemon pushes each appended mission event to the subscribed CLI. The printed lines, their order, `--since`/`--oldest-first`/`next_offset` cursor semantics, the snapshot drain on start, and the "watch ended" message and exit code on disconnect stay identical, with no event printed twice or skipped. Keep the existing follow tests passing and add one for a disconnect mid-follow. Update the polling sentence in `help.rs` and in the shipped skill text in `agent_skill.rs` to describe pushed delivery; that wording is the only intended output change besides `status`. If the push needs daemon plumbing beyond forwarding what the event bus already observes, describe it in the handoff before building it.

## Boundaries

Crews never run the dev app or drive Jason's Runner; Jason smoke-tests. Jason chose no QA slot and no live regression for this mission (2026-10-07): finish the code first. Do not launch live agents. Daemon tests use isolated temporary app-data directories and test pipe names; never touch real or development app data, or the production and development daemons. Do not start extra agents, crews or subagents. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff at the phase 0 checkpoint and at the end, then reviews the branch diff against #821 with must-fix findings first and file:line pointers. Focus on: goldens unchanged, every CLI path backed by a request, identity checks preserved, the sentinel guard on both platforms, nothing of Settings → MCP removed, no `rmcp` left, and Windows pipe handling. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run `cargo test --locked --workspace --no-fail-fast --profile ci`, workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`--features updater`), `cargo fmt --all --check` and `git diff --check`. Record the exact commands and exit codes.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main`, with a subject that names the change (for example `refactor(cli): move the runner CLI from MCP to the runnerd client protocol`), pushing `refactor/821-cli-client-protocol`, and opening a PR against main whose body says `Closes #821`. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, what changed, tests and exit codes, CI result, the reviewer's verdict, and what Jason should smoke-test (`runner status` names `runnerd.sock`; role, crew and mission commands with the app closed; `mission feed --follow` on a live mission; a command under the Codex sandbox exits 5). Then stand by.
