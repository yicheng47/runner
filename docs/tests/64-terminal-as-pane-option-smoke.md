# Feature 64 Smoke Test — terminal as a pane option

Human smoke pass for `feat/64-terminal-impl`, covering all four phases of `docs/features/archive/64-native-terminal.md`. Every check below is something the automated suites cannot reach: real PTYs, a real relaunch, real pixels, and real process groups.

What landed underneath, in one paragraph each phase. **Phase 1** adds `session_start_shell` (resolves `$SHELL`, falls back to `/bin/zsh`, spawns a plain login shell with no agent injection), `session_close` (kill, then drop the pane slot and the row together), and excludes shells from completion stamping and sidebar attention. **Phase 2** brings terminal panes back live across a relaunch regardless of the resume-on-launch setting, and resolves a vanished cwd to the nearest existing ancestor with a loud in-terminal notice. **Phase 3** replaces the pane header with the 26 px identity line, wires `⋯` / `×` / inline rename, and makes closing a terminal permanent with a foreground-process confirmation. **Phase 4** adds the Recent/project `+`, empty-pane and command-palette entry points, terminal-only sidebar icon/close behavior, and the **Shell exited** card.

Run against the development database with `make run`. One Runner instance, no crew mission in flight. Budget about fifteen minutes.

## Already covered — do not re-verify by hand

`make verify` is green on this branch: 563 backend tests, 256 native-app tests, workspace clippy (all targets), and fmt. Those pin argv and environment composition, the `session_close` transaction, completion suppression, the launch-claim gating and chat/shell stagger split, cwd fallback selection and notice byte order, the identity-line branches, menu contents, mixed-tab archive planning and counted confirmation copy, rename decisions, pane-close behaviour, and the palette entry. There is no value in re-checking any of that manually — the list below is deliberately only the parts a test cannot see.

## Setup

- [ ] In Ghostty, note `echo $SHELL` and `echo $PATH`. You will compare against both in §2.
- [ ] Settings → General → **STARTUP** → turn **"Resume running agents on launch"** OFF. The chat/terminal asymmetry is only observable with it off.

## 1. Entry points

- [ ] Open the `+` beside **RECENT** → **New chat**, **New terminal**, and **New mission** are siblings. Choose **New terminal** → it creates a new single-pane tab rather than splitting the active tab.
- [ ] Open a project's `+` → the same three actions appear. Choose **New terminal** → its new single-pane tab is nested under that project and starts at the project cwd.
- [ ] The new terminal-only tab uses the terminal glyph in the sidebar rather than the chat bubble. Split tabs keep their two-/three-pane layout glyphs.
- [ ] Open a chat and choose a split layout → the new pane stays empty and the **Start a chat** modal does not open automatically.
- [ ] From a single-pane tab, `⌘D` grows side by side and `⇧⌘D` grows stacked; repeat the same shortcut to reach three panes. Each added pane stays empty without opening a modal, and another split attempt shows the three-pane limit without changing the layout.
- [ ] In that empty pane, **New terminal** fills it and **New chat** remains the explicit path to the chat modal.
- [ ] `⌘K` → **New terminal** is the first entry, and typing `shell` still matches it.
- [ ] Choose **New terminal** from the palette with a focused *empty* pane → the terminal lands there and the preset does not change.
- [ ] Choose it with a focused *filled* pane while a different pane sits empty → it fills **the existing empty pane** rather than growing the split.
- [ ] Choose it on a full 3-pane tab → a clean error message, no crash.

## 2. It is a real login shell

- [ ] The terminal opens in **the cwd of the focused sibling agent**, not `$HOME`.
- [ ] `echo $PATH` matches what Ghostty printed — nvm, homebrew and friends all present.
- [ ] `env | grep RUNNER_` is **empty**. No agent environment, no bundled `runner` CLI on `PATH`.
- [ ] `which runner` does not resolve to Runner's bundled shim.

## 3. Attention — the section that decides whether the feature is usable

- [ ] Start `pnpm dev`, or any long streaming command, in a terminal pane.
- [ ] The sidebar tab row shows **no working spinner and no unread dot** — while it streams, and after it falls quiet.
- [ ] The terminal pane's identity line has **no status dot**. Chat panes beside it still do.
- [ ] A busy agent in the same tab **still** shows working. The shell must not suppress its peer.
- [ ] Stop the stream, let the agent go idle → the tab stamps unread normally.

## 4. Chrome, rename, close

