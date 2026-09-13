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
| Claude Code | `✳ Claude Code`, `⠐ Native spike fixture ok` | cleared |
| zsh + oh-my-zsh auto-title | `sleep 4` | `jason@Jasons-Mac-Studio:~/repos/runner` |

`crates/runner-terminal/src/terminal.rs:407` is a `title()` accessor over a mutex alacritty keeps current. It has **zero callers in the workspace**.

## Scope

**Status is derived from the title when the session declares one, and from bytes otherwise.** The byte detector is not deleted; it becomes the documented fallback so nothing regresses for a runtime that says nothing.

### Classification

Two layers, in order:

1. **Spinner prefix.** A title whose first grapheme is a known animation glyph means busy. The observed sets are braille `U+2800`–`U+28FF` and `✳`. Absence of the prefix, on a session that has shown one, means idle.
2. **Baseline divergence.** For runtimes with no spinner, a title differing from the session's learned idle baseline means busy, and a return to it means idle.

Layer 1 alone fixes #583 and covers both agent runtimes with no learning and no state. Layer 2 is what covers shells, and it needs a baseline-learning rule that nothing in the captures settles — see Phases.

**Precedence.** The title detector arms itself per session the first time it observes a usable transition. Until armed, the byte detector owns that session's status. Once armed, title wins and byte transitions are ignored for that session. No setting, no per-runtime configuration.

**Debounce.** Codex rewrites its title roughly ten times a second while spinning. Only classification *changes* reach the status path; identical consecutive classifications are dropped at the terminal layer, before the channel.

### Not changing

The status vocabulary (`SessionActivityState`), the transition sink (`note_forwarder_transition`, which already dedupes and already accepts a `source` discriminator alongside `forwarder`, `agent` and `input-submit`), the mission event shape, and every UI surface that renders status.

## Implementation phases

**Phase 1 — spinner detection, agents only.** Classify in the existing `Event::Title` arm of the `native-term-events-{session_id}` thread (`terminal.rs:381`), report through a sibling of `report_input_state` (`terminal.rs:329`, `:514`), and emit with `source: "title"`. Byte fallback untouched for unarmed sessions. This closes #583.

**Phase 2 — baseline divergence, shells.** Only after phase 1 ships and the rule has been validated against recordings. The open question is how to learn the baseline: the title at spawn before any work, the title at the first byte-quiet moment, or the longest-lived title over a window. Each has a failure mode — a `cd` rewrites a shell's idle title, and a session that starts busy never shows a clean baseline. **Do not guess this in phase 1.** Prototype against the fixtures and write down what was chosen and why.

**Phase 3 — precedence with the third signal.** `InputTracker` (`crates/runner-terminal/src/input_state.rs`) already reports composer state and is a third opinion on the same question. Three signals need one owner and a stated order, not three independent votes. Scope it once phases 1 and 2 have settled what the title can actually carry.

## Verification

Fixture-driven, against real recordings rather than synthetic input. Add the two #583 captures and a `sleep 4` shell recording to `crates/runner-terminal/fixtures/`.

- **Codex idle-with-animation:** derived status goes idle at t≈2 s, not t=19.7 s. This is the #583 regression test.
- **Codex working:** stays busy for the full span the spinner is present.
- **Claude Code:** the existing `claude-session.ndjson` classifies busy while `✳`/braille is prefixed.
- **Silent shell work** (phase 2): busy for the full four seconds of `sleep 4`.
- **No title signal:** a session whose runtime sets no usable title behaves exactly as today, proven by an unchanged assertion.
- **Debounce:** ten title writes per second produce at most one transition per classification change.
- `make verify` green.

## Non-goals

- OSC 9;4 and OSC 133. Real future-proofing, but no runtime we ship emits either — zero occurrences across 600 KB of capture. Separate issue.
- Hung-agent detection, where the title claims busy while bytes have been silent for a long time. The design should not preclude it; v1 does not attempt it.
- Surfacing the declared task text (`Create a random mission`, `renaming...`) in the sidebar or rail. This makes it possible and it is worth having, but it changes pixels and needs a design pass first.
- Suppressing the animation itself, or implementing DEC 2026 synchronized output so the renderer stops observing half-drawn frames. Both are rendering concerns, not status ones.
