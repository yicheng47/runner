# 645 m1 — One request surface (runnerd 1a)

Program [#645](https://github.com/yicheng47/runner/issues/645) (`runnerd`), phase 1, mission 1a. Jason requested this mission on 2026-10-05 and moved its landing to the umbrella `feat/645-runnerd` that day, so QA’s regression on the umbrella covers 1a. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-645-m1-request-surface`, on the existing branch `refactor/645-m1-request-surface`, created from `origin/main` at `c4a58648`. The root checkout stays on `main`. Do not create another branch or worktree, and do not share a Cargo target directory. Treat any other worktree as another machine's checkout.

## The goal

Today the GPUI app builds `AppCore` in its own process and calls into it directly. `runnerd` (phase 1c) will move the core into a daemon, and the app will reach it over a socket. This mission does the part that changes no behaviour. **Every call from `runner-app` into the core goes through a `DaemonClient`, over an in-process transport that serializes every request and response.** When 1c adds the socket, it adds a transport and nothing else. The app looks and behaves exactly as it does today.

## Read first

- `AGENTS.md`, especially Worktrees and Crew Missions.
- `docs/features/645-session-host.md`: "The daemon owns the state", "What moves where", "The client protocol" (its three rules), and Decisions.
- `docs/impls/645-runnerd/plan.md`: "Crates" and "Mission 1a". The design is there. This brief wins on any detail.
- `crates/runner-backend/src/lib.rs` (`AppCore`), `events.rs` (`AppEvent`, `EventChannel`), `model.rs`, and `ops/*.rs`. Ops come in two shapes: low-level `fn(conn, …)` and wrappers that take `&AppCore`.
- `crates/runner-backend/src/mcp/`: the MCP tools already wrap ops with serde inputs, the closest existing precedent for a request table.
- `crates/runner-core/src/lib.rs` and `model.rs`. Core already holds the event types and has an optional `schemars` feature.
- `crates/runner-app/src/app_store.rs` (the `core` field, the event thread, and the payload decode), `bootstrap.rs` and `main.rs`.

## What the app touches today (main at `c4a58648`)

- 82 files reach `runner_backend`.
- They call 99 distinct `ops::` functions (session 26, mission 16, runtime 14, node 10, role 9, slot 7, window 7, crew 6, project 4) and 16 `repo::` functions.
- Direct `AppCore` field use: `core.db` 18 times, `usage` 6, `events` 6, `runtime_discovery` 4, `runtime_shell_env` 3, `session_events` 3, `mcp` 3, `windows` 2, `sessions` 2, `broadcast_focus_map` 1.
- Other types the app reaches: `usage::{UsageService, RuntimeUsage}`, `router::runtime::PermissionMode`, `session::status::{AgentStatus, Lifecycle, AgentObservation}`, `runtime_status`, `shell_path::{DiscoveryOutcome, LoginShellEnv}`, `windows::Subject`, the `cli_install` status types, `agent_skill::SKILL_MARKER` and `skills::skill_catalog`.
- About 88 call sites already run inside `cx.background_spawn`. About 100 outside tests run on the main thread, most in `start_chat.rs`, `app_store.rs`, `bootstrap.rs`, `chat.rs`, `main.rs` and `windowing.rs`.

These counts come from a script; derive your own at the branch point.

## Deliverables

1. **`runner_core::protocol`, a module of `runner-core`, not a new crate.** It holds:
   - the row types moved from `runner-backend/src/model.rs`;
   - every argument and result type the app sends or receives, moved from their backend modules;
   - `ClientEvent { name: String, payload: serde_json::Value }`;
   - `api.rs`, one macro table that lists every request once: its name, argument type, result type, and whether it is `fast`. A `fast` op is a read of in-memory state or a single-row query. The macro generates `Request`, `Response` and typed `DaemonClient` methods, all synchronous and blocking;
   - `client.rs`, with `DaemonClient` (holding an `Arc<dyn Transport>`), the `Transport` trait (`call` and `subscribe`), and a `ClientError` that carries the daemon's error message.

   Every type that moves keeps a `pub use` at its old backend path, so backend code and the MCP tools compile unchanged. `schemars` derives stay behind core's existing `schemars` feature. Add no new dependencies.
2. **`runner-backend/src/daemon/`**, with two parts:
   - `dispatch(&AppCore, Request) -> Response`, driven by the same table;
   - `InProcessTransport(AppCore)`, which encodes each request to JSON, decodes it, dispatches, and does the same with the response. It does this on every call, not only in tests, so a type that could not cross a socket fails now. Its `subscribe` maps `EventChannel`.
3. **The app converts.** Every call outside test modules goes through `DaemonClient`. `AppStore` holds the client, and `AppEvent`s arrive through `client.subscribe()`. Some logic that lives in the app today drives core work itself: `consume_resume_on_launch` from Settings, `usage.set_enabled` in `main.rs`, and the discovery reads behind skill and command defaults. That work also goes through requests. Where the daemon will own the logic, move it into `runner-backend` behind a request. The skill and command defaults may stay in the app as file effects, reading their data through requests.
4. **The thread rule (Jason corrected it on 2026-10-05 after review round 1).** Keep each call on its original thread: synchronous main-thread calls stay synchronous requests, and existing `background_spawn` calls stay there. Use `fast` requests for direct in-memory reads, including discovery reads behind runtime forms; forms read current daemon state rather than a lagging cache. Do not add ordering queues, generations or stale-completion guards to compensate for newly moved work. No request from `render`, `prepaint` or `paint`; report any baseline core access found on those paths and refresh the local render data outside rendering. Put every non-`fast` request that remains on the main thread in the PR body, with file:line and op. Mission 1c decides what moves for the socket transport.
5. **The guard test.** It scans `runner-app/src` outside test modules and fails on any `runner_backend::` path, `.db.get(`, or the `AppCore` fields above, except an explicit allowlist. Each allowlist entry names the mission that removes it:
   - `bootstrap.rs`: `boot_core`, `NativeMcpServer` and quit teardown (1c);
   - `terminal/*`, `terminal_ime.rs` and `surfaces/agent_update.rs` (1b);
   - the `TerminalBridge::new(core…)` construction in `app_store.rs` (1b).
6. **Tests.**
   - One serde round-trip test that covers every request and response with a compact sample. Keep it small, with no large goldens: #791's first goldens reached 57,650 lines and had to be compacted before merge ([record](../../tests/archive/791-session-state-reducer.md)).
   - App tests build their client through one helper. The eight helpers that build `AppCore` by hand today use it: `bootstrap.rs`, `app_store/skill_defaults.rs`, `app_store/command_default.rs`, `terminal/element.rs`, `settings_page.rs`, `app_shell.rs`, `agent_update.rs` and `start_chat.rs`.
   - Every existing test passes unchanged.

## Not in this mission

- **The terminal:** `runner-terminal`, `TerminalSession`, `TerminalBridge`, `terminal_ime.rs`'s error type, and `UpdateTerminalEvents`. That is 1b.
- **The daemon process:** the socket, frames, handshake, `runnerd` itself, and moving `boot_core` or `NativeMcpServer`. That is 1c.
- Any behaviour, UI, design file or README change.
- The crate rename, which is 1d.

## Rules

- **No behaviour change.** Keep the same UI, timings and call threads. Do not move synchronous work to the background in 1a; list any render-path access that must move into a main-thread handler or event callback.
- Stage by path, never `git add -A`.
- **Do not launch the app (`make run`); Jason smoke-tests.** The CLI in the build tree is `target/debug/runner-agent-cli`. `target/debug/runner` is the GUI on this case-insensitive volume; never run it.
- Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`. Windows clippy has failed on this four times.
- Do not start extra agents, crews or subagents.

## Verification

1. Record `cargo test --locked --workspace --profile ci` passed and ignored counts at the branch point, before any change. After the change, every original test still passes, and the only additions are the new tests. Use `set -o pipefail` or check Cargo's exit status directly.
2. `make verify`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, macOS updater Clippy (`--features updater`), `cargo fmt --all --check` and `git diff --check`. Record each command and its exit code.
3. Report:
   - the number of requests in the table, and how many are `fast`;
   - the main-thread table;
   - every moved type, with its old and new paths;
   - the allowlist;
   - every logic move into `runner-backend`;
   - every call that moved off the main thread.

## Review

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against this brief, with must-fix findings first and file:line pointers. It checks four things:
- no behaviour change;
- no request on a render path; original call threads preserved, with every remaining non-`fast` main-thread request inventoried;
- every request crossing JSON in the in-process transport;
- the guard test failing on a planted violation (try one in a scratch copy, never the repository).

Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

## Authorization

After a clean review, Jason authorizes the following:
- Squash all work on this branch, this brief and the `docs/impls/645-runnerd/` updates included, into one commit on top of current `origin/feat/645-runnerd`, with a subject that names the change (for example `refactor(app): reach the core only through DaemonClient (#645 1a)`).
- Push `refactor/645-m1-request-surface` and open a PR against `feat/645-runnerd`. The body says `Refs #645`, not `Closes`, and carries the gates and the main-thread table, with no Claude session link.
- Fetch origin and rebase onto `origin/feat/645-runnerd`; never merge the umbrella into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`.
- Drive CI green on `Rust / macOS` and `Rust / Windows`.

Do not merge, delete the branch or worktree, or cut a nightly or release.

The final Runner handoff carries:
- the PR URL;
- what changed;
- tests and exit codes;
- the CI result;
- the reviewer's verdict;
- what Jason should smoke-test: a direct chat and a role chat (type, stop, resume, fork), a mission (start, stop and resume a slot, archive), creating, editing and deleting roles, crews and projects, every Settings pane (an Agents refresh, the usage pill, Skills, MCP, the command install status), and the sidebar (pin, rename, drag, a second window).

Then stand by.
