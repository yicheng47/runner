# First-prompt readback verification

> Post-tmux-cutover bugfix. Replaces the `FIRST_PROMPT_DELAY = 2500ms`
> blind-wait heuristic in `inject_first_turn` with a paste → capture-pane →
> verify → retry → Enter loop, so persona injection lands deterministically
> regardless of how long the agent's TUI takes to bind raw-mode input.
>
> Companion to `0004-tmux-session-runtime.md` (the runtime that makes this
> feasible — `capture-pane` is what unlocks readback). Targets the v0.1.4
> release.

## Why

Issue #50 (also reported live by the user post v0.1.3): after a fresh
chat spawn, the system_prompt sometimes fails to land in the agent's
input box. The 2500ms `FIRST_PROMPT_DELAY` is supposed to cover the
window between `tmux new-session` and the agent's TUI binding raw-mode
keypress reading; in practice that window varies with CPU contention
(parallel spawns, cold disk, mid-update Node startup) and 2500ms isn't
always enough.

When the paste lands too early it gets eaten by whichever pre-input
handler is on screen — Claude Code's "trust this folder" dialog, the
banner animation phase, or just an unbound stdin during Node init.
Once eaten, no recovery: we send Enter on a 120ms timer regardless,
and the agent boots vanilla with no persona context. The user has to
notice and re-inject manually.

