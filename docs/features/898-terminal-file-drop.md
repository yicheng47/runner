# 898 — Drop files onto a terminal to insert their paths

> Tracking issue: [#898](https://github.com/yicheng47/runner/issues/898)
> Priority: P2. Milestone: 0.16. Platforms: macOS and Windows.

## Motivation

Dropping a file onto a terminal in Ghostty, iTerm2 or Terminal.app inserts its escaped path. Runner's terminals only accepted Runner's own drags (pane reordering), so a file dragged from Finder or Explorer did nothing. Dragging a screenshot or a file into Claude Code or Codex is the quickest way to hand it over, and copying the file and pasting it already worked through `terminal_paste.rs`.

## Behavior

- Chat panes, chat drawer terminals, mission slots and mission drawer terminals accept files dragged in from the system file manager. GPUI turns the platform drag into an `ExternalPaths` drag on both macOS and Windows, so one implementation covers both.
- A drop inserts the paths exactly as a file paste does: each quoted with `shell_quote_path`, joined with spaces by `format_pasted_paths`, and written with `TerminalMirror::paste`, so it is bracketed when the program enabled bracketed paste and carries no trailing newline.
- The terminal that takes the drop gets keyboard focus. In a chat tab its pane also becomes the focused pane, as a click would make it.
- While files are dragged over a terminal, it shows the pane-reorder preview's treatment: the accent at 12% over the terminal with a 1px accent border. The highlight and the drop are one hitbox laid over the terminal surface, so the terminal that lights up is the one that takes the drop. Pane reordering is a different drag type and keeps its own target on the pane.
- Only an interactive terminal is a target, under the same condition that registers its paste action. A starting, ended, archiving or duplicate terminal neither highlights nor takes the drop.

## Decisions on the open questions

- **Images insert their path.** A pasted image works by sending the agent's image-paste shortcut, after which the agent reads the system clipboard itself. A drag does not touch the clipboard, so the shortcut would read whatever was copied last. Claude Code and Codex both read an image from a path.
- **Mission slots need nothing different.** A drop goes through `TerminalMirror::paste`, the call a paste makes: the daemon observes it as a paste for the delivery gate and writes it through the same input path. The slot handler checks `mission_terminal_interactive`, as the paste handler does.

## Out of scope

- The agent update terminal in Settings → Agents.
- Remote sessions: the paths are the local machine's, as with file paste.

## Verification

- `terminal_paste::tests::dropped_paths_insert_like_pasted_files`: dropped paths format exactly like pasted files, and an empty drop inserts nothing.
- `surfaces::panes::tests::dropped_files_focus_the_pane_they_land_on`: in a two-pane split, a file drop on the unfocused pane's terminal focuses that pane and its terminal, and the paste is sent to that pane's session.
- Live, in the development app: drag a file whose name contains a space from Finder onto a shell chat pane. The pane highlights while the file is over it, the quoted path appears at the prompt without running, and the pane takes focus. Repeat on a split's other pane and on a drawer terminal; drag a pane by its header to confirm reordering still previews and lands.
