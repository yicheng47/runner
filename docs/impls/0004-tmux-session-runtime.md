# Tmux Session Runtime and PATH Recovery

## Context

Issue #65 reports an intermittent packaged-app failure when a direct chat
spawns a runner whose command is a bare CLI name, for example `claude`:

```text
session_start_direct: spawn claude: Unable to spawn claude because it doesn't exist on the filesystem and was not found in PATH
```

The immediate cause is still macOS GUI PATH bootstrap. Finder/Dock starts Runner
through launchd with a stripped PATH (`/usr/bin:/bin:/usr/sbin:/sbin`). The
current backend captures the user's login-shell PATH once at startup in
`src-tauri/src/lib.rs`; if that single capture times out or returns `None`,
direct chats can fail for the whole app lifetime.

The broader cause is that Runner currently owns too much terminal lifecycle
state directly through `portable-pty`: attach/detach timing, output buffering,
scrollback replay, session survival, and input injection. We fixed parts of
that, but the same class of bugs keeps returning.

Decision: use `tmux` as the session runtime for now. We can add a `native-pty`
runtime later behind the same internal boundary if we want a no-dependency mode.

## Goals

- Use `tmux` as the owner of direct-chat and mission agent terminals.
- Make app restart/window reload attach to existing terminal state instead of
  reconstructing it from frontend buffers.
- Preserve scrollback/history through `tmux capture-pane`.
- Send user/system-prompt input through tmux paste/send APIs instead of racing
  frontend terminal mount.
- Make PATH resolution deterministic at launch; no spawn correctness should
  depend on a shell PATH timer succeeding.
- Keep direct chats off the mission bus. Direct chats must not receive the
  bundled `runner` CLI or mission event-log env unless they are mission sessions.

## Non-goals

- Keep `portable-pty` as an active runtime in this PR.
- Add Docker/runtime sandboxing.
- Add a new frontend product surface.
- Depend on terminal keystroke typing as the primary system-prompt mechanism
  when an agent supports a first-class flag/config. Tmux paste is the fallback.

## Approach

Introduce a tmux-backed runtime inside the backend session layer:

1. Add tmux discovery and clear dependency errors.
2. Create one tmux session per Runner session row.
3. Store tmux identifiers on the session entity.
4. Stream output by polling/capturing tmux pane deltas.
5. Send input via tmux paste-buffer / send-keys.
6. Reattach after app restart by reading stored tmux identifiers and checking
   whether the tmux pane still exists.
7. Resolve launch PATH through a deterministic command wrapper, not by waiting
   on a startup timer.

The first implementation can replace the active `portable-pty` path. Keep the
new API shaped as a runtime boundary so `native-pty` can return later without
rewriting commands/frontend.

---

## Step 1: Add tmux runtime boundary

**Files:**

- `src-tauri/src/session/manager.rs`
- optionally `src-tauri/src/session/runtime.rs`
- optionally `src-tauri/src/session/tmux.rs`

Add a small internal runtime abstraction:

```rust
trait SessionRuntime {
    fn spawn(&self, spec: SpawnSpec) -> Result<RuntimeSession>;
    fn resume(&self, session: &Session) -> Result<RuntimeSession>;
    fn stop(&self, session: &Session) -> Result<()>;
    fn send_input(&self, session: &Session, bytes: &[u8]) -> Result<()>;
    fn capture_since(&self, session: &Session, cursor: CaptureCursor) -> Result<CaptureChunk>;
    fn resize(&self, session: &Session, cols: u16, rows: u16) -> Result<()>;
}
```

This does not need to be public or over-abstracted. It is just the seam between
the command layer and the terminal owner. For this PR, instantiate only the
tmux runtime.

## Step 2: Discover tmux without depending on GUI PATH

**File:** `src-tauri/src/session/tmux.rs`

Add `resolve_tmux_binary()`:

- If `RUNNER_TMUX` is set, use it.
- Search the current PATH.
- Search common absolute locations:
  - `/opt/homebrew/bin/tmux`
  - `/usr/local/bin/tmux`
  - `/usr/bin/tmux`
- Return a clear error if missing:

```text
tmux is required for Runner sessions but was not found. Install tmux or set RUNNER_TMUX=/path/to/tmux.
Searched: ...
```

This is independent of runner command PATH resolution. Even if the app launched
with a stripped PATH, Homebrew tmux should still be found through the fallback
locations.

## Step 3: Store tmux identifiers on sessions

**Files:**

