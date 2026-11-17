# #1366 Phase 1 — macOS terminal editing keys

Jason approved this mission for the hydrogen trio on 2026-11-17. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1366-terminal-editing-keys` on `feat/1366-terminal-editing-keys`, based on `origin/main` at `352a19fc959663d9f9718067c82164f55cea61e0`. The worktree uses its own `target/`. Scope is Phase 1 of [#1366](https://github.com/yicheng47/runner/issues/1366) only; Phases 2–4 (clear scrollback, prompt navigation, notifications) are out of scope.

## Problem

In macOS terminals, ⌘⌫, ⌘← and ⌘→ do nothing. `terminal_key_route` (`crates/runner-app/src/terminal/ime.rs`) sends every ⌘ chord to `TerminalKeyRoute::AppShortcut`, and `install_bindings` (`crates/runner-app/src/keymap.rs`) binds only ⌘C and ⌘V in the `Terminal` context, so the chords never reach the PTY. A user who "clears" a line with ⌘⌫ keeps the old text and the next typing is appended to it (#1356's capture).

Separately, `TerminalInput` (`crates/runner-app/src/terminal/ime.rs`) does not override `apple_press_and_hold_enabled`, which gpui defaults to `true`. In `gpui-pre-macos`'s key-down handling, a held printable key then goes to AppKit's press-and-hold path instead of `replace_text_in_range`, so holding a letter likely opens the accent picker instead of repeating the key.

## Outcome

In a focused macOS terminal (chat pane, terminal tab, mission slot, drawer shell, remote terminal):

| Chord | Sends | Effect in zsh/bash/agent TUIs |
|---|---|---|
| ⌘← | `\x01` (`^A`) | cursor to start of line |
| ⌘→ | `\x05` (`^E`) | cursor to end of line |
| ⌘⌫ | `\x15` (`^U`) | delete to start of line (zsh: whole line) |

These match Ghostty's defaults. The bytes are the same on the primary and alternate screen; no Home/End special case. Writing them follows the existing user-input path (scroll to bottom, respect the session's control/desktop gating like other terminal writes). A Runner shortcut that a user has bound to one of these chords in **Settings → Keyboard shortcuts** still wins over the terminal binding; every other ⌘ chord keeps routing to the app as today, and ⌘C/⌘V are unchanged.

`TerminalInput` returns `false` from `apple_press_and_hold_enabled`, so a held key repeats in the terminal. IME composition (marked text) is unaffected.

macOS only: Windows and Linux keep their current Ctrl handling.

## Read first

- `crates/runner-app/src/terminal/ime.rs`: `terminal_key_route`, `TerminalInput`, the `EntityInputHandler` impl and its tests.
- `crates/runner-app/src/terminal/input.rs`: `handle_key`.
- `crates/runner-app/src/keymap.rs`: `install_bindings`, how user overrides and contexts are layered; `crates/runner-app/src/keymap/tests.rs`.
- `gpui-pre-macos` 0.3.7 `window.rs` key-down handling around `apple_press_and_hold_enabled`, to confirm the held-key path.
- `docs/guide/reference/keyboard-shortcuts.md` and its `zh-CN` twin.

## Deliverables

- The three chords and the press-and-hold override, implemented the way the keymap already layers terminal bindings and user overrides. Choose between `Terminal`-context key bindings with actions and routing inside `handle_key`, whichever keeps user overrides winning with the least new machinery.
- Tests: routing/encoding for each chord, that other ⌘ chords still route to the app, that a user override on one of the chords wins, and the press-and-hold override.
- Guide: add the three chords to the terminal section of `docs/guide/reference/keyboard-shortcuts.md` and the zh-CN page, and correct the sentence there saying "⌘ shortcuts never reach a terminal".

Non-goals: Windows/Linux equivalents, a settings toggle, Home/End variants for the alternate screen, clear-scrollback, prompt jumping, any change to the ⌥-click cursor move or IME composition logic.

## Validation

Changed behavior: what three ⌘ chords send in a macOS terminal, held-key handling in the terminal input handler, and the keyboard-shortcuts guide.

Automated (coder): `cargo test -p runner-app` for the routing, keymap and input-handler tests; `make clippy`; `make fmt`; `git diff --check`. No workspace-wide test run is needed beyond what CI does.

Live checks (qa, macOS development app, after a clean review), each tied to the change:

1. **Line editing in zsh** (the chords' main target): in a new terminal tab, type `echo hello world`; ⌘← then type `X` → `Xecho hello world`; ⌘→ then type `Y` → text ends in `Y`; ⌘⌫ → line is empty and typing starts fresh.
2. **Line editing in an agent TUI** (a different line editor): the same three chords in a Claude Code chat's prompt (cheap model, low effort, a fresh folder under `/tmp/1366-keys-*`), without submitting a turn.
3. **Held key repeats** (press-and-hold change): hold `k` for about a second in the zsh tab → several `k`s, no accent picker. Also hold `e` (a letter with accents) the same way.
4. **IME unaffected** (the change touches the input handler): one Pinyin composition in the zsh tab commits the chosen characters.
5. **App shortcuts still work in a terminal** (routing change): ⌘C copies a selection, ⌘V pastes, ⌘K opens the command palette, ⌘T opens a terminal.
6. **User override wins**: bind a Runner shortcut (e.g. Toggle sidebar) to ⌘← in Settings → Keyboard shortcuts, confirm ⌘← in a terminal runs it and doesn't send `^A`, then restore the default. Back up the development keymap setting before changing it.

Skip agy per standing QA guidance. Record results in `docs/tests/runs/1366-terminal-editing-keys-mac.md` with screenshot evidence, and use `script/qa-lock` as AGENTS.md describes.

## Authorization

Approved crew: hydrogen trio (coder, reviewer, qa). The crew may commit, push this branch and open one PR against `main` that closes nothing (#1366 has later phases; reference it with `Refs #1366`). Label it `frontend-only`, `risk-medium` and `needs-live-test` until qa passes. Fold this brief into the implementation commit.