`claude-squad` (cs#266) ships with the same bug — their `SendPrompt`
has a 100ms gap between body and Enter and no readiness wait at all.
We're already doing better with 2500ms; the goal here is to do better
than "better".

The tmux runtime is a hard prerequisite: `capture-pane -p -e` is the
primitive that makes "did our paste actually land" answerable. The
prior `portable-pty` path could have stitched together output-buffer
substring matching, but with much weaker guarantees (frontend-coupled
buffer, no alternate-screen awareness). Now that we own the pane via
tmux, the readback loop is straightforward.

### Three injection paths share the race

`SessionManager::inject_first_turn` is one of three places where Runner
hands a first user turn to a freshly-spawned agent on the same 2500ms
budget. All three race the agent's TUI bind, all three need the fix:

| Path | Caller | Today's primitive | Used for |
|---|---|---|---|
| `inject_first_turn` (manager.rs:1897) | `schedule_direct_first_prompt`, `schedule_mission_first_prompt` (non-lead workers) | `inject_paste` — `tmux paste-buffer -p -r -d` + 120ms + `send_key("Enter")` | Direct-chat persona; non-lead mission worker preamble |
| `Router::inject_and_submit_delayed` (router/mod.rs:425) | `handlers::mission_goal` (mission boot), `Router::fire_lead_launch_prompt` (resume-fresh-fallback) | `StdinInjector::inject` — raw bytes via `tmux send-keys -l`; trailing `b"\r"` after 80ms | Mission lead launch prompt (system_prompt + bus protocol + mission goal) |
| `schedule_continue_on_resume` (manager.rs:1953) | claude-code resume path | `inject_paste` of `b"continue"` | Auto-`continue` after a successful resume |

The lead-launch-prompt path is the most fragile of the three: it
delivers a multi-line prompt (preamble + roster + brief) as
*keystrokes* (`send-keys -l`, character-by-character) rather than as a
bracketed paste, so even when the readiness window does hold,
embedded `\n`s can render as line breaks the TUI never wraps the same
way a real paste would. Plus the readiness race itself.

A v0.1.4 fix that only patched `inject_first_turn` would leave Codex
mission leads (always on this path because their argv `[PROMPT]`
positional gets swallowed by the approval dialog —
`router/runtime.rs:13-16`) and Claude Code mission leads still
exposed. The plan covers all three.

## What we're not doing

- **Not bumping the timer.** 4000ms or 5000ms would mask the bug for
  most cases but adds wasted UX latency for fast spawns and still
  fails under contention. The retry loop is deterministic — fast
  agents return on attempt 1 (faster than today, since the initial
  wait drops to 1500ms); slow agents get retried.
- **Not changing the paste primitive.** `tmux paste-buffer -p -r -d`
  stays the bracketed-paste delivery mechanism. Only the
  *verification* layer changes.
- **Not surfacing failures to the UI.** A first-prompt give-up after
  the max retry budget logs to stderr; the user can re-paste from the
  composer manually. UI surfacing is a follow-up if it turns out to
  matter.
- **Not refactoring `inject_paste`.** The general-purpose paste path
  (composer submits, mission `--to` deliveries) still uses the
  blind-wait shape — those land into a known-ready agent and don't
  need readback. Only the *first-turn* injection (and the post-resume
  `continue` injection, which has the same race) gets the new path.
- **Not implementing this on `InertRuntime` / non-tmux runtimes.**
  `InertRuntime` returns an error from `capture_visible`; the verify
  loop treats capture errors the same as "needle not visible" and
  retries. Non-tmux runtimes don't exist yet in production.

## Approach

A new manager-side method `inject_paste_with_verify` runs the loop:

```text
sleep(initial_wait)          // first-cut readiness wait, smaller than today
loop attempt = 0..max_attempts:
    if session no longer in self.sessions: bail (user killed it)
    runtime.paste(session, body)
    sleep(render_wait)        // give tmux + agent TUI time to render
    snapshot = runtime.capture_visible(session)
    if pane_acknowledged_paste(snapshot, marker(body)):
        runtime.send_key(session, "Enter")
        return Ok
    sleep(between_attempts)
return Err("persona not visible after N attempts")
```

`pane_acknowledged_paste` matches **either** of:

1. The body marker (first ≤32 chars of trimmed body, line-bounded)
   appears verbatim in the pane after CSI escapes are stripped. Covers
   short personas (Claude Code shows the paste verbatim under its
   wrap threshold) and the codex TUI (always verbatim).
2. The literal substring `Pasted text` appears in the pane. Covers
   long personas where Claude Code substitutes a `[Pasted text #N
   +M lines]` placeholder for the actual content.

Either match means the paste landed in the input box. The placeholder
match is Claude-Code-specific by design — codex is the other
production runtime and it always shows verbatim, so the body-marker
match covers it.

### Timing budget

| Phase | Production | Test (`cfg(test)`) |
|---|---|---|
| `initial_wait` | 1500ms | 0 |
| `render_wait` (per attempt) | 600ms | 0 |
| `between_attempts` | 800ms | 0 |
| `max_attempts` | 4 | 4 |

- **Best case** (fast spawn, paste landed first try): 1500 + 600 ≈
  2100ms — *faster* than today's 2500ms blind wait.
- **Worst case** (4 retries before giving up): 1500 + 4×(600+800) =
  7100ms. Worse than today only if today would also have failed; in
  exchange the persona actually lands.

Test mode collapses every duration to zero so unit tests stay
synchronous (matches the existing `FIRST_PROMPT_DELAY = ZERO` shape).

### Idempotency under retry

The retry loop accepts a small risk of duplicate paste: if the agent
*just* started rendering when we capture-pane and our marker isn't
visible yet, we paste again, the agent now has the body twice. In
practice Claude Code merges the two into a single
`[Pasted text #1 +N lines] [Pasted text #2 +M lines]` — the user
sees both segments delivered as one prompt. Mildly weird, not
broken, and rare in the timing budgets above (600ms render wait is
generous).

The alternative (capture *before* paste, paste only when pane
"looks ready") would need a runtime-specific readiness signal.
`Pasted text` indicates a successful paste; there's no symmetric
"agent is now ready" indicator across runtimes. Post-paste
verification is the simpler primitive.

## Touch surface

### `src-tauri/src/session/runtime.rs`

Add one method to `SessionRuntime`:

```rust
/// Snapshot of the pane's currently-rendered visible region with
/// SGR escapes preserved (`tmux capture-pane -p -e`). Used by the
/// manager's first-prompt readback loop to verify a paste actually
/// landed in the agent's input box before sending Enter.
fn capture_visible(&self, session: &RuntimeSession) -> RuntimeResult<Vec<u8>>;
```

### `src-tauri/src/session/tmux_runtime.rs`

Implement `capture_visible` by promoting the existing private
`capture_visible_region` helper to a public trait method (it's
already called by `attach_streaming` for fresh-spawn snapshots, so
the shape is exactly what we need).

### `src-tauri/src/session/manager.rs`

Five changes:

1. Replace the `FIRST_PROMPT_DELAY` constant with a `FirstPromptConfig`
   struct holding all four durations + max_attempts. Production /
   test variants gated on `cfg(test)`.
2. Add `inject_paste_with_verify(session_id, body, config)` method on
   `SessionManager`. Pure orchestration — runtime calls only.
3. Add the helpers `paste_marker(body)` and
   `pane_acknowledged_paste(snapshot, marker)` (the latter strips
   CSI escapes from the snapshot before substring matching, reusing
   the strip logic shape from `is_visually_blank`).
4. Switch `inject_first_turn` from `inject_paste` blind-wait to
   `inject_paste_with_verify`. Same `cfg(test)` inline-vs-thread
   split as today.
5. Switch `schedule_continue_on_resume` to `inject_paste_with_verify`
   too — same race, same fix. The marker for the `continue` body is
   just the literal string "continue" (covered by the body-marker
   path; `Pasted text` doesn't fire for an 8-char paste).

### `src-tauri/src/router/mod.rs`

Extend the `StdinInjector` trait so the router can route lead launch
prompts through the verified path without bypassing the seam the
test-side fake injector relies on:

```rust
pub trait StdinInjector: Send + Sync + 'static {
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    /// Verified paste-and-submit. Used by `inject_and_submit_delayed`
    /// for mission lead launch prompts. Performs the readback retry
    /// loop internally; on success the agent has the body in its
    /// input buffer AND has received Enter.
    fn inject_paste_with_verify(&self, session_id: &str, body: &[u8]) -> Result<()>;
}

impl StdinInjector for SessionManager {
    fn inject(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        SessionManager::inject_stdin(self, session_id, bytes)
    }
    fn inject_paste_with_verify(&self, session_id: &str, body: &[u8]) -> Result<()> {
        SessionManager::inject_paste_with_verify(
            self,
            session_id,
            body,
            FIRST_PROMPT_CONFIG, // exposed pub(crate) from session::manager
        )
    }
}
```

Update `Router::inject_and_submit_delayed` to drop the
keystroke-then-`\r` chord on the non-zero-delay path and use
`injector.inject_paste_with_verify(session_id, &body)` instead. The
verified path delivers the body as a bracketed paste (so multi-line
launch prompts render correctly inside the input box, not as
character-by-character keystrokes that wrap differently) and
internally handles the Enter once it's confirmed the body landed.

The zero-delay path (used by router unit tests with
`LEAD_LAUNCH_PROMPT_DELAY = ZERO`) keeps the existing inline-`inject`
behavior — those tests assert byte-write counts against a fake
injector and shouldn't see a different call shape.

`inject_and_submit` (the synchronous, non-delayed path used by
`human_said` and `ask_lead`) stays on raw byte injection. Those
target *running* agents that already have their TUI bound, so the
readback isn't needed; using a paste there would just add latency
and double-encode user-typed text.

### Test stubs

- `InertRuntime` (in-test stand-in for tests that never reach the
  runtime): return `Err` from `capture_visible`, matching its other
  methods. The verify loop treats capture errors as "not seen" and
  retries — the eventual give-up path is exercised this way.
- `FakeRuntime`: add `pane_content: Mutex<Vec<u8>>` (canned snapshot
  body) and `acknowledge_after: Mutex<usize>` (number of pastes
  before `capture_visible` reveals the canned content). Default
  `acknowledge_after = 0` so existing tests are unaffected.
- The router's test-side fake `StdinInjector` (in `router/tests.rs`)
  needs the new `inject_paste_with_verify` method too. For
  zero-delay router unit tests this routes through the same
  recording shape as `inject` (one byte-write captured), so existing
  push-count assertions stay valid.

### Tests

Three new manager-level unit tests against `FakeRuntime`:

1. **`first_prompt_landed_first_try`** —
   `acknowledge_after = 0`, `pane_content = persona`. Inject. Assert
   1 paste captured, 1 `Key("Enter")` captured, no `Err` log.
2. **`first_prompt_landed_after_retry`** —
   `acknowledge_after = 2`, `pane_content = persona`. Inject. Assert
   2 pastes captured (the loop retried once), 1 Enter captured.
3. **`first_prompt_gives_up_after_max_attempts`** —
   `acknowledge_after = 999`. Inject. Assert `max_attempts` pastes
   captured, **zero** Enters captured.

Plus one router-level unit test:

4. **`lead_launch_prompt_routes_through_verified_paste`** —
   configure `LEAD_LAUNCH_PROMPT_DELAY` non-zero, fire
   `mission_goal`, assert the fake injector's
   `inject_paste_with_verify` was called once with the composed
   launch-prompt body (and that the legacy `inject` path was *not*
   called for that body). Guards against future churn re-introducing
   the keystroke path.

Existing first-prompt tests stay green because they don't configure
`acknowledge_after` — the default-zero value means
`capture_visible` returns the canned `pane_content` immediately, and
the loop terminates on attempt 1 with the same call shape as today
plus one `capture_visible` invocation per spawn.

## Risks

- **`Pasted text` match is Claude-Code-specific.** If a future
  Claude Code version renames the placeholder, large pastes start
  failing the body-marker path AND failing the placeholder path —
  retry loop exhausts attempts, persona doesn't land. Mitigation:
  the substring check is a single literal in `pane_acknowledged_paste`,
  trivial to update. Worst case we revert to the blind wait — same
  failure mode as today, no regression.
- **Body marker false-positive on agent prior output.** The first 32
  chars of a persona could coincidentally match unrelated pane
  content (e.g. agent's own banner). For "You are an…" personas this
  is unlikely; for very generic openings it could fire. Mitigation:
  the marker comes from the *first non-whitespace line* of the body,
  capped at 32 chars — distinctive enough in practice. If it
  becomes a real issue we'd extend the match to require the marker
  appear *after* a known input-box delimiter.
- **Capture-pane cost.** ~1ms per call on macOS in benchmarks; 4
  calls per spawn worst case. Negligible.

## Out of scope follow-ups

- Surfacing a "persona not delivered" toast to the UI when
  `inject_paste_with_verify` exhausts its budget. Today this only
  hits stderr; if it turns out to fire often enough that users
  notice silently, we'd add a UI signal. Punted until we have signal.
- Generalizing the readback to mission `--to` deliveries. Those
  paste into a known-ready agent (the spawn already settled), so
  the race we're fixing here doesn't apply. Could still be
  defensive armor against the next class of stuck-paste bugs;
  punted for now.
- Extracting `paste_marker` / `pane_acknowledged_paste` into a
  shared helper module if a third caller appears. Currently they're
  internal to the first-prompt path.

## Rollout

Single PR off `fix/first-prompt-readback`, target v0.1.4. CI runs
the full unit suite (so the three new tests + 197 existing pass);
manual smoke is "spawn three direct chats with a persona in
parallel, confirm all three show the persona as the first user turn
in each agent".
