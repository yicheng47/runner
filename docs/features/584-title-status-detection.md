# 584 — Session status from the declared window title

> Tracking issue: [#584](https://github.com/yicheng47/runner/issues/584)
> Priority: P1. Closes [#583](https://github.com/yicheng47/runner/issues/583).
> No design frame: v1 changes no pixels. The status dot, sidebar attention and rail badges all render `SessionActivityState` exactly as they do today; only the value flowing into them gets more accurate.

## Motivation

`IdleDetector` (`crates/runner-backend/src/session/pty_runtime.rs:674`) decides busy/idle from byte arrival alone: any byte is busy, two seconds of silence is idle, content is never inspected. Both halves are wrong, and both were measured rather than reasoned about.

**False busy.** Codex `0.154.0` animates a braille particle field in its composer at 6.6 Hz. In the capture attached to #583, the session declares itself idle at t=2 s by dropping its title spinner, then keeps emitting 6–8 frames per second for another 17 seconds. Runner reports busy for the whole 19.7 s. Steady-state cost is 10.1 KB/s per idle pane, all of it parsed and repainted for no information.

**False idle.** A shell running `sleep 4` emits nothing. Simulating the detector against a real byte timeline:

```
byte detector: Idle at 4.58s, Busy at 6.63s
title truth:   busy 2.57s -> 6.59s
```

It calls the session idle 2 s before the command finishes. Every silent task — a long compile, a slow fetch — has this today, and nobody had noticed because nothing surfaces it.

This is not cosmetic. Inbox reconciliation only nudges runners whose status is `Idle` (`crates/runner-backend/src/router/mod.rs:635`). False busy suppresses delivery; false idle routes work to a runner still mid-task.

**The correct answer is already arriving and being discarded.** Every runtime we ship declares its state in the window title:

| Source | Working | Idle |
|---|---|---|
| Codex `0.154.0` | `⠋ Create a random mission \| yicheng47`, ten braille frames cycling ~10 Hz | `yicheng47` |
| Claude Code | `⠂ Claude Code`, `⠐ Native spike fixture ok` — braille only | `✳ Native spike fixture ok`, and cleared at exit |
| zsh + oh-my-zsh auto-title | `sleep 4` | `jason@Jasons-Mac-Studio:~/repos/runner` |

`crates/runner-terminal/src/terminal.rs:407` is a `title()` accessor over a mutex alacritty keeps current. It has **zero callers in the workspace**.

## Scope

**Status is derived from the title when the session declares one, and from bytes otherwise.** The byte detector is not deleted; it becomes the documented fallback so nothing regresses for a runtime that says nothing.

### Classification

Two layers, in order:

1. **Spinner prefix.** A title whose first grapheme is braille (`U+2800`–`U+28FF`) means busy. Absence of it, on a session that has shown one, means idle.

   **`✳` is not a busy glyph, despite appearing in Claude Code's working titles.** An earlier draft of this table listed it as one; the fixture disproves it. Claude's in-TUI spinner cycles `✻ ✽ ✶ ✳`, and the title freezes on whichever frame was current when work stopped, so a star marks *rest*, not work. In `claude-session.ndjson`: `✳ Claude Code` at 2.637 s, before the user has typed anything; braille from 5.038 s, 34 ms after submit; `✳ Native spike fixture ok` at 7.891 s, 13 ms after the reply prints and 3 ms before the prompt returns. Treating `✳` as busy arms a Claude session before its first turn and never releases it, which is a permanent version of #583. Matching one frame of four would also flip an armed session to idle mid-turn on the other three, so the star is wrong under either reading.
2. **Foreground process, for shells.** A shell session is busy while its PTY has a foreground process other than the shell itself, and idle when it does not. This is a fact about the process tree, not a reading of the screen.

Layer 1 fixes #583 and covers both agent runtimes with no learning and no state. Layer 2 covers shells, and it does not use titles at all — see Phases for why the title route was abandoned.

**Scope of the win, stated plainly.** Shell is not a selectable runtime: `runtime_catalog_options` offers Codex, Claude Code and TRAE only, so no runner and no crew slot can be a shell. Shell sessions are terminal tabs, panes and drawer shells that the user opens directly, and nothing routes crew work to them. Phase 1 therefore carries the whole coordination benefit; phase 2 buys a correct status dot on terminal panes and nothing more. Worth doing because it is small and exact, not because anything is blocked on it.

**Precedence.** The title detector arms itself per session the first time it observes a usable transition. Until armed, the byte detector owns that session's status. Once armed, title wins and byte transitions are ignored for that session. No setting, no per-runtime configuration.

**Debounce.** Codex rewrites its title roughly ten times a second while spinning. Only classification *changes* reach the status path; identical consecutive classifications are dropped at the terminal layer, before the channel.

### Not changing

The status vocabulary (`SessionActivityState`), the transition sink (`note_forwarder_transition`, which already dedupes and already accepts a `source` discriminator alongside `forwarder`, `agent` and `input-submit`), the mission event shape, and every UI surface that renders status.

## Implementation phases

**Phase 1 — spinner detection, agents only.** Classify in the existing `Event::Title` arm of the `native-term-events-{session_id}` thread (`terminal.rs:381`), report through a sibling of `report_input_state` (`terminal.rs:329`, `:514`), and emit with `source: "title"`. Byte fallback untouched for unarmed sessions. This closes #583.

**Phase 2 — foreground process, shells.** Gate the byte detector's Busy→Idle transition on `SessionRuntime::has_foreground_process` for shell sessions: when the two-second silence would flip a shell to Idle, ask first, and stay Busy while a foreground child exists. One query per would-be transition, no polling.

**Titles were the wrong route for shells, and the recordings say so.** A shell does declare through its title — oh-my-zsh sets it to the running command via `preexec` and back to the prompt via `precmd` — but nothing tells the two apart reliably. A capture of five commands including a `cd` kills each candidate rule: "a title seen before is the baseline" fails because running `sleep 2` twice makes the *busy* title a repeat; "a title not seen before is busy" fails because `cd` mints a new prompt title; strict alternation from the first title classifies all eleven writes in that capture correctly but desynchronises permanently and silently the first time anything else sets a title, which any full-screen program does. The process tree has none of these problems and needs no shell configuration.

`has_foreground_process` is already implemented on the PTY runtime (`pty_runtime.rs:456`), already reads the process tree, and is already used to decide the close-confirmation prompt. Phase 2 is a gate on an existing query, not new detection.

**Phase 3 — precedence with the third signal.** `InputTracker` (`crates/runner-terminal/src/input_state.rs`) already reports composer state and is a third opinion on the same question. Three signals need one owner and a stated order, not three independent votes. Scope it once phases 1 and 2 have settled what the title can actually carry.

## Verification

Fixture-driven, against real recordings rather than synthetic input. Add the kept #583 capture and a `sleep 4` shell recording to `crates/runner-terminal/fixtures/`.

- **Codex, one capture for both properties:** `codex-title-working.ndjson` stays busy for the full span the spinner is present, 0.177 s to 7.428 s, then stays idle through 46 further output events over 3.465 s whose largest gap is 153 ms. Not one of those gaps reaches the two-second threshold, which is why the byte path reports Busy for the entire recording — that continuous tail *is* the #583 regression test. It is not a replay of the original 19.7 s animation; what makes it conclusive is the zero, not the duration. The capture that carried the longer animation was deleted rather than replaced: it leaked a home path, a memory-repo listing and a project record into a public repo, in the recording and again in plaintext in its snapshot.
- **Claude Code:** the existing `claude-session.ndjson` classifies busy only across 5.038–7.891 s, the braille window that brackets submit and reply. The two `✳` titles at 2.637 s and 7.891 s must classify idle; asserting otherwise encodes the bug this table originally had.
- **Silent shell work** (phase 2): busy for the full four seconds of `sleep 4`, driven by the foreground-process gate rather than by output. Today the byte detector reports Idle at 4.58 s, two seconds before the command finishes.
- **No title signal:** a session whose runtime sets no usable title behaves exactly as today, proven by an unchanged assertion.
- **Debounce:** ten title writes per second produce at most one transition per classification change.
- `make verify` green.

## Non-goals

- OSC 9;4 and OSC 133. Real future-proofing, but no runtime we ship emits either — zero occurrences across 600 KB of capture. Separate issue.
- Hung-agent detection, where the title claims busy while bytes have been silent for a long time. The design should not preclude it; v1 does not attempt it.
- Shell integration. Runner spawns the shell and injects environment already, so it *could* emit OSC 133 semantic prompts and read exact command boundaries, which is what Ghostty does. Phase 2 does not, because the process tree answers the same question without touching the user's shell configuration.
- Surfacing the declared task text (`Create a random mission`, `renaming...`) in the sidebar or rail. This makes it possible and it is worth having, but it changes pixels and needs a design pass first.
- Suppressing the animation itself, or implementing DEC 2026 synchronized output so the renderer stops observing half-drawn frames. Both are rendering concerns, not status ones.
