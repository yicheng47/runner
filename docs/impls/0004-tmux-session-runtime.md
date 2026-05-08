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

## Progress

Implementation lives on `feature/tmux-session-runtime` (PR #67).
Foundation lands first as `#![allow(dead_code)]` modules; the
manager wiring (Step 9) flips the active code path off
`portable-pty` once Steps 5–8 are in.

| Step | Status | Commit | Notes |
|---|---|---|---|
| 1. SessionRuntime trait | ✅ done | `7649548` (refined `46f25f1`) | Push channel `OutputStream` with `Replay`/`Stream` discriminator; input split into `paste` / `send_bytes` / `send_key`. |
| 2. tmux discovery + private socket | ✅ done | `7649548` | `resolve_tmux_binary` (RUNNER_TMUX → PATH → Homebrew/Linux fallbacks; `TmuxRequiresUnix` on Windows), `write_runner_config` (idempotent), `tmux_cmd()` helper. |
| 3. Schema migration | ✅ done | `112b43e` | `0003_session_runtime.sql` adds nullable `runtime_*` columns; defensive `PRAGMA table_info` test. |
| 4. Launch wrapper + composed PATH | ✅ done | `eaba871` (env-name validation `e47e085`) | `compose_path` (off-bus invariant for direct chats, dedupe, fallback dirs always included), `shell_quote`, `render_launch_script` (POSIX-identifier validation), `write_launch_script` (mode 0700). |
| 5+6+7. `TmuxRuntime` impl (spawn / streaming / input) | ✅ done | `553cc88` (review fixes `1ba563d`) | `tmux_runtime.rs` (Unix-only). spawn / resume share an `attach_streaming` helper that installs `pipe-pane` BEFORE `capture-pane` (closing the gap-loss case from review pass 1), drains FIFO non-blocking, then sends Replay + buffered Stream + spawns forwarder. FIFO opens `O_RDONLY \| O_NONBLOCK`; forwarder uses `poll(POLLIN, 200ms)` and an `Arc<AtomicBool>` cancellation flag (set by `OutputStream::Drop`) so detach doesn't leak threads. spawn cleans up the new tmux session if any post-`new-session` step fails. paste = `load-buffer -` then `paste-buffer -p -r -d`. send_bytes = `send-keys -l --`. send_key = `send-keys` with name validation. Plan corrections discovered: `window-size manual` triggers tmux 3.5a server-exit (dropped); `=name` exact-match is session/window only — pane ids (`%N`) need raw target. |
| 8. Resume + status + config reconciliation | ✅ done | `13c4f76` (legacy-server fix `8b64335`) | `SessionRuntime::status()` returns `Option<SessionStatus { alive, exit_code, pid, command }>` parsed from `list-panes -s -F '#{pane_id} #{pane_dead} #{pane_dead_status} #{pane_pid} #{pane_current_command}'`; `Ok(None)` = terminal-unavailable. `TmuxRuntime::reconcile_config()` two-step probe: `list-sessions` for liveness, then `show-options -g -v @runner_config_version` for stamp; missing/different stamp → `source-file <runner.conf>`. Bug fix during integration testing: `has-session`/`kill-session` need `-t` before the target arg — bare argv didn't work even though tmux's interactive parser is more forgiving. |
| 9. Replace `portable-pty` in active code path | ✅ done | `6600d79` (scaffold) + cutover | `SpawnSpec` extended with `shim_dir` / `bundled_bin_dir` / `shell_path` / `initial_size`; `TmuxRuntime::spawn` consumes them via `launch::compose_path` and applies the initial pane size via post-spawn `resize-window`. `SessionManager` now holds `Arc<dyn SessionRuntime>` and delegates spawn / spawn_direct / resume / inject_stdin / resize / kill through the runtime layer. New `inject_paste(payload)` uses `paste-buffer -p -r -d` + `send_key("Enter")` so multi-line first-prompt blocks land as one bracketed-paste event rather than 50 keystrokes. Forwarder thread drains `OutputStream` into `session/output` events; on channel close queries `runtime.status()` for exit code and applies the existing stop-vs-crash discrimination. `portable-pty` dropped from `Cargo.toml`; the old reader/writer/master/Child machinery is gone. 12 manager tests that exercised real PTY spawn through portable-pty are `#[ignore]`'d for Step 10 to rewrite against `TmuxRuntime`; the remaining 185 manager + module tests pass. |
| 10. Unit + integration tests | ⏳ rolling | — | Per-step unit tests landing alongside foundation; integration tests over `-L runner-test-<pid> -f <temp.conf>` arrive in Step 10. |

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
4. Stream live output via `tmux pipe-pane` (raw byte stream into a
   per-session FIFO that the runtime forwards as `session_output`).
   Use `capture-pane` only for one-shot scrollback replay on attach.
5. Send input via tmux paste-buffer / send-keys.
6. Reattach after app restart by reading stored tmux identifiers and checking
   whether the tmux pane still exists.
7. Resolve launch PATH through a deterministic command wrapper, not by waiting
   on a startup timer.

The first implementation can replace the active `portable-pty` path. Keep the
new API shaped as a runtime boundary so `native-pty` can return later without
rewriting commands/frontend.

## Prior art and design constraints

Two reference points shaped the decisions below; the rest of the plan
quietly assumes them.

- **`claude-squad`** (`smtg-ai/claude-squad`, `session/tmux/tmux.go`)
  is the closest existing Go orchestrator running multiple Claude Code
  instances on tmux. Patterns we copy: persist-and-resurrect by
  recomputing the tmux session name from the runner-side id (no opaque
  tmux ids in the DB); a sanitized name prefix
  (`runner-<session_id>`); `set-option history-limit` on session
  create; exact-match `-t=name` rather than `-t name` (the latter
  prefix-matches and bites — `tmux.go:459` calls this out inline).
  Patterns we deliberately **reject**: claude-squad keeps a hidden
  `tmux attach` PTY per session and writes user keystrokes to that PTY
  (`tmux.go:184,211-231`). That model is the proximate cause of the
  dropped first-prompt bug (cs#266 — papered over with a 100ms sleep
  before sending CR), the "error capturing pane content" races
  (cs#189/#216/#218 — pane gone between `has-session` and
  `capture-pane`), and a 50ms stdin "nuking" hack (`tmux.go:305-317`)
  to scrub leaked terminal-control sequences from Warp/iTerm.
  claude-squad also has **no death detection** — `pane_dead` is never
  read. We use tmux's own `load-buffer` / `paste-buffer` / `send-keys`
  APIs (no attached PTY) and poll `pane_dead_status`.
- **tmux man page + GitHub issues** for behaviors that bite if you
  don't know them: `force-width`/`force-height` were removed in 2.9
  (tmux#2671) — detached panes need `window-size manual` plus
  `default-size` plus explicit `resize-window` (tmux#1367);
  `pane_dead`/`pane_dead_status` only populate when `remain-on-exit
  on` is set on the window (tmux#2552); control mode (`tmux -CC`) is
  tempting for structured `%output`/`%exit` events but `%output`
  chunks split mid-UTF-8-codepoint (wezterm#6769) and the parser
  surface is non-trivial — defer to a later iteration.

Net of the above: v1 is **`pipe-pane` for live byte streaming +
`capture-pane` for attach replay + `load-buffer`/`paste-buffer
-r`/`send-keys` for input, all over a private tmux socket loaded
from a generated Runner config file** (so server-wide options can't
get lost between transient `set-option` invocations). `remain-on-exit
on` + `window-size manual` + `history-limit 50000` live in that
config.

---

## Step 1: Add tmux runtime boundary

**Files:**

- `src-tauri/src/session/manager.rs`
- optionally `src-tauri/src/session/runtime.rs`
- optionally `src-tauri/src/session/tmux.rs`

Add a small internal runtime abstraction. Output is a **push
channel** with discriminated `Replay` (one-shot snapshot, xterm.js
resets) vs `Stream` (live PTY bytes, xterm.js appends) so a
careless impl can't conflate them. Input is split by intent
(`paste` / `send_bytes` / `send_key`) so the runtime can pick the
right tmux primitive without inferring from byte content.

```rust
pub enum RuntimeOutput {
    Replay(Vec<u8>),
    Stream(Vec<u8>),
}
pub type OutputStream = std::sync::mpsc::Receiver<RuntimeOutput>;

pub trait SessionRuntime: Send + Sync {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)>;
    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream>;
    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()>;

    /// `paste-buffer -p -r -d` semantics — bracketed paste, LF
    /// stays literal. Runtime does NOT submit; manager calls
    /// `send_key("Enter")` after the readiness wait.
    fn paste(&self, session: &RuntimeSession, payload: &[u8]) -> RuntimeResult<()>;

    /// `send-keys -l -- <bytes>` — literal byte stream from
    /// xterm.js passthrough.
    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()>;

    /// `send-keys -t=<pane> <name>` — named keys (`"Enter"`,
    /// `"C-c"`, `"Up"`).
    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()>;

    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()>;
}
```

Std `mpsc::Receiver` rather than tokio's: `manager.rs` is
`std::thread`-based throughout — no tokio runtime in the session
layer. This does not need to be public or over-abstracted. It is
just the seam between the command layer and the terminal owner.
For this PR, instantiate only the tmux runtime.

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

### Use a private tmux server (`-L runner -f <runner.conf>`)

Every tmux invocation must include both `-L runner` (or `-L
$RUNNER_TMUX_LABEL` for tests/dev) **and** `-f <runner.conf>`, where
`<runner.conf>` is a Runner-managed tmux config file written to the
app data dir (e.g. `~/Library/Application
Support/com.wycstudios.runner/tmux.conf` on macOS,
`$XDG_DATA_HOME/com.wycstudios.runner/tmux.conf` on Linux).

Why both:

- **`-L runner`** resolves to a separate socket at
  `/tmp/tmux-<UID>/runner` with the inherited 0700 dir perms — we
  don't share state with the user's default `tmux ls` server. Avoids
  cs#277 (env-var inheritance from a pre-existing user tmux server)
  by construction.
- **`-f <runner.conf>`** replaces `~/.tmux.conf`. `-L` only isolates
  the *socket*; the server still reads the user's config by default,
  which can rebind keys, set `default-shell`, alias `paste-buffer`,
  override `history-limit`, etc. We need a config we control.
- Use a **real config file**, not `-f /dev/null`. With a stand-alone
  empty config, the server-wide options have to be applied via
  separate `tmux set-option -g …` invocations — but with default
  `exit-empty on` and no sessions yet, the server can exit between
  the `set-option` command and the first `new-session`, throwing the
  options away. This bites `history-limit` particularly hard because
  it must be set **before** the first pane is created. Loading the
  options from `-f <runner.conf>` makes them apply at every server
  start, no race window.

The config we generate (one-shot on first use, idempotent
overwrite-if-stale on subsequent starts):

```text
# Runner-managed tmux config — do not hand-edit; rewritten on app start.
set-option -s exit-empty off
set-option -g history-limit 50000
set-option -g window-size manual
set-option -g default-size 120x32
set-option -g remain-on-exit on
set-option -g status off
```

`exit-empty off` keeps the server alive between sessions — useful so
that "the user closed the chat tab, then immediately opened a new
one" doesn't churn through a server start/stop cycle, and so the
server stays around for app-restart reattach.

Other benefits of this socket+config layout:

- We own server-wide options without polluting the user's tmux
  config.
- "Kill all Runner sessions" is a single `tmux -L runner -f
  <runner.conf> kill-server` for diagnostics and the dev "wipe
  state" flow.

Prefer `-L runner` over `-S /path/to/sock` because tmux already
places `-L` sockets under the per-UID dir with the right perms;
rolling a custom path means hand-managing the parent directory mode.

Wrap this in a single helper: `fn tmux_cmd() -> Command` returning a
`Command::new("tmux").args(["-L", "runner", "-f", &config_path])` so
the global flags can never be forgotten at a call site. Tests can
inject a tempfile path via the same helper.

### Windows fallback

Tmux does not run natively on Windows. `resolve_tmux_binary()` should
return a typed `RuntimeUnavailable::TmuxRequiresUnix` error on Windows;
the runtime registry should refuse to construct `TmuxRuntime` and the
session manager should surface a "tmux runtime is macOS/Linux only;
native-pty runtime not yet shipped" error. v1 ships macOS + Linux only.

## Step 3: Store tmux identifiers on sessions

**Files:**

- `src-tauri/migrations/....sql`
- `src-tauri/src/model.rs`
- `src-tauri/src/commands/session.rs`
- `src-tauri/src/commands/mission.rs`

Extend `sessions` with nullable runtime metadata:

- `runtime`: text, initially `tmux`
- `runtime_socket`: text, the `-L` label (so we can support multiple
  Runner instances or test sockets without losing reattachment)
- `runtime_session`: text, deterministic `runner-<session_id>` (the
  same session_id as the row's primary key — no opaque mapping)
- `runtime_window`: text or integer if needed
- `runtime_pane`: text, tmux pane id like `%3` (pane ids survive
  index reshuffles; never persist `:0.0`-style indexes)
- `runtime_cursor`: integer, last captured pane history offset if we
  persist it for delta replay

Keep the schema generic enough for a future `native-pty` runtime. Do not name
columns `tmux_*` unless there is a strong reason; use `runtime_*`.

On spawn, write these fields immediately after tmux creates the session/pane.
On list/load, include enough metadata for the backend to reattach; the frontend
does not need to understand tmux.

Naming rule: `runner-<session_id>` must match `^[A-Za-z0-9_-]+$` (tmux
treats `:` and `.` as target separators / index separators; `;` is a
command terminator inside `send-keys`). Reject any external string
that doesn't match — runner handles can contain spaces / unicode and
must never reach a tmux name unsanitized. claude-squad's sanitizer
(`tmux.go:60-68`) strips whitespace and replaces `.` with `_` but
doesn't cap length or escape `:`; we use the row's ULID directly,
which is already in the safe alphabet.

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

### Server-level options (loaded from config, not runtime-set)

Server-wide options live in the generated `runner.conf` (Step 2);
they are **not** applied via runtime `set-option` calls. Doing it at
runtime is fragile: with default `exit-empty on` and no sessions yet,
the server can exit between the `set-option` invocation and the next
`new-session`, throwing the option away. `history-limit` is the
worst offender because it must be set before the first pane is
created — existing panes keep their old limit.

For reference, the config contains:

| Option | Value | Why |
|---|---|---|
| `exit-empty` (server) | `off` | Keep the server alive between sessions for snappy reattach. |
| `history-limit` | `50000` | 25× default 2000; long agent runs scroll back fine. |
| `window-size` | `manual` | No client is ever attached; without `manual`, tmux fights our `resize-window` calls (tmux#1367, tmux#2671). |
| `default-size` | `120x32` | Initial pane size before the first `resize-window`. |
| `remain-on-exit` | `on` | Required to read `pane_dead_status` after the agent exits (tmux#2552). |
| `status` | `off` | Cosmetic; no client renders it, and turning it off avoids tmux burning a row on resize calc. |

If we ever need a per-session-window override (e.g. a slot that
shouldn't persist on exit), use `set-option -w -t=<target>` — the
server-loaded global is the default; per-window flips override.

### Spawn shape

```text
tmux -L runner -f <runner.conf> new-session -d -P -F '#{pane_id}' \
    -s runner-<session_id> -n main -c <cwd> -- '<shell-quoted-launch-script>'
```

Notes:

- `-P -F '#{pane_id}'` makes `new-session` itself print the new
  pane id on stdout. Avoids a second `display-message` round-trip
  (one less point where the pane could exit between create and
  query).
- Use **exact-match `-t=name`** everywhere else, not `-t name`: the
  latter does prefix matching and a session named `runner-1` will
  collide with `runner-10`. claude-squad calls this footgun out
  inline at `tmux.go:459` — catch it at the helper layer (a
  `fn target(name: &str) -> String { format!("={}", name) }` keeps
  it hard to forget).
- The trailing positional argument to `new-session` is a
  **`shell-command` string**, not argv — tmux passes it through the
  user's `default-shell` with `-c`. If the launch-script path can
  contain spaces or special characters (it lives under the
  per-session runtime dir, which we control, but session ids /
  session paths can drift), single-quote the path explicitly when
  generating the command. The `--` before it stops tmux's own
  option parsing but does **not** disable the shell-string
  semantics.

Persist the returned `pane_id`. Pane ids (e.g. `%3`) survive index
reshuffles; never persist `:0.0`-style indexes.

Before creating a new tmux session, check whether `runner-<session_id>`
already exists (`tmux -L runner -f <runner.conf> has-session -t=runner-<session_id>`).
If it exists and the DB says the session is running, reattach instead
of spawning a duplicate. If it exists and the DB says stopped, kill
the stale session before respawning.

### Resize

Frontend resize events translate to:

```text
tmux -L runner -f <runner.conf> resize-window -t=runner-<session_id>:main -x <cols> -y <rows>
```

Because `window-size manual` is set at the server level, this is the
**only** way the pane size changes — there's no attached client
SIGWINCH'ing the server (we never run `tmux attach`). claude-squad
piggy-backs on the PTY they keep attached and uses `pty.Setsize` with
a 50ms SIGWINCH debounce (`tmux_unix.go:43-77`); we get the same
effect with `resize-window` plus an in-Rust debounce in the runtime
layer (target ~50ms; coalesce repeated resizes within the window).

## Step 6: Stream output from tmux

**File:** `src-tauri/src/session/tmux.rs`

Replace direct `portable-pty` reads with two distinct mechanisms:

| Need | Tool |
|------|------|
| Live byte stream → `session_output` events | `pipe-pane` |
| One-shot scrollback replay on attach | `capture-pane` |

These are not interchangeable. **`capture-pane` returns a screen
snapshot, not a stream of terminal bytes.** Hashing snapshots and
appending them into xterm.js on change would duplicate the same
on-screen cells every tick they redraw — the cursor moves, a
spinner spins, and we'd append a near-identical screen on every
hash mismatch. xterm.js needs the raw output the agent wrote to its
PTY, in order, exactly once. That's `pipe-pane`.

### Live stream via `pipe-pane`

Two-step install, always:

```text
# 1. Close any existing pipe on this pane (no-op if none):
tmux -L runner -f <runner.conf> pipe-pane -t=<pane_id>

# 2. Install the new pipe (output direction only):
tmux -L runner -f <runner.conf> pipe-pane -O -t=<pane_id> \
    'cat >> '"'"'/path/to/per-session/runtime-dir/output.fifo'"'"
```

Flag semantics (often misremembered):

- **`-O`** = pipe pane **output** to the shell command's stdin (i.e.
  what the agent wrote to its PTY). Pair with `-I` if we ever want
  to feed input back through the same channel — we don't; input
  goes through `paste-buffer`/`send-keys` (Step 7).
- **Don't use `-o`.** `-o` (lowercase) = "only open if no pipe is
  currently open." It looks like a safety flag but is the wrong
  primitive for reattach: if a stale pipe from a previous Runner
  process is still registered against this pane (the Runner
  process crashed/restarted but tmux kept running because of
  `exit-empty off`), `-o` will *refuse* to install our new pipe
  and live output keeps going to a dead FIFO. Instead, always do
  the explicit close above first, then install without `-o`. tmux
  itself closes any existing pipe before attaching a new
  `pipe-pane` command, so the close is belt-and-suspenders for
  the case where we want to stop output entirely (no replacement).
- Stop a pipe explicitly with `tmux pipe-pane -t=<pane_id>` (no
  command, no flags) when shutting down a session.

Other notes:

- The shell command's stdin is the pane's raw PTY output,
  byte-for-byte. That's exactly what xterm.js needs.
- The trailing positional is a **shell-command string**, not argv
  (same shape as `new-session`'s last arg) — single-quote the FIFO
  path explicitly when generating the command. The double-quote /
  single-quote dance above (`'"'"'…'"'"'`) is one safe way; in Rust
  use a small `shell_quote_single` helper and concatenate.
- Use a **named FIFO under the per-session runtime dir** (`mkfifo`)
  rather than a regular file: regular files grow unbounded; the
  FIFO blocks the writer until a reader is attached, which is
  exactly the backpressure semantics we want.
- `pipe-pane` survives client detach (we never attach) and
  persists across `capture-pane` calls.
- Alternative if FIFOs become a portability headache on Linux
  containers: pipe to a Unix-domain socket we accept on, or open a
  control-mode child (deferred — see below). For v1, FIFO under
  the per-session dir.

### One-shot scrollback on attach

claude-code and codex are TUIs that switch into the **alternate
screen** (`\e[?1049h`) when running. tmux's main-screen scrollback
(`-S -`) is what was on screen *before* the TUI took over; the
alternate screen has no scrollback at all. So the right capture
shape depends on which screen the pane is currently displaying.

Query that first:

```text
tmux -L runner -f <runner.conf> display-message -p -t=<pane_id> '#{alternate_on}'
```

Then capture conditionally:

| `alternate_on` | Capture command | What we get |
|---|---|---|
| `0` (main screen) | `capture-pane -p -e -S - -E - -t=<pane_id>` | Full main-screen scrollback up to `history-limit`. |
| `1` (alternate screen) | `capture-pane -p -e -t=<pane_id>` | Just the current alternate-screen visible region (no `-S/-E` because alternate has no history). For belt-and-suspenders, capture both: run the main-screen variant first as historical context, emit it as one `session_replay`, then capture the alternate-screen visible region and emit it as a second `session_replay` chunk so xterm.js renders the TUI on top. |

Flag meanings:

- `-p` print to stdout (versus `-b` to a tmux buffer).
- `-e` preserve SGR/ANSI escape sequences. **Required** for xterm.js
  to render colors and cursor movement.
- `-S - -E -` full available scrollback. Bounded by `history-limit`.
  Only meaningful on the main screen.
- `-a` exists for "capture the *other* (currently inactive) screen";
  we don't need it — we always capture the active screen, which is
  the default behavior of `capture-pane`.
- **No `-J`**. `-J` joins wrapped lines into a single physical line
  with trailing spaces — useful for archival, breaks xterm.js's own
  reflow on resize. Use `-J` only for the "export transcript"
  feature if/when we add one.

Emit the capture as a **single replay payload** distinct from
`session_output` — xterm.js needs to know "this is a snapshot,
reset the buffer to it" rather than "append this to the live
stream". Use a separate `session_replay` Tauri event, or a
`{ kind: "replay" | "stream" }` discriminator on the existing
output event. This also handles the "switch tabs and the terminal
re-mounts" case cleanly.

Order on attach. Two FIFO open semantics drive this; getting either
backwards wedges the pane:

- `open(O_RDONLY)` on a FIFO **blocks** until a writer is attached,
  unless `O_NONBLOCK` is set. (POSIX `fifo(7)`.)
- `open(O_WRONLY)` on a FIFO **always blocks** until a reader is
  attached (`O_NONBLOCK` causes ENXIO instead of blocking — also not
  what we want).
- A small Linux-specific shortcut: `open(O_RDWR)` on a FIFO never
  blocks. Don't rely on this on macOS — POSIX leaves it undefined.

So:

1. `mkfifo` the per-session FIFO if it doesn't already exist.
2. **Open the FIFO reader-side first** in the runtime, with
   `O_NONBLOCK` set
   (`OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(…)`).
   Without `O_NONBLOCK`, this open blocks until tmux runs `cat` —
   but tmux can't be told to run `cat` until after we've registered
   the pipe, so we'd deadlock. With `O_NONBLOCK` the open returns
   immediately; subsequent reads return `EAGAIN` until data lands.
   Pair with `tokio::io::unix::AsyncFd` (or `tokio::fs::File` on a
   `dup`'d fd with `O_NONBLOCK` cleared) for the async read path.
3. **Start the forwarder task immediately**, draining the reader-side
   FD into an in-memory buffer (e.g. `tokio::sync::mpsc` of byte
   chunks). The buffer accumulates everything the pane writes from
   this point forward.
4. Close any stale pipe on this pane: `pipe-pane -t=<pane_id>` (no
   command). No-op for a fresh spawn; required for reattach where
   a prior Runner process may have registered a pipe to a now-dead
   FIFO. Then install the new writer: `pipe-pane -O -t=<pane_id>
   'cat >> fifo'` (no `-o`). The writer-side open returns
   immediately because step 2 has the FIFO open for read; the
   forwarder is already pulling bytes through.
5. `capture-pane -p -e -S - -E -` for the replay snapshot (with
   alternate-screen handling — see below).
6. Emit `session_replay` with the snapshot.
7. **Switch the forwarder from "buffer" to "emit"**: drain the
   buffered chunks as `session_output` events in order, then keep
   emitting future chunks live.

Why steps 3 and 7 are split: if the forwarder doesn't start until
*after* `session_replay` is emitted (the previous version of this
plan), the FIFO can fill during a slow `capture-pane`, blocking
`cat`'s `write()`, which back-pressures into the pane PTY buffer
(macOS/Linux: 16–64 KB), and once that fills the agent's `write()`
blocks and the pane wedges. Always be draining; choose what to do
with the bytes (buffer vs. emit) at a layer above the read.

There is still a tiny window between (5) and (7) where bytes the
agent writes are both in the replay snapshot and in the buffered
chunks — they appear twice. Acceptable for v1 (rare, a few ms of
duplicate cells); the upgrade is to mark a "buffer cursor" at step
4 and only flush bytes accumulated *after* the snapshot's terminal
state. Revisit if visible.

This must preserve the user-visible scrollback across:

- tab switch (replay from in-memory buffer, no tmux call needed)
- route switch (same)
- window reload (replay via fresh `capture-pane`)
- app restart while tmux server is still alive (replay via fresh
  `capture-pane`; live stream resumes via `pipe-pane`)

### Why not control mode (`tmux -CC`) yet

Control mode would replace the FIFO+pipe-pane plumbing with a
single long-lived `tmux -L runner -f <runner.conf> -CC new-session …` child emitting
`%output`, `%window-renamed`, `%exit`, etc. Tempting. Skipping it
for v1 because:

- `%output` chunks can split mid-multi-byte UTF-8 codepoint
  (wezterm#6769); the parser must buffer and decode lazily.
- Output bytes < 0x20 and `\` are octal-escaped (`\nnn`); the
  unescape pass is non-trivial.
- `pipe-pane` ships ~the same byte-stream semantics with less
  parser surface.

Revisit if FIFO management or per-pane forwarder tasks become a
bottleneck, and put it behind a feature flag so we can A/B in the
field.

## Step 7: Send input through tmux

**File:** `src-tauri/src/session/tmux.rs`

Use tmux APIs instead of writing to a frontend-owned PTY writer.

> **Anti-pattern: do not keep an attached `tmux attach` PTY per
> session and write user keystrokes to it.** That's claude-squad's
> design (`tmux.go:184,211-231`), and it's the proximate cause of
> their dropped-first-prompt bug (cs#266) and the 50ms stdin-nuking
> hack to scrub leaked `\033[?62c` sequences (`tmux.go:305-317`).
> Every input must go through tmux's own command surface.

### Command shapes

- **Multi-line / pasted prompt** (preserves bracketed-paste so the
  agent recognizes it as a paste rather than typed input):

  ```text
  printf %s "$payload" | tmux -L runner -f <runner.conf> load-buffer -b runner-<id> -
  tmux -L runner -f <runner.conf> paste-buffer -p -r -d -b runner-<id> -t=<pane_id>
  ```

  - `-p` wraps the payload in `\e[200~ … \e[201~` so claude-code /
    codex see a real paste event.
  - **`-r` (do not replace LF with CR).** This is critical and
    counter-intuitive: `paste-buffer` **defaults to LF→CR
    translation**, so a multi-line prompt without `-r` would have
    every `\n` rewritten to `\r` and the agent would see each line
    as a submitted message. `load-buffer` itself is verbatim — the
    LF→CR happens at *paste* time. With `-r`, the buffer's `\n`s
    arrive as `\n` on the pane and we send a separate `Enter` to
    submit the whole multi-line prompt as one message.
  - `-d` deletes the named buffer after pasting (no leak).
  - **Strip the trailing newline from the payload before
    `load-buffer`**; submit with a separate `send-keys Enter` so the
    agent's `\r`-bound submit handler fires.

- **Enter / control keys** (single keystrokes, no paste markers):

  ```text
  tmux -L runner -f <runner.conf> send-keys -t=<pane_id> Enter
  tmux -L runner -f <runner.conf> send-keys -t=<pane_id> C-c
  tmux -L runner -f <runner.conf> send-keys -t=<pane_id> -- Up
  ```

- **Literal byte stream** (terminal app input from xterm.js, e.g. raw
  arrow keys or vim-mode keystrokes from a passthrough terminal
  view):

  ```text
  tmux -L runner -f <runner.conf> send-keys -t=<pane_id> -l -- <payload>
  ```

  - `-l` disables key-name lookup so the payload is delivered as
    literal UTF-8.
  - `--` is mandatory before any user-derived payload — `send-keys`
    treats a trailing `;` as a command separator even inside quoted
    args (tmux#1849).

### Quoting / injection

Never go through `sh -c` for tmux invocations. Always
`tmux_cmd().arg("send-keys")…` (where `tmux_cmd()` already binds
`-L runner -f <runner.conf>`)
with one arg per token. User-controlled strings (prompts, env values)
go either over stdin (`load-buffer -`) or after `--` on the argv —
never interpolated into a shell string. Validate session/window/buffer
names against `^[A-Za-z0-9_-]+$` at the type boundary.

### Paste readiness

claude-squad's first-prompt drop (cs#266) is really an
agent-readiness race: the prompt arrives before claude-code finishes
its raw-mode TUI bind. We already have this problem on the current
portable-pty path; the resolution shipped in PR #59 was a 2500ms
fixed wait. Carry that forward as a pre-paste delay (`paste_after =
2500ms` configurable per runtime in `runtime.rs`), and revisit once
agents grow a "ready" signal we can detect.

System prompt delivery order, unchanged:

1. Prefer agent-native config/flags when available.
2. Use generated config/prompt files when the agent supports them.
3. Use `tmux paste-buffer -p` as the fallback for interactive prompts.

## Step 8: Resume and status reconciliation

**Files:**

- `src-tauri/src/session/manager.rs`
- `src-tauri/src/commands/session.rs`
- `src-tauri/src/commands/mission.rs`

### App-start config reconciliation

`-f <runner.conf>` is read **only when the tmux server starts**. With
`exit-empty off` keeping the server alive across Runner restarts, an
upgraded app finds an old server still running with the previous
config's options. New options in the shipped config (e.g. a bumped
`history-limit`, an added option) are not applied until the server
restarts — which we don't want, because that would tear down all
running panes.

On every app start, after writing the latest config to disk and
before spawning new panes:

```text
# Stamp the current config version into a server-scoped option:
tmux -L runner -f <runner.conf> set-option -g @runner_config_version "<sha>"

# If has-session shows the server was already running (pre-upgrade),
# reload the config explicitly:
tmux -L runner -f <runner.conf> source-file '<runner.conf>'
```

`source-file` re-runs the `set-option`s in the config against the
already-running server, so the running panes get the new values
where applicable (history-limit per-pane is sticky for existing
panes — only new panes pick up the new value, which is acceptable).

The stamped `@runner_config_version` (a hash of the config contents
or the app version string) lets a future Runner cheaply check
"already reconciled?" before doing the source-file dance:

```text
tmux -L runner -f <runner.conf> show-options -g -v @runner_config_version
```

If the value matches our expected hash, skip `source-file`. If it
differs (or is missing), reload and re-stamp.

### Session reconciliation

On app startup or session list:

- If a session row has `runtime = 'tmux'`, check whether its pane still exists:

```text
tmux -L runner -f <runner.conf> has-session -t=runner-<session_id>
tmux -L runner -f <runner.conf> list-panes -s -t=runner-<session_id> \
    -F '#{pane_id} #{pane_dead} #{pane_dead_status} #{pane_pid} #{pane_current_command}'
```

- `-s` makes `list-panes` treat the target as a session and
  enumerate every pane in every window of that session. Without
  `-s`, `list-panes` interprets the target as a *window* and a
  bare `runner-<session_id>` won't resolve to anything (or worse,
  resolves to the wrong window via tmux's lookup rules). The
  alternative is targeting the window directly:
  `-t=runner-<session_id>:main` without `-s`. Either works; `-s`
  is robust against window-name drift.

- If pane exists and `pane_dead = 0`, mark the session `running` and
  allow attach.
- If pane exists and `pane_dead = 1`, read `pane_dead_status` (the
  child's exit code, populated only because we set
  `remain-on-exit on` at server start — tmux#2552), capture final
  scrollback (`capture-pane -p -e -S - -E -`), then `kill-pane` and
  mark `stopped` (status 0) or `crashed` (non-zero).
- If `has-session` fails, mark stopped with a clear
  terminal-unavailable reason. This is the only path where exit code
  is unrecoverable — the user closed the tmux server out from under
  us.

Treat `list-panes` polling as the authoritative source of truth for
death detection. The `pane-died` hook is documented as inconsistent
when multiple panes exit simultaneously (tmux#2483); use it only as a
"poll soon" wakeup, not as the counter.

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
  - searches Homebrew paths even with stripped GUI PATH
  - returns clear missing-tmux error on Linux/macOS
  - returns `RuntimeUnavailable::TmuxRequiresUnix` on Windows
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
  - rejects unsafe names if any external string is used (regex
    `^[A-Za-z0-9_-]+$`, with explicit cases for `:`, `.`, `;`,
    whitespace, unicode)
- target helper:
  - emits `=runner-<id>` (exact-match form), never `runner-<id>` (the
    prefix-match footgun from cs#`tmux.go:459`)
- send-keys argv builder:
  - inserts `--` before any user-derived payload
  - rejects/escapes a payload ending in `;`

Integration tests, gated behind local tmux availability. **Each test
process must use both an isolated socket label and an isolated
config file**, mirroring production. Without `-f <temp.conf>`, tests
load `~/.tmux.conf` and the assertions that depend on
`remain-on-exit on` (most notably the `pane_dead_status` exit-code
test) become non-deterministic against developer machines.

- Test fixture: `tempfile`-backed `runner-test-<pid>.conf` containing
  the same options as the production config (`exit-empty off`,
  `remain-on-exit on`, etc.). Wire it into `tmux_cmd()` via a
  `(label, config_path)` constructor parameter so test code uses the
  exact same helper as production.
- Spawn a session running a simple shell command on a private socket
  (`-L runner-test-<pid>`) **with `-f <temp.conf>`** so the test
  neither touches the user's tmux server nor inherits their tmux
  config.
- Capture initial and later output via `pipe-pane`/`capture-pane`
  exactly as production does.
- Send input via `paste-buffer -p -r -d` and observe the output
  (round-trip multi-line UTF-8 with embedded `;`, `\n`, and a
  trailing newline that must stay literal — `-r` keeps LF→LF and
  the trailing `Enter` submits separately).
- Re-create `SessionManager` and attach to the same tmux session;
  assert pipe-pane reattach works (close stale pipe → install new
  pipe without `-o` → fresh FIFO).
- Stop a session and reconcile status: assert `pane_dead = 1` and
  `pane_dead_status` matches the child's exit code (requires
  `remain-on-exit on` — and it's now in the test config).
- Config reconciliation: start a session, externally rewrite the
  config (bump a stamped `@runner_config_version`), call the
  reconciler, assert `source-file` was issued and the new option
  visible via `show-options -g -v`.
- Post-test cleanup: `tmux -L runner-test-<pid> -f <temp.conf>
  kill-server` so the CI runner doesn't accumulate tmux daemons or
  config tempfiles.

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
