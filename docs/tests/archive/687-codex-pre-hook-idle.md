# #687: Codex before its first hook

## Reproduction evidence (2026-09-21, macOS)

The installed executable reports `codex-cli 0.155.1`. Probes used a disposable `CODEX_HOME` and working directory, a trusted canonical temporary path, `gpt-5.6-terra`, a loopback-only model provider, and Runner-equivalent overrides for all nine hooks. No real conversation, configuration, database, or Runner app was changed. The disposable TUI configuration also matched the reported status line: `model-with-reasoning`, `project-name`, `git-branch`, `five-hour-limit`, `weekly-limit`.

- Fresh untouched prompt: bracketed-paste enable (`ESC[?2004h`) appeared at 35 ms in the recorded startup capture. The empty composer rendered, but the hook feed remained zero bytes throughout a 12-second observation.
- A local canned Responses endpoint accepted a disposable `fixture hello` submission and returned a fixed assistant response. Before submission the feed was empty; after submission the installed binary emitted `SessionStart` (source `startup`), `UserPromptSubmit`, then `Stop`. Both recorded model requests went to localhost; no paid model request was made.
- Resuming that disposable conversation by its valid native key restored the completed fixture response and empty composer. Its fresh hook feed again remained zero bytes for 12 seconds without submission.
- Native `/status` before any model turn returned to the empty composer without emitting hooks: the feed remained zero bytes for the 6-second capture, with the final redraw at 2.181 seconds. This exposed a draft implementation that would have latched Working on text plus Enter; the final implementation instead keeps such input provisional and uses the ordinary baseline fallback.
- The local upstream source snapshot at `7f01a84eff` queues SessionStart until turn execution. This is corroborating source evidence, not a claim that the snapshot is the source of the installed binary.

The default empty composer settled after its startup draws in these probes. Continuous unsolicited empty-composer repaint was **not** reproduced in the installed binary. An earlier capture with 167 reads in 12 seconds was a directory-trust dialog, caused by macOS `/var` versus `/private/var` canonicalization; it is not evidence of continuous empty-composer repaint. That setup was corrected before capturing the fixture used by the tests.

`crates/runner-backend/src/session/fixtures/codex-0.155.1-idle.json` contains only the observed terminal-mode setup and a final actual empty-composer redraw frame, without paths or conversation identifiers. The integration fixture repeats that frame every 80 ms through a real PTY, with an empty hook feed. This tests the reported mechanism deterministically rather than claiming that the user's original bytes were captured.

## Before and after

The final `codex_pre_hook_startup_ignores_continuing_idle_redraw` regression was run with the original `HEAD` version of `pty_runtime.rs`, adapting only its test `SpawnSpec` literal for the newly added metadata field. The test failed with exit 101:

```text
assertion `left == right` failed: fresh
  left: Working
 right: Idle
```

Restoring the implementation makes the same test pass for fresh direct launch, unkeyed direct resume, and keyed mission resume. Each case repeats the captured frame for longer than the old two-second silence threshold, checks that the hook feed is empty and hooks are unarmed, and verifies baseline Idle with no turn outcome.

The original detector resets its silence timer on every nonempty PTY read. Consequently, an indefinitely repeating idle redraw cannot reach Idle. The backend had no distinction between that stream and work while Codex had not yet emitted a hook.

Review exposed two additional cases, both demonstrated failing before their corrections: without a readiness sequence the first implementation never reached Idle after silence, and Up/Down history recall followed by Enter incorrectly remained Idle. Portable tests now cover the missing-readiness fallback and both normal/application-cursor arrow encodings with hooks available and unavailable. Missing readiness is a tested input condition; whether a particular Windows console host omits the sequence was not measured here.

## Ownership and limits

- The shared spawn-argument path explicitly tells the PTY whether this is Codex and whether Runner has queued an automatic first turn. Windows batch prompts are counted before the existing argv suppression, so readiness cannot cancel or hide their pending delivery.
- Before the first authoritative hook, Codex terminal readiness plus no pending turn produces **baseline Idle**, not hook Ready or a completed outcome. Repeated redraw, resize, draft echo, and bare Enter do not turn that empty startup into persistent work.
- Until readiness is actually observed, an unprompted launch retains the ordinary byte/silence fallback. A terminal that omits readiness therefore retains the prior behavior rather than acquiring a permanent Working hold. Continuous redraw without readiness remains a limitation of that fallback.
- Runner's known automatic first turn preserves baseline Working while a configured hook bridge awaits the first turn. Local text followed by Enter, router delivery, and paste submission publish provisional baseline Working and then retain the ordinary output-based fallback: Enter can dispatch a native command such as `/status`, so it cannot latch model work indefinitely. A SessionStart arriving after submission cannot cancel that work. UserPromptSubmit or another accepted turn observation hands ownership to the existing hook reducer and manager arbitration.
- Hook-owned Working cannot be ended by quiet output, an idle-looking frame, or the startup heuristic. Completion, interruption, human-input holds, generation/session/turn filtering, child filtering, and draft delivery protection retain their existing mechanisms.
- A disabled hook bridge, failure to initialize the watcher, or a watcher read failure uses the existing raw-output fallback for pending/submitted work. Its Idle is baseline inference with no completion outcome. An untouched ready prompt can still use startup Idle without claiming the hook bridge is armed.
- An existing, readable, but permanently empty feed cannot establish completion. After a local/native-command submission, the ordinary output fallback may infer Idle from silence, but does not emit Ready, a completion outcome, or hook authority. A known automatic launch prompt remains protected until actual turn-hook evidence or a detected bridge failure. The raw input classification uses the existing local-input heuristic plus Codex history-recall arrows, not proof that the CLI accepted a model request. Repeated redraw after a local/native command remains a limitation of the ordinary fallback; this fix targets untouched startup/resume and does not attempt to parse Codex commands or infer completed model turns from terminal contents.

