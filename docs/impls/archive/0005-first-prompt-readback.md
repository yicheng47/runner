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
  loop's capture-error policy (zero baselines on `before` failure,
  skip-the-attempt on `after` failure — see "Capture error
  handling" under Approach) covers it. Non-tmux runtimes don't
  exist yet in production.

## Approach

A new manager-side method `inject_paste_with_verify` runs the loop:

```text
sleep(initial_wait)              // first-cut readiness wait, smaller than today
before = runtime.capture_visible(session).unwrap_or_default()  // see error policy
(head_marker, tail_marker) = paste_markers(body)
before_stripped = strip_ansi(before)
before_head_count = count_substr(before_stripped, head_marker)
before_tail_count = count_substr(before_stripped, tail_marker)
before_placeholder_count = count_substr(before_stripped, b"Pasted text")
loop attempt = 0..max_attempts:
    if session no longer in self.sessions: bail (user killed it)
    runtime.paste(session, body)
    sleep(render_wait)           // give tmux + agent TUI time to render
    after = match runtime.capture_visible(session) {
        Ok(b) => b,
        Err(_) => { sleep(between_attempts); continue }   // skip this attempt's check
    };
    after_stripped = strip_ansi(after)
    accept = count_substr(after_stripped, head_marker) > before_head_count
          || count_substr(after_stripped, tail_marker) > before_tail_count
          || (body.len() >= PLACEHOLDER_MIN_BODY_LEN
              && count_substr(after_stripped, b"Pasted text") > before_placeholder_count)
    if accept:
        runtime.send_key(session, "Enter")
        return Ok
    sleep(between_attempts)
return Err("paste not visible after N attempts")
```

All three acceptance signals use a **count delta** between `before`
(one capture, taken once before the loop) and `after` (one capture
per attempt). Any one delta increasing by ≥ 1 means our paste
landed:

