# Spawn-time first-prompt delivery

> Replaces the post-spawn paste → capture-verify → Enter dance for the
> *first user turn* (mission lead launch prompt + per-slot personas +
> direct-chat personas) with a positional CLI argument passed at
> process spawn. Eliminates the readback race entirely for the boot
> path. Targets the next patch release.

## Why

Issue #88 (also referenced in #50, partially mitigated by plan 0005):
after `mission_start`, the composed launch prompt / persona text
sometimes lands in the agent's input editor but is never submitted —
exactly **one** copy of the body sits in every slot's input, no Enter
fires, all slots stuck. The user has to press Enter in each slot to
unstick the mission.

Plan 0005 / PR #72 (`inject_paste_with_verify`) was meant to make the
paste→Enter handoff deterministic via a capture-pane readback loop.
It's strictly better than the prior `FIRST_PROMPT_DELAY = 2500ms`
blind wait, but it still has gaps:

- **Render-lag at verify-accept.** Even when the marker delta fires
  on attempt 1, claude-code's input editor may not be in a "ready to
  receive Enter as submit" state yet — multi-line mode triggered by
  the long paste, transitional alternate-screen, etc. PR #89's 120ms
  `submit_wait` was a speculative blind shot at this corner.
- **Verify rejection with no observability.** App stdout/stderr are
  wired to `/dev/null` under launchd, so the loop's `eprintln!`
  failure path is invisible in Console.app or `log show`. Diagnosis
  requires a terminal-relaunch dance or file-logging follow-up.
- **Three timer constants** in the critical path of every mission
  start: 1500ms `initial_wait`, 600ms `render_wait`, 800ms
  `between_attempts`. Each is a heuristic guess at agent-TUI
  readiness.

The architectural pivot: **don't paste the first user turn at all**.
Both agent CLIs accept a positional `[PROMPT]` argument that's read
as the first user turn during process init — *before* the TUI binds
raw-mode keypress handling, *before* any trust-folder dialog, *before*
the input editor exists. The race is eliminated by removing the
contestant.

```bash
# claude-code — verified
claude --permission-mode acceptEdits --model claude-opus-4-7 "<launch prompt body>"

# codex (codex-cli 0.130.0) — verified
codex --model gpt-5 "<launch prompt body>"
```

The codebase's earlier caveat in `router/runtime.rs:13-16` ("a
startup permission / approval dialog can swallow or misorder the
positional `[PROMPT]` argv") is stale. Modern codex has no startup
dialog by default; the `--ask-for-approval` modes apply to in-session
*command* approvals, not boot.

### Why this is a structural fix, not another heuristic

Today's flow:
```
spawn agent → wait 1.5s → paste body → wait 0.6s → capture-pane → match marker → send Enter
       ↑                ↑                ↑                ↑
     can't know        could be          marker match     submit might not
     when TUI binds    too short         is heuristic     fire on first try
```

Each arrow is a guess about agent-TUI internals. After this plan:
```
compose body → spawn agent with body as argv[N]
              ↑
            done. Agent reads its argv at init; the first user turn
            is committed before the TUI even renders.
```

The only remaining time-based wait is `initial_wait` between
process spawn and the FIRST tmux pane interaction — but that wait
exists for unrelated reasons (giving tmux time to attach the pane,
not the agent time to bind), and removing it is a separate concern.

## What we're not doing

- **Not removing `inject_paste_with_verify`.** Several mid-session
  paths still need it:
  - Resume `continue` injection (`schedule_continue_on_resume`) —
    delivered to an already-bound agent post-resume, no spawn-argv
    available.
  - Router-driven `--to` deliveries between slots (`handlers.rs`).
  - `ask_lead` / `human_said` cross-slot pushes (`Router::push_*`).
  - Bus-replay re-injections after `mission_attach`.
  These all hit an agent whose TUI is already bound; the readback
  race doesn't apply.

- **Not changing the agent CLI invocation surface.** No new flags,
  no new env vars. The positional was always there — we just
  weren't using it.

- **Not removing `Router::inject_and_submit_delayed`** as a public
  surface — its single caller (`handlers::mission_goal`) becomes a
  no-op for the launch-prompt body but the function stays for
  `fire_lead_launch_prompt`'s resume-fresh-fallback path (see Resume
  below).

