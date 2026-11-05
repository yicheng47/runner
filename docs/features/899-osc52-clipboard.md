# 899 — OSC 52 clipboard writes from terminal programs

Tracking issue: [#899](https://github.com/yicheng47/runner/issues/899). No design file: the feature adds no UI. The open questions were settled with Jason on 2026-11-05 (see Decisions).

## Motivation

Terminal programs set the system clipboard with OSC 52 (`ESC ] 52 ; c ; <base64> BEL`). Neovim uses it as its clipboard provider over SSH, tmux forwards copies through it with `set-clipboard`, and Helix relies on it. The vendored `alacritty_terminal` parses the sequence, decodes it and emits `Event::ClipboardStore`, but the terminal model's event loop (`crates/runner-terminal/src/terminal/model.rs`, the `_ => {}` arm of the `native-term-events-*` worker) drops it. A yank in any of these programs silently copies nothing. Ghostty, iTerm2 and kitty accept these writes by default.

## Decisions

- **The app writes the clipboard, not `runnerd`.** The daemon is headless and has no clipboard code; the app already writes it through gpui's `ClipboardItem`. Routing through the app also puts a copy made in a remote window on the viewer's clipboard rather than the host machine's.
- **No window showing the session → drop the write.** A program running unseen must not replace what the person pastes next.
- **Silent.** No toast or other feedback, as in other terminals.
- **Reads stay refused.** Answering `ClipboardLoad` (OSC 52 with `?`) would let any program in a pane read the clipboard.
- **Size cap: 1 MiB of decoded text.** Anything larger is ignored whole, never truncated.

## Scope

### Terminal model (`runner-terminal`)

- Handle `Event::ClipboardStore(ty, text)` in the event worker and pass it to the host through a new `TerminalHost` method, e.g. `fn clipboard_store(&self, session_id: &str, target: ClipboardTarget, text: &str)`. Every `TerminalHost` implementation, including test hosts, gains it; test hosts may ignore it.
- Ignore the write when `text` is empty or longer than 1 MiB (byte length of the decoded UTF-8 string). An empty payload does not clear the clipboard.
- `ClipboardLoad` stays unhandled. The term's `Config::osc52` stays at alacritty's default `OnlyCopy`, so the parser never emits it; a test pins that `?` produces no reply on the PTY.
- `ClipboardType::Selection` (`p`/`s`) maps to the primary selection on Linux and to the clipboard on macOS and Windows.

### Daemon (`runner-daemon`)

- The session manager's `Host` forwards the write as a new `SessionEvents` method, and `CoreSessionEvents` emits it on the event channel as `session/clipboard` with `{"session_id", "target", "text"}`, the same path `session/input-error` takes.
- The text is never written to the event log, the database or the log file. The event channel is an in-memory broadcast, so emitting it persists nothing; keep it out of any `log::` line too.
- A remote viewer must receive it: extend the remote client's event routing (`remote/client/route.rs`) the way it maps `session/input-error`'s session id, so the viewer's app sees the event under its remote session address.
- `runner feed` and other CLI event consumers do not print it.

### App (`runner-app`)

- The root event pump (`surfaces/root/init.rs`) passes `session/clipboard` on. The app process writes the clipboard **once** when any of its windows shows that session in a pane of its active tab (chat surface or mission workspace), and drops it otherwise.
- Write with `cx.write_to_clipboard(ClipboardItem::new_string(text))`, or `write_to_primary` for the Linux selection target.
- No toast, status line or other UI.

### Out of scope

- OSC 52 reads, a Settings toggle for OSC 52, and clipboard formats other than plain text.
- Any change to Runner's own copy (mouse selection, ⌘C) or paste.

## Tests

- `runner-terminal`: feeding `ESC ] 52 ; c ; <base64> BEL` (and the `ST` terminator) calls the host with the decoded text; `p`/`s` map to the selection target; empty, oversize and invalid-base64 payloads call nothing; `ESC ] 52 ; c ; ? BEL` calls nothing and writes nothing to the PTY.
- `runner-daemon`: the host forwards to `session/clipboard` with the session id; the remote route maps it.
- `runner-app`: the routing decision (session shown in an active tab → write once; not shown → drop; shown in two windows → one write) as a unit test over the window/tab state, not a live clipboard.

## Live checks

Required, on macOS in the development app (`make run`). Each one exercises behaviour the in-process tests cannot reach, the real pasteboard:

1. **Shell copy.** In a shell pane run `printf '\e]52;c;%s\a' "$(printf 'runner-osc52' | base64)"`, then `pbpaste` in Terminal.app or another pane prints `runner-osc52`.
2. **Neovim yank.** In a shell pane, `nvim --clean -c 'let g:clipboard="osc52"'` (Neovim 0.10+), yank a line with `"+yy`; ⌘V into another app pastes that line.
3. **Read refused.** `printf '\e]52;c;?\a'` in a pane prints nothing back and leaves the clipboard unchanged.
4. **Oversize ignored.** Write a payload of just over 1 MiB (e.g. `head -c 1100000 /dev/zero | tr '\0' a | base64`); the clipboard keeps its previous content.
5. **Not shown → dropped.** In a pane run `sleep 5; printf '\e]52;c;%s\a' "$(printf hidden | base64)"`, switch that window to another tab before it fires; the clipboard does not change.
6. **Remote viewer** (only if a second paired computer is available; otherwise record it as not run): a copy in a remote window lands on the viewer's clipboard, not the host's.

Windows and Linux are covered by CI and the unit tests; record them as not live-tested.

## Affected docs

- `docs/guide/`: the terminal page that covers copy and paste gains a short paragraph: programs in a pane can set the clipboard with OSC 52 while the pane is shown; reading the clipboard is refused.
- `docs/product/vision.md` lists OSC 52 among TUI edge cases; no change needed.