Only backend code changes. The status vocabulary and UI are unchanged. The existing Windows first-turn readiness matcher is reused as a small streaming helper; no terminal screen parser is added.

## Esc interruption follow-up

After Jason confirmed the resume fix, his nominated development session was inspected read-only. Its live backend observation remained Working from baseline, but the current-generation feed contained exactly one `Interrupt` entry and no `SessionStart` or `UserPromptSubmit`. The native transcript recorded `task_started` at 12:33:49.797 UTC and a matching `turn_aborted` with reason `interrupted` at 12:33:50.288 UTC on 2026-09-21. The original conversation was created with 0.154.0; that saved metadata is not evidence of the resumed binary version. A separate disposable probe of installed 0.155.1 with a held loopback-only response confirmed that ordinary Esc interruption emits `SessionStart`, `UserPromptSubmit`, `Interrupt`, and the native abort boundary. An earlier-keypress probe did not interrupt the turn and is not claimed as a reproduction of the user's event ordering.

The reducer previously required `UserPromptSubmit` to establish the current turn ID before it would accept `Interrupt`. Early cancellation without that prompt hook therefore discarded the only authoritative interruption signal. The correction permits a root `Interrupt` to establish the turn only when no turn is established. It first publishes the existing interrupted/unavailable state, then the existing matching native abort boundary yields Ready with an Interrupted outcome (shown as Idle in the UI). It does not infer completion from Esc, silence, or terminal text. Existing generation, child, session, active-turn, retired-turn, and ended-session guards remain in place.

The real PTY/manager regression `codex_pre_hook_early_escape_accepts_interrupt_as_first_hook` submits text, sends Esc, and supplies only the observed interruption/abort sequence while redraw continues. Before the correction it failed with `expected Unavailable/Hook, got ... Working, source: Baseline`; afterward it verifies interrupted hook takeover, the matching abort boundary, stable Ready/Interrupted through redraw, and subsequent hook-owned work. A portable reducer regression covers first interruption with no SessionStart and with startup/resume SessionStart, absent turn IDs, child events, mismatched abort records, delayed events, and interruption isolation once a new turn is active.

## Validation

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`: 899 passed after the Esc follow-up; all 107 Codex-focused tests also passed. The first full run hit the existing three-second fork-materialization timeout in `headless_fork_rejects_nonzero_exit_and_kills_timed_out_process_group` (898 passed, one failed); that test passed in isolation and the unchanged full rerun passed.
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`: passed (existing dependency future-compatibility notices only).
- `cargo fmt --all --check` and `git diff --check`: passed.
- Native Windows execution: not available in this macOS session. Portable split-readiness/input tests run here. Existing Windows batch-delivery tests use the shared helper, and an additional Windows test checks that pending Codex work is recorded before argv suppression; native ConPTY smoke remains required.
- Jason confirmed the resume behavior was fixed, then confirmed the smoke test passed after the Esc correction. This does not establish native Windows coverage.

## Minimal live smoke

1. In the next development build Jason chooses to launch, open a fresh Codex direct chat without a persona prompt, then separately resume an unkeyed direct chat and a keyed mission slot. Leave the prompt untouched. Check that each becomes and remains Idle, with baseline source until hooks arrive. Separately resume a keyed mission slot with a router message queued for it; confirm the message lands and submits despite the earlier readiness-based Idle transition. Busy/Idle is not a delivery gate; existing interaction, ticket, and draft protections still control delivery.
2. Confirm an empty Enter does not leave the untouched prompt Working. Type a draft without submitting; confirm Idle and no routed message overwrites the draft.
3. Submit real work and run an automatically prompted mission. Confirm Working through the first SessionStart/UserPromptSubmit sequence, including a quiet tool, and Ready/Idle only on the hook's completion or interruption result. Deliver one routed message and confirm it starts a new turn. In a resumed Codex chat, submit and immediately press Esc. Confirm the interrupted turn becomes Idle even when Interrupt is its first hook, then submit another turn and confirm normal Working/completion.
4. On Windows, repeat with the npm `codex.cmd` launcher and verify the automatic first-turn paste actually reaches the agent after readiness. Include a Windows 10 system-conhost run: an untouched prompt must reach Idle even if readiness bytes are absent. This host-specific behavior still needs native measurement.

## Review

Reviewer completed round 2 for the resume fix through Runner on 2026-09-21 at 12:11 UTC: **“VERDICT: no remaining must-fix.”** The Esc follow-up received **“no remaining must-fix on the complete current tree”** at 12:41 UTC (Runner message `01M31ZRBE2QK0MYT6SCZPRGT4D`). The reviewer verified the failing-before regressions, passing check logs, and ownership boundary. The existing fork-materialization timeout remains a nonblocking test-timing watch item after passing isolated and full reruns. Native Windows execution, naturally continuous composer repaint, and the ordinary fallback after native commands remain documented limitations. Deriving pending work from delivery success and handling Esc/Ctrl-U draft clearing were explicitly left outside this fix.

After confirming the live smoke test passed, Jason authorized publishing and merging the fix and removing its local worktree on 2026-09-21. Runtime-aware title fallback remains a separate follow-up in #688.