- `src-tauri/migrations/....sql`
- `src-tauri/src/model.rs`
- `src-tauri/src/commands/session.rs`
- `src-tauri/src/commands/mission.rs`

Extend `sessions` with nullable runtime metadata:

- `runtime`: text, initially `tmux`
- `runtime_session`: text, for example `runner-<session_id>`
- `runtime_window`: text or integer if needed
- `runtime_pane`: text, tmux pane id like `%3`
- `runtime_cursor`: integer, last captured pane line/output cursor if we persist it

Keep the schema generic enough for a future `native-pty` runtime. Do not name
columns `tmux_*` unless there is a strong reason; use `runtime_*`.

On spawn, write these fields immediately after tmux creates the session/pane.
On list/load, include enough metadata for the backend to reattach; the frontend
does not need to understand tmux.

## Step 4: Launch agent commands through a wrapper

**Files:**

- `src-tauri/src/session/tmux.rs`
- `src-tauri/src/session/manager.rs`
- `src-tauri/src/shell_path.rs`

Tmux alone does not fix PATH. If Runner starts tmux from a GUI environment, the
tmux server can inherit the stripped launchd PATH. So the tmux runtime must
launch agents through a deterministic wrapper.

For each session, generate a small launch script under the existing per-session
runtime directory. The script should:

1. Export the composed PATH.
2. Export mission/direct-chat env vars.
3. `cd` to the working directory.
4. `exec` the runner command and args.

The composed PATH should be:

- mission sessions: shim dir, bundled app bin, best-effort shell PATH, fallback
  CLI dirs, process PATH
- direct chats: best-effort shell PATH, fallback CLI dirs, process PATH

Direct chats must not include the bundled app bin. That preserves the off-bus
invariant from PR #51.

The shell PATH resolver may still run, but only as an enrichment source:

- Cache successful shell PATH values.
- Leave cache empty on timeout/failure so a future attempt can retry.
- Never make tmux launch correctness depend on the resolver finishing before a
  fixed timer.

Fallback CLI dirs:

- `~/.local/bin`
- `~/.cargo/bin`
- `~/.npm-global/bin`
- `/opt/homebrew/bin`
- `/usr/local/bin`

The launch script should be generated with controlled quoting. Do not build a
single shell string from user input when Rust can write each argument with a
small shell-quote helper and test it.

## Step 5: Create tmux sessions and panes

**File:** `src-tauri/src/session/tmux.rs`

For each Runner session:

- tmux session name: `runner-<session_id>`
- window name: sanitized runner handle or `main`
- pane: first pane in the session
- working directory: mission/direct-chat working directory

Spawn shape:

```text
tmux new-session -d -s runner-<session_id> -n main -c <cwd> <launch-script>
tmux display-message -p -t runner-<session_id>:main '#{pane_id}'
```

Persist the returned pane id. Prefer pane ids over indexes because indexes can
change.

Before creating a new tmux session, check whether `runner-<session_id>` already
exists. If it exists and the DB says the session is running, reattach instead of
spawning a duplicate.

## Step 6: Stream output from tmux

**File:** `src-tauri/src/session/tmux.rs`

Replace direct `portable-pty` reads with tmux capture:

- Use `tmux capture-pane -p -t <pane_id> -S <start> -E -`
- Track the last captured line or capture sequence in backend memory.
- On frontend attach, capture the full available scrollback once and emit it as
  replay.
- While running, poll at a short interval and emit only new content.

This does not need perfect byte-for-byte terminal history in the first pass, but
it must preserve the user-visible scrollback across:

- tab switch
- route switch
- window reload
- app restart while tmux server is still alive

If capture-pane line cursoring is too lossy for interactive redraws, keep a
small backend transcript as a second layer. Tmux still remains the source of
truth for process lifetime.

## Step 7: Send input through tmux

**File:** `src-tauri/src/session/tmux.rs`

Use tmux APIs instead of writing to a frontend-owned PTY writer:

- normal text / pasted prompt:

```text
tmux load-buffer -
tmux paste-buffer -t <pane_id> -d
```

- Enter/control keys:

```text
tmux send-keys -t <pane_id> Enter
tmux send-keys -t <pane_id> C-c
```

This should address the input-buffer problems where injected content lands one
line off or disappears because the xterm component was not attached at the exact
right time.

System prompt delivery order:

1. Prefer agent-native config/flags when available.
2. Use generated config/prompt files when the agent supports them.
3. Use tmux paste-buffer as the fallback for interactive prompts.