- **Not touching `FIRST_PROMPT_CONFIG`** in this PR. With the
  first-turn paths off the readback, the production durations stop
  mattering for boot — but the path stays alive for mid-session
  paste verification, where the same durations are still correct.

- **Not fixing the `/dev/null` stderr problem here.** Worth doing,
  but separable; if this plan ships and the bug is gone, the
  observability gap matters less. Tracked as a follow-up.

## Approach

### Pre-spawn prompt composition

Today's composition site for the lead launch prompt is
`router/handlers::mission_goal` — the bus handler that fires when
the mission-start opening event lands. That's *post-spawn*: by the
time the handler runs, the lead's PTY exists and the router is
mounted.

Move composition to **before** the spawn loop in
`commands/mission::mission_start`:

```text
mission_start(input):
  open the mission row + the mission_goal event   ← unchanged
  compose lead_launch_prompt from goal + roster + brief
  compose per-slot persona for each non-lead       ← currently done
                                                     post-spawn by
                                                     `schedule_mission_first_prompt`
  spawn loop:
    for each slot:
      argv_extra = if slot.is_lead { launch_prompt } else { persona }
      spawn agent with [...existing args..., argv_extra]
  mount bus + router                               ← unchanged
```

The composition functions
(`router::prompt::compose_launch_prompt`,
`runner.system_prompt`) are pure and side-effect-free; pulling them
forward is mechanical.

### Spawn-argv plumbing

`session/launch.rs` builds the launch script (`exec '<cmd>' '<arg1>' '<arg2>' …`).
The composed body becomes the trailing positional after all
runtime/permission/model/effort flags:

```bash
exec 'claude' '--permission-mode' 'acceptEdits' '--model' 'claude-opus-4-7' '--effort' 'xhigh' '<composed body>'
```

The existing shell-quote helper in `render_launch_script` handles
embedded single quotes via the `'\''` chord; multi-line bodies are
just text — newlines pass through fine.

`SpawnArgs` (or whatever the launch-script input struct is called)
gains an optional `first_turn: Option<String>` field that, when
present, becomes the trailing positional in the rendered `exec` line.

### Direct chat path

`session_start_direct` already composes a persona-only body via
`compose_direct_chat_prompt` and currently injects it post-spawn via
`schedule_direct_first_prompt`. Two changes:

1. Pass the composed body into `SpawnArgs::first_turn`.
2. Skip the post-spawn `schedule_direct_first_prompt` call when
   `first_turn` was set at spawn — the prompt is already delivered.

### Router behavior change

`router::handlers::mission_goal` today calls
`Router::inject_and_submit_delayed(lead_handle, body, delay)` to
deliver the lead launch prompt via paste-verify. After this plan:

- `mission_goal`'s call becomes a no-op for **fresh** missions — the
  body was delivered at spawn time.
- For **resume**: the path depends on whether the agent's own resume
  mechanism (claude session UUID, codex rollout) restored the prior
  conversation. If yes → no re-injection needed. If no (fresh
  fallback) → we still need to deliver the prompt to a
  freshly-spawned-but-no-context agent. Two sub-options:
  - **Resume-fresh-fallback uses paste-verify**, same as today.
    `Router::fire_lead_launch_prompt` keeps its current shape.
  - **Resume-fresh-fallback restarts the agent process with the
    prompt in argv.** Cleaner, but adds a kill-respawn cycle on
    resume. Probably overkill.

The plan picks option 1: resume keeps paste-verify as the recovery
mechanism. Fresh-spawn (the dominant case) uses argv.

A small flag on the spawn call tells the router whether the body
was delivered at spawn — when set, the `mission_goal` handler skips
the inject. When unset (resume), it falls through to today's path.

### Per-slot persona delivery

`schedule_mission_first_prompt` (manager.rs) currently schedules
`inject_first_turn(persona)` against each non-lead slot's session
post-spawn. After this plan:

- Persona is composed before spawn and passed as `first_turn` in
  `SpawnArgs`.
