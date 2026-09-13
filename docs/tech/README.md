# Tech Notes

Deep dives on the libraries Runner is built on, written for someone reading the code for the first time. `arch/` says how Runner is designed; these notes say how the machinery underneath actually works, with pointers into the pinned dependency sources so the reader can follow along in real code.

Each note names the crate version it was written against. When a dependency is bumped, re-read the note against the new source and fix what moved; a note that drifts from the pinned version is worse than none.

## Notes

- [`terminal-rendering.md`](./terminal-rendering.md) — the path from a PTY read to painted pixels inside Runner: the terminal model crate, the bridge, the GPUI element, and the four design decisions that shape it.
- [`alacritty-terminal.md`](./alacritty-terminal.md) — `alacritty_terminal` 0.26 as an emulator without a window: the vte parser, the Handler vocabulary, the Term and its ring-buffer grid, cells, colors, modes, reflow, and the event channel back to the PTY.
- [`gpui-rendering.md`](./gpui-rendering.md) — `gpui-ce` 0.3 as Runner uses it for the terminal: the frame lifecycle, the Element trait, the scene and its GPU batches, text shaping and the layout cache, paths, masks, hitboxes, IME input, and entities.

## Background reading

These notes assume terminal fundamentals: a PTY, escape sequences, and the cell grid. When a note reads as jargon, the gap is usually here rather than in the note. In order:

- [Anatomy of a Terminal Emulator](https://poor.dev/blog/terminal-anatomy/) — Zellij's author builds an emulator from scratch in Rust: spawn a shell, read its bytes, parse the sequences, draw a grid. The best single prerequisite for `terminal-rendering.md`, because it establishes the emulator-and-renderer split that note opens by assuming.
- [The TTY demystified](https://www.linusakesson.net/programming/tty/) — where the PTY master, the blocking reader thread, and SIGWINCH come from. Skim the teletype history at the top and slow down at the diagrams.
- [ANSI escape code](https://en.wikipedia.org/wiki/ANSI_escape_code) — a reference to keep open, not a read-through. `ESC [` is the Control Sequence Introducer and `ESC [ ?` marks a DEC private mode, which is all it takes to read `CSI ? 1049 h` and the 2026 and 2031 sequences as English.
- [The Secret Rules of the Terminal](https://wizardzines.com/zines/terminal/), or free on the same author's blog, [Entering text in the terminal is complicated](https://jvns.ca/blog/2024/07/08/readline/) and [Rules that terminal programs follow](https://jvns.ca/blog/2024/11/26/terminal-rules/) — the shell, the emulator, the program, and the TTY driver as four separate pieces with no single owner. Motivates the key-encoding and IME half of `terminal-rendering.md`.
- [A parser for DEC's ANSI-compatible video terminals](https://vt100.net/emu/dec_ansi_parser) — skim the state diagram before layer 1 of `alacritty-terminal.md`. The `vte` crate implements this document.

`gpui-rendering.md` has a different prerequisite, immediate-mode rendering and glyph atlases rather than terminals, and nothing above covers it.

## Finding the sources

Cargo keeps every pinned crate's source on disk. On macOS:

```sh
ls ~/.cargo/registry/src/*/alacritty_terminal-0.26.0/src
ls ~/.cargo/registry/src/*/vte-0.15.0/src
ls ~/.cargo/registry/src/*/gpui-ce-0.3.3/src
```

For alacritty, a clone of `github.com/alacritty/alacritty` at tag `alacritty_terminal_v0.26.0` is byte-identical to the registry copy and comes with history and blame.