## Step 8: Resume and status reconciliation

**Files:**

- `src-tauri/src/session/manager.rs`
- `src-tauri/src/commands/session.rs`
- `src-tauri/src/commands/mission.rs`

On app startup or session list:

- If a session row has `runtime = 'tmux'`, check whether its pane still exists:

```text
tmux has-session -t runner-<session_id>
tmux list-panes -t runner-<session_id> -F '#{pane_id} #{pane_dead} #{pane_current_command}'
```

- If pane exists and is alive, mark the session `running` and allow attach.
- If pane exists but is dead, capture final scrollback and mark stopped/failed
  based on exit status when available.
- If pane/session is missing, mark stopped with a clear terminal-unavailable
  reason.

This is where app restart gets its value: closing Runner should not kill tmux
sessions; reopening Runner should discover and attach to them.

## Step 9: Replace portable-pty usage

**Files:**

- `src-tauri/src/session/manager.rs`
- `src-tauri/Cargo.toml`

Move active spawn/input/output/resize paths to tmux. Leave `portable-pty` code
only if it is behind disabled/dead code that does not run, or remove it in the
same PR if the patch stays clean.

The externally visible command API should stay stable:

- `session_start_direct`
- `session_input`
- `session_resize`
- `session_stop`
- mission start/stop/resume commands

Frontend should continue rendering the same terminal component. It receives
backend output events and sends input events; it does not need to know whether
the runtime is tmux.

## Step 10: Tests

Unit tests:

- tmux binary resolution:
  - honors `RUNNER_TMUX`
  - searches fallback paths
  - returns clear missing-tmux error
- PATH composition:
  - direct chat excludes bundled app bin
  - mission includes shim and bundled app bin first
  - fallback CLI dirs are included without relying on shell resolver success
- launch script generation:
  - quotes args with spaces/special characters
  - exports expected env
  - uses `exec`
- session naming:
  - deterministic `runner-<session_id>`
  - rejects unsafe names if any external string is used

Integration tests, gated behind local tmux availability:

- spawn a session running a simple shell command
- capture initial and later output
- send input and observe output
- re-create `SessionManager` and attach to the same tmux session
- stop a session and reconcile status

Manual smoke:

1. Install/start packaged app from Finder/Dock.
2. Start a direct chat with `command = "claude"` or `codex`.
3. Confirm startup output appears.
4. Switch sessions and confirm the terminal switches and previous scrollback
   remains.
5. Close and reopen Runner while the tmux session is alive; confirm the session
   reattaches with scrollback.
6. Start a mission with two slots; confirm each runner tab is clickable and
   attaches to the right pane.
7. Send a long prompt/system-prompt fallback paste and confirm it lands in the
   active agent prompt without line-offset corruption.

## Files to modify

| File | Change |
|------|--------|
| `src-tauri/src/session/tmux.rs` | New tmux runtime: discovery, spawn, capture, input, resize, stop, resume. |
| `src-tauri/src/session/manager.rs` | Route session lifecycle through tmux runtime and persist runtime metadata. |
| `src-tauri/src/shell_path.rs` | Keep best-effort shell PATH parser; remove it from spawn correctness. |
| `src-tauri/src/lib.rs` | Initialize tmux runtime/session manager. |
| `src-tauri/src/model.rs` | Add runtime metadata fields to session model. |
| `src-tauri/src/commands/session.rs` | Surface runtime-backed session state without exposing tmux details. |
| `src-tauri/src/commands/mission.rs` | Use runtime-backed spawn/resume/stop for mission sessions. |
| `src-tauri/migrations/...sql` | Add nullable `runtime_*` columns to sessions. |
| `src-tauri/Cargo.toml` | Remove `portable-pty` only if no active code still needs it. |

## Verification

Automated:

- `cargo fmt --all --check`
- `cargo test --lib`
- `cargo clippy --lib --tests -- -D warnings`
- `pnpm exec tsc --noEmit`
- `pnpm run lint`

Manual packaged-app smoke:

1. Launch from Finder/Dock, not `pnpm tauri dev`.
2. Confirm missing tmux produces a clear actionable error.
3. Confirm installed Homebrew tmux is discovered even under stripped GUI PATH.
4. Confirm direct chat starts without the shell PATH resolver needing to finish
   before a timer.
5. Confirm direct chats still cannot call bundled `runner` bus commands unless
   the user's own PATH contains a separate `runner`.