- `schedule_mission_first_prompt` becomes a no-op when the persona
  was spawn-delivered (signal via a `persona_delivered: bool` flag
  on the spawn input, propagated to the manager's per-session state).
- On resume of a non-lead, the agent's own session resume restores
  the persona context; no re-injection needed for either runtime.

### ARG_MAX guard

macOS `ARG_MAX` is ~256KB. Mission prompts are typically 1-5KB.
Add a defensive cap:

```rust
const SPAWN_ARGV_PROMPT_MAX_BYTES: usize = 32 * 1024; // 32KB

if first_turn.len() > SPAWN_ARGV_PROMPT_MAX_BYTES {
    log + fall back to post-spawn paste-verify for this slot
}
```

32KB leaves an 8x safety margin against the OS cap and trips well
before any realistic mission prompt. The fallback path is the
existing `schedule_mission_first_prompt` / `inject_paste_with_verify`
machinery — unchanged.

### Trust-folder dialog (claude-code)

claude-code on first launch in a never-trusted directory shows
"Trust this folder?" before binding the input editor. With argv-based
delivery, the positional sits queued in claude's process state until
the human dismisses the dialog; once dismissed, claude reads it as
the first user turn. UX cost: one click on first launch in a new
directory. Determinism intact (no race).

This is *strictly better* than today: today the dialog can swallow
the paste outright, leaving the agent with no context. Argv survives
the dialog.

## Touch surface

### `src-tauri/src/router/prompt.rs`
No code change. Composition functions stay pure; new callers pull
them forward.

### `src-tauri/src/router/runtime.rs`
- Drop the stale "approval dialog swallows the positional" comment
  block on `system_prompt_args` (lines 1-30).
- Add a new helper `first_turn_argv(body: &str) -> Vec<String>` that
  returns `vec![body.to_string()]` (or empty if body is blank). Both
  runtimes use the same shape; future runtimes with different
  positional semantics override.
- Update `extra_args_for` (or the equivalent argv composition site)
  to append `first_turn_argv` last.

### `src-tauri/src/session/launch.rs`
- `SpawnArgs` (the struct passed to `render_launch_script`) gains
  `first_turn: Option<String>`.
- `render_launch_script` appends the first_turn as a final
  positional in the `exec` line, single-quoted via the existing
  helper, only when `Some(non_empty)`.
- One new unit test: launch script with a multi-line first_turn
  renders the positional correctly escaped.

### `src-tauri/src/commands/mission.rs`
- `mission_start`: compose lead launch prompt and per-slot personas
  before the spawn loop. Pass each slot's body via `SpawnArgs`.
- Set the `persona_delivered_at_spawn` flag on per-session state so
  `schedule_mission_first_prompt` / `mission_goal` know to skip
  post-spawn injection.

### `src-tauri/src/commands/session.rs`
- `session_start_direct`: pass the composed persona via `SpawnArgs`.
- Skip `schedule_direct_first_prompt` when `first_turn` was set.

### `src-tauri/src/session/manager.rs`
- `inject_first_turn` and `schedule_*_first_prompt` callers gain a
  guard: skip when the prompt was spawn-delivered.
- Keep the functions themselves alive for the ARG_MAX fallback and
  for the resume-fresh-fallback (router's
  `fire_lead_launch_prompt`).

### `src-tauri/src/router/handlers.rs`
- `mission_goal`: skip `inject_and_submit_delayed` for the
  launch-prompt body when the lead's `persona_delivered_at_spawn`
  flag is set.

### `src-tauri/src/router/mod.rs`
- `Router::inject_and_submit_delayed` stays. Used by
  `fire_lead_launch_prompt` (resume-fresh-fallback) and by future
  resume paths.
- `fire_lead_launch_prompt` is unchanged — that path only fires on
  resume-fresh-fallback, and we keep paste-verify there.

## Risks

- **ARG_MAX edge cases on weird systems.** macOS is fine. Linux
  varies (per `getconf ARG_MAX`); typical 2MB. The 32KB cap is
  defensive. Embedded environments with smaller limits would trip
  the fallback path — acceptable.

- **Shell-quoting bugs.** The existing
  `render_launch_script` quoter handles single quotes; multi-line
  bodies pass through. But pathological inputs (e.g. control chars
  in the persona, or NUL bytes from a corrupted DB row) could break.
  Mitigation: a sanity check on the body before quoting — reject
  NUL bytes, normalize line endings to `\n`.

- **Argv visible in `ps`.** The composed body shows up in process
  listings (`ps aux | grep claude`). For Runner this is fine — the
  user owns both processes and the body isn't a secret. Worth
  documenting; not a blocker.

- **agent CLI version drift.** Older `codex` versions DID have the
  startup-dialog problem the codebase's stale comment described.
  If a user pins an old codex, argv delivery degrades to "prompt
  lost". Mitigation: the ARG_MAX-fallback path catches this if we
  detect non-delivery via... well, we don't have that signal today.
  Probably accept the edge case; the v0.130.0+ codex CLIs are the
  shipping baseline.

- **Trust-folder dialog UX**. Acknowledged above — strictly better
  than today.

## Tests

### Existing tests that need updates

- `router/tests.rs::lead_launch_prompt_routes_through_verified_paste`
  (added by PR #72) — assertion needs to flip: under the new shape,
  the lead's mission_goal handler must NOT call
  `inject_paste_with_verify` for the launch-prompt body when the
  body was spawn-delivered. New assertion shape:
  `paste_pushes_for("S-LEAD").is_empty()`.

- Manager tests that exercise `inject_first_turn` via
  `schedule_*_first_prompt` need to pass a fixture where the
  persona-delivered-at-spawn flag is `false` (preserving the legacy
  behavior path for resume / ARG_MAX fallback).

### New tests

1. **`launch_script_renders_first_turn_positional`** (launch.rs) —
   `SpawnArgs { first_turn: Some(multi_line_body), .. }` renders
   `exec 'claude' '<flags>' '<body with embedded newline and single quote>'\n`
   with the body correctly single-quoted.

2. **`mission_start_passes_lead_prompt_via_argv`** (mission.rs) —
   start a mission with a fake runtime; assert the lead's spawn
   call received `first_turn = compose_launch_prompt(...)`.

3. **`mission_start_passes_worker_persona_via_argv`** (mission.rs) —
   same, for a non-lead slot's persona.

4. **`direct_chat_passes_persona_via_argv`** (session.rs) —
   `session_start_direct` calls spawn with `first_turn = persona`.

5. **`spawn_argv_too_long_falls_back_to_paste`** (mission.rs or
   manager.rs) — a body > 32KB triggers the fallback path and
   `inject_paste_with_verify` is called instead.

6. **`mission_goal_handler_no_op_when_prompt_spawn_delivered`**
   (router) — bus replay of `mission_goal` against a fresh mission
   does NOT call `inject_paste_with_verify` for the lead's body.

7. **`mission_goal_handler_falls_back_for_resume_fresh`** (router) —
   when the per-session flag is unset (resume-fresh-fallback path),
   `mission_goal` still calls `inject_paste_with_verify`. Guards
   the resume path against regression.

## Rollout

Single PR off `feat/spawn-time-prompt-delivery`, target a patch
release after merge (v0.1.7 if no other changes pile up, otherwise
bundle).

Manual smoke:
- (a) Start the default Build squad mission in a never-trusted
  directory — confirm claude-code's trust dialog appears, accept
  it, observe the launch prompt arrives and the lead starts
  working immediately.
- (b) Start a codex-lead mission with a long preamble + brief +
  goal (≥2KB total) — confirm codex starts working immediately
  with no manual Enter.
- (c) Resume a previously-stopped mission — confirm the
  resume-fresh-fallback path still works for sessions where
  claude's resume context is missing.
- (d) Start a direct chat with a non-trivial persona — confirm
  the persona lands as the first user turn with no manual Enter.

## Out of scope follow-ups

- **File-based logging.** `$APPDATA/runner/logs/runner.log` so the
  next regression-class bug is observable without a terminal
  relaunch. Independent value; ship separately.
- **Pre-spawn body sanitization.** Currently we trust composer
  output; a sanitize step (NUL-strip, control-char filter, line-end
  normalize) would defend against weird DB-stored personas.
- **Remove `FIRST_PROMPT_CONFIG`'s production durations from the
  manager.** With first-turn paths off the readback, the production
  values stop mattering for boot — but the path stays alive for
  resume-fresh-fallback and ARG_MAX-fallback. Could be revisited
  once those become rare.
- **Re-route resume-fresh-fallback to argv via kill-respawn.**
  Cleaner determinism story for resume, at the cost of a
  kill-respawn cycle. Tracked as a separate explore.