1. **Head-marker count delta.** The first ≤32 chars of the trimmed
   body's first non-empty line — `head_marker` — counted as a
   substring in the CSI-stripped snapshot. Covers short pastes
   (Claude Code shows them verbatim) and any TUI where the input
   editor scrolls to keep the *start* of the paste visible. For
   8-byte bodies like `"continue"` this is the only signal that
   fires; using the delta (rather than just "marker is now
   visible") handles the resume case where the prior conversation
   already contains the marker text — a `continue` resume against
   a transcript mentioning the word "continue" still verifies
   correctly, because the paste pushes the count from N to N+1.
2. **Tail-marker count delta.** The last ≤32 chars of trimmed body's
   last non-empty line — `tail_marker`. Covers TUIs that scroll the
   input editor to keep the *cursor* (and therefore the *end* of the
   paste) visible — codex's chat composer is the known case. A long
   codex mission prompt (preamble + roster + brief, often >2KB)
   will not show its first line in the visible region; the
   editor's bottom rows show the trailing lines instead. Tail-marker
   delta picks that up. For short bodies where head and tail
   overlap, both signals fire on the same paste — equivalent to
   "either marker matched" with no double-count concern.
3. **Placeholder count delta.** Occurrences of the literal
   `Pasted text` substring in the CSI-stripped snapshot. Consulted
   only when `body.len() >= PLACEHOLDER_MIN_BODY_LEN` (64 bytes —
   well below Claude Code's actual wrap threshold of ~200, but
   above any reasonable short-prompt zone where the placeholder
   shouldn't fire). Skipping for short bodies plus using a delta
   both defend against the resume false-ack case: a resumed pane
   that already shows `[Pasted text #5 ...]` from prior turns has
   `before_placeholder_count = 1`; a failed `continue` paste
   leaves `after_placeholder_count = 1`; delta = 0 → reject and
   retry.

The capture-before is taken **once**, before the retry loop. Each
retry compares against that fixed baseline, so duplicate pastes that
actually do land (a paste-then-render race resulting in two
placeholders) still verify on the first delta-positive attempt.

#### Capture error handling

The `capture_visible` call can fail (tmux daemon flaked, pane id no
longer valid, runtime returns `Err` — `InertRuntime` always errors
this way for unit-test paths that never reach a real runtime). Two
distinct cases:

- **Baseline (`before`) capture fails.** Treat as zero counts and
  proceed. Concretely: `unwrap_or_default()` returns an empty
  `Vec<u8>`, and `count_substr(b"", _) == 0` — so the baselines
  become 0 across the board. The first post-paste capture will
  count whatever appears in the pane against zero; if that pane
  contains the marker text from prior content, we'd false-ack on
  attempt 1. The alternative (abort the verify and fall back to
  raw send-keys) regresses every baseline-capture failure to the
  pre-fix state, which is worse than a rare false-ack. We log the
  failure to stderr so operators can correlate. In practice
  baseline-capture failures are transient — tmux is alive enough
  to spawn but momentarily unresponsive — and resolve by the next
  attempt.
- **Per-attempt (`after`) capture fails.** Skip the check for
  *this* attempt and continue the loop after the
  `between_attempts` sleep. Don't retry the capture inline; the
  loop's natural cadence already rate-limits.

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

Update `Router::inject_and_submit_delayed` so **both** branches route
through `injector.inject_paste_with_verify(session_id, &body)`,
regardless of whether the `delay` parameter is zero or non-zero. The
delay parameter now controls only "spawn a thread vs run inline";
the *routing* decision is settled by the method name (the
`_delayed` suffix means launch-prompt-class injection, which always
needs the verified path):

- **Non-zero delay** (production): spawn a thread, immediately call
  `inject_paste_with_verify` (no outer `sleep(delay)` — see Risks /
  budget below).
- **Zero delay** (tests): call `inject_paste_with_verify` inline.
  Under `cfg(test)` the verified path's own durations are all zero,
  so this stays a synchronous millisecond no-op.

Drop the trailing `\r` chord entirely from the delayed path — the
verified primitive sends Enter internally once it confirms the body
landed.

`inject_and_submit` (the synchronous, non-delayed path used by
`human_said` and `ask_lead`) stays on raw byte injection. Those
target *running* agents that already have their TUI bound, so the
readback isn't needed; using a paste there would just add latency
and double-encode user-typed text.

#### Readiness budget — verified path owns it

The lead launch prompt today wraps the legacy raw-byte path in an
outer `LEAD_LAUNCH_PROMPT_DELAY` (2500ms). The verified path has
its own `initial_wait` (1500ms) plus per-attempt `render_wait`
(600ms). **Stacking both would mean ~4600ms before the first
verify, which is worse UX than today's blind 2500ms.** To avoid the
stack:

- The verified path is the sole owner of pre-paste readiness
  waiting. `inject_paste_with_verify` does its own `initial_wait`
  internally; callers must not sleep before calling it.
- `inject_and_submit_delayed`'s thread *immediately* calls
  `inject_paste_with_verify` — no outer `sleep`. The `delay`
  parameter is preserved as a no-op argument for API stability with
  the existing call sites (`handlers::mission_goal` and
  `Router::fire_lead_launch_prompt`); the constant
  `LEAD_LAUNCH_PROMPT_DELAY` itself can be removed once the
  legacy raw-byte branch is gone, but keeping it as a documented
  vestigial argument for one release simplifies the diff.
  Optionally we delete it in the same commit — judgment call at
  implementation time, plan accepts either.
- Net production cost on lead path: 1500ms initial + 600ms render =
  ~2100ms best case (faster than today's 2500ms blind), 7100ms
  worst case (vs today's 2500ms-then-fail).

### Test stubs

- `InertRuntime` (in-test stand-in for tests that never reach the
  runtime): return `Err` from `capture_visible`, matching its other
  methods. The verify loop treats capture errors as "not seen" and
  retries — the eventual give-up path is exercised this way.
- `FakeRuntime`: capture the new "before-then-after" probe shape
  with three fields: `pane_pre_paste: Mutex<Vec<u8>>` (what
  `capture_visible` returns before the canned acknowledge
  threshold), `pane_post_paste: Mutex<Vec<u8>>` (what it returns
  after), and `acknowledge_after: Mutex<usize>` (paste count
  threshold). Defaults: pre-paste empty, post-paste = persona body,
  threshold 0 — so any paste is acknowledged on first capture-after,
  matching existing test expectations. Tests that exercise
  retry/give-up override the threshold; the false-ack-on-resume test
  sets `pane_pre_paste = pane_post_paste = stale-placeholder` and
  asserts the delta check rejects.
- `RecordingInjector` in `router/tests.rs`: implements the new
  `StdinInjector::inject_paste_with_verify` method. To preserve
  existing push-count assertions (e.g. `lead_pushes.len() == 1`
  in `tests.rs:230`), tag each recorded entry with an `InjectKind`
  enum (`Raw` / `PasteVerified`) and have the existing
  `pushes_for` / `all_pushes` helpers continue projecting the kind
  away — so existing assertions on body content stay valid as-is.
  Add `paste_pushes_for(session_id)` filter so the new
  lead-routing test can assert which path was used.

### Existing test impact

- Router tests that currently assert on lead-launch-prompt body
  content via `pushes_for("S-LEAD")` (e.g. `tests.rs:230-232`,
  `tests.rs:273-275`, `tests.rs:296`) keep their content
  assertions unchanged — the body still goes through the recorder,
  just tagged `PasteVerified` now. The trailing `\r` push that
  exists in the *non-zero-delay production* path doesn't appear in
  these tests today (they run under `cfg(test)` zero-delay which
  skips the `\r` chord), so the chord removal is a no-op for the
  test suite.
- Manager-level first-prompt tests (the existing ones, pre-this-PR)
  stay green because the default `acknowledge_after = 0` +
  non-empty `pane_post_paste` means the verified loop terminates
  on attempt 1 with the same paste + Enter call shape — plus one
  capture_visible-pre and one capture_visible-post per spawn.

### Tests

Five new unit tests:

1. **`first_prompt_landed_first_try`** (manager) —
   `acknowledge_after = 0`, `pane_post_paste` contains the persona
   body. Inject. Assert 1 paste captured, 1 `Key("Enter")` captured,
   no `Err` log.
2. **`first_prompt_landed_after_retry`** (manager) —
   `acknowledge_after = 2`, `pane_post_paste` contains the persona
   body, `pane_pre_paste` empty. Inject. Assert 2 pastes captured
   (loop retried once), 1 Enter captured.
3. **`first_prompt_gives_up_after_max_attempts`** (manager) —
   `acknowledge_after = 999`. Inject. Assert `max_attempts` pastes
   captured, **zero** Enters captured.
4. **`continue_resume_rejects_stale_placeholder`** (manager) —
   `body = b"continue"`, `pane_pre_paste = pane_post_paste =
   "[Pasted text #5 +20 lines]"` (resume showing prior placeholder),
   `acknowledge_after = 0`. Inject. Assert `max_attempts` pastes,
   zero Enters — the placeholder count delta is 0 (both before and
   after see one placeholder), `body.len() = 8 < 64` so the
   placeholder check wouldn't fire anyway, and the body marker
   "continue" is not in the canned content. Guards the
   round-2 false-ack regression directly.
5. **`lead_launch_prompt_routes_through_verified_paste`** (router) —
   fire `mission_goal` against the router with the existing
   `cfg(test)` zero-delay constant; assert the fake injector's
   `inject_paste_with_verify` was called once with the composed
   launch-prompt body, **and** that the legacy `inject` path was
   *not* called for that body (no leftover keystroke-then-`\r`
   chord). The test works under `cfg(test)`'s zero-delay shape
   because the new routing decision (verified path for *all*
   delay values, not just non-zero) is no longer keyed on the
   `LEAD_LAUNCH_PROMPT_DELAY` constant — guards against future
   churn re-introducing the keystroke path.

## Risks

- **`Pasted text` match is Claude-Code-specific.** If a future
  Claude Code version renames the placeholder, large pastes start
  failing the body-marker path AND failing the placeholder-delta
  path — retry loop exhausts attempts, persona doesn't land.
  Mitigation: the substring check is a single literal in
  `count_placeholders`, trivial to update. Worst case we revert to
  the blind wait — same failure mode as today, no regression.
- **Body marker false-positive on agent prior output.** The first
  ≤32 chars of a persona could coincidentally match unrelated pane
  content (e.g. agent's own banner). The count-delta-vs-before
  scheme defends against the *static* version of this (resume into
  a pane that already contains the marker — `before` and `after`
  both count it once, delta = 0, reject) but not the *dynamic*
  version: if the agent's banner produces the marker substring
  between our `before` capture and our `after` capture, the count
  goes 0 → 1 even though our paste hadn't landed yet. Concretely
  this would require the agent's TUI to render text matching the
  first 32 chars of the persona during the 600ms render-wait;
  vanishingly rare for "You are a/an…" personas, and impossible
  for codex (its banner is a fixed string with no persona-shaped
  text). If it bites we'd extend the marker to require it appear
  inside the input box specifically (e.g. only count occurrences
  in the bottom N rows of the visible region).
- **Capture-pane cost.** ~1ms per call on macOS in benchmarks; 1
  pre-paste capture + up to 4 post-paste captures per spawn worst
  case. Negligible.

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
the full unit suite (5 new tests + 197 existing pass); manual smoke:
(a) spawn three claude-code direct chats with personas in parallel,
confirm all three show the persona as the first user turn; (b) start
a fresh codex-lead mission with a long preamble+brief (≥2KB), confirm
the lead receives the launch prompt and submits it (covers the
tail-marker path).

## Addendum: resume-continue caller policy (issue #94)

`schedule_continue_on_resume` pastes the literal `continue` (8 bytes)
into a freshly resumed claude-code pane so the agent picks up where
it left off without the user touching the keyboard. The body is 8
bytes — below `PLACEHOLDER_MIN_BODY_LEN = 64` — so the placeholder
delta gate inside `inject_paste_with_verify` is closed and only the
head/tail-marker delta for "continue" can ack the paste. Three real-
world conditions defeat that single ack signal:

1. **Capture race.** The 600ms render-wait isn't always enough; the
   pane snapshot we take post-paste sometimes hasn't repainted yet.
2. **TUI line-wrap.** When the composer happens to wrap mid-word,
   the rendered "continue" gets split across a soft-wrap boundary
   and the literal 8-char substring search misses it.
3. **Stale transcript content.** A resumed pane that already has the
   word "continue" scrolled into view (the agent's last turn echoed
   it, the user typed it earlier, etc.) makes `before_head_count`
   match `after_head_count`. Delta = 0, verify rejects.

Originally the caller just logged the verify failure and gave up —
leaving the user staring at a pane with `continue` typed into the
composer but no Enter sent. The fix layers a **caller-side recovery
policy** on top of the unchanged strict primitive:

- On verify success, `inject_paste_with_verify` sends Enter and
  returns Ok. Fallback path does not fire.
- On verify failure (Err), the caller sends one fallback Enter via
  `SessionManager::send_enter`. Two cases land here:
  - Paste body did land but readback missed it → Enter submits the
    visible "continue" and the agent resumes. User's intent.
  - Paste body did not land → Enter on an empty claude-code composer
    is a no-op. Cheap.

The downside of NOT recovering (user stuck, has to type manually)
clearly outweighs the downside of a stray Enter on this specific
caller. The policy is intentionally caller-local: `inject_first_turn`
and other callers paste multi-KB launch prompts where a stray Enter
on a partial body would submit garbage. Those callers keep the
strict "only Enter on verified paste" contract guarded by
`continue_resume_rejects_stale_placeholder`.

Tests added alongside the fix:
- `continue_resume_falls_back_to_enter_on_verify_failure` — exact
  one fallback Enter on stale-placeholder Err.
- `continue_resume_no_double_enter_on_verify_success` — exactly one
  Enter on the happy path; fallback does not double-fire.
- `continue_resume_skips_for_non_claude_runtime` — no paste, no Enter
  for shell / codex.

Distinct stderr logs (`verify failed … sending fallback Enter` vs.
`fallback Enter … failed`) let us detect frequency in the field
without re-instrumenting.