- [ ] A single-pane tab renders **no identity line at all**. Split it → a 26 px line per pane.
- [ ] Focus a terminal pane → the chat side panel disappears. Focus a chat pane → **it comes back**, which proves the saved setting was not rewritten.
- [ ] `⋯` on a chat: Stop **⌘.** · Rename… · Archive chat. On a terminal: Stop **⌘.** · Rename…. **Never** a Close item on either.
- [ ] `⌘.` with the terminal focused stops it — the keystroke must reach the app rather than being swallowed by the PTY.
- [ ] Double-click the pane name → an inline field. Enter commits, and **the composed header title above updates with it**.
- [ ] Escape reverts. Clearing the field and pressing Enter restores the default (`zsh`).
- [ ] `×` on a **chat** pane: the split re-flows, and the chat is still in the sidebar and reopenable.
- [ ] Archive a chat pane: the pane stays in place, empty, offering **New chat** / **New terminal**.
- [ ] In a mixed chat/terminal tab, **Archive all** always asks once, including at a bare prompt. The dialog gives the live chat/terminal counts and says archived chats are restorable while closed terminals are not; confirming archives the chat, permanently closes the terminal, and removes the whole tab.
- [ ] `×` on a terminal sitting at a bare prompt → closes silently.
- [ ] Run `sleep 60`, then `×` → a **confirmation dialog**. Cancel keeps the pane; confirm kills the process and drops it.
- [ ] Right-click a single-pane terminal tab → **Close terminal** occupies the destructive slot where a chat has Archive. At a bare prompt it closes immediately; with `sleep 60` running it shows the same foreground-process confirmation before removing the tab.
- [ ] `⌘W` on a terminal pane inside a split behaves as `×`. On a single-pane tab, `⌘W` closes the **window** — that is intended, not a regression.

## 5. Lifecycle and relaunch

- [ ] Type `exit` → the **Shell exited** card: `exit 0 · zsh · ~/your/path`, compact, centered, with **Restart** and **Close**.
- [ ] **Restart** brings a fresh shell up in the same pane at the same cwd.
- [ ] Rename a terminal, quit, relaunch → **the name survives**. An un-renamed one shows `zsh` again.
- [ ] Quit with a split of one chat and one terminal. Relaunch: the **terminal is live immediately** while the chat waits on **Resume** (resume-on-launch is still off). Terminals should feel instant — they skip both the 300 ms auto-resume stagger and the claude launch gate.

## 6. Missing cwd — the loud path

- [ ] Spawn a terminal whose cwd is a scratch directory, e.g. `mkdir -p /tmp/smoke/deep` first, then open a terminal from a chat working there.
- [ ] Quit the app. Delete `/tmp/smoke/deep`. Relaunch.
- [ ] The terminal comes back at `/tmp/smoke` — the nearest existing ancestor, not `$HOME` — with a two-line notice above the shell's first output: `runner: … no longer exists` / `        opened … instead`, with `~` abbreviation where the path is under home.
- [ ] **The colour is the check no test can make.** The notice must render in the palette's yellow (`#FFB020`), clearly distinct from ordinary shell output. If it looks like normal foreground text, the SGR wrap is not reaching the parser.

## 7. Archive isolation

- [ ] Settings → Archived never lists a terminal, in any state.
- [ ] Create a project, put a tab holding one chat and one terminal under it, then **delete the project**. Archived shows **the chat only**; the terminal row is gone, with no orphan left behind.
- [ ] A terminal created from an empty pane or the command palette never sprouts an additional sidebar row; only **New terminal** from a sidebar `+` intentionally creates a new single-pane tab.

## If you only run five

§3 in full, the relaunch asymmetry in §5, the amber notice in §6, the project delete in §7, and the `sleep 60` confirmation in §4. Those are the four defects review turned up, plus the one thing only pixels can settle.

## Known and accepted — not smoke failures

- `⌘T` is deliberately unbound and absent from Shortcuts. It is reserved for the deferred multi-tab design rather than duplicating the empty-pane and command-palette entry points.
- The `⋯` menu has no **Close** item. The design frame `Spec — Split panes, slimmed chrome (64) · v1` → `view_menu` still draws one, but its own adjacent note says "Close pane is NOT in it". The code follows the note and the spec; the canvas frame is stale and still uses the deferred design's "view" wording.
- A single-pane tab holding a terminal shows a terminal glyph in the workspace header. Deliberate: with no identity line, the header carries the identity.
- A shell running without job control will not raise the foreground-process confirmation, because the foreground process group never moves. Interactive shells on a tty enable job control by default, so this is a corner rather than a gap.
