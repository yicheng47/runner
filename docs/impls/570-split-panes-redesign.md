# 570 — Split panes redesign, PR 1: no focus border, unfocused panes faded, chat glyph

Tracking issue: [#570](https://github.com/yicheng47/runner/issues/570). Spec: [570](../features/570-split-panes-redesign.md), phase 2 only. Feature, P2. Design: `design/runner.pen` frame `Spec — Split panes redesign (570) · v1` (`w2FDzK`), surfaces B and C, committed `63e5818`. Branch: **`feat/570-split-panes-redesign` already exists and is checked out** — it carries this brief; work on it, do not create another. The layout tree, the split icon, and the picker removal (spec phases 3–5) are a second brief after this lands; do not start them.

## What ships

In a split tab the focused pane has no border, and every unfocused pane's body below its identity line is drawn at 70 % opacity: grid, cursor, selection, ended card, empty stub, all together. The 1 px divider stays the only chrome between panes. The identity line never dims. Chat panes show the chat glyph instead of the bare prompt, on the identity line and on the header title, matching the sidebar rows. Nothing else changes.

## Where the code is

- `crates/runner-app/src/surfaces/panes.rs:1855` `focused` and `:1856` `grouped` for a leaf; `:2345` the border: `.when(grouped, |pane| pane.border_1().border_color(if focused { accent } else { transparent }))`.
- `panes.rs:2048` `let body: AnyElement = if let Some(entry) …` builds the pane body: the terminal stacked with its overlay (ended card, transitions) in the `if`, the empty stub in the `else`. `:2351` `.children(header).child(body)` assembles the pane.
- `panes.rs:2424` `pane_identity_icon`: shell → `square-terminal.svg`, other runtimes → `terminal.svg`, none → `square-dashed.svg`. Test at `:2799` `pane_identity_branches_for_chat_terminal_and_empty_panes`.
- `panes.rs:450`–`:456` the `WorkspaceHeader` glyph: grouped → `square-split-horizontal.svg`, focused shell → `square-terminal.svg`, else `terminal.svg`.
- `crates/runner-app/src/surfaces/sidebar.rs:3598` `sidebar_tab_icon`, the reference pair: single chat `message-square.svg`, single shell `square-terminal.svg`. `message-square.svg` is registered in `assets.rs:143`.
- GPUI element opacity is already in use: `ui/list.rs:180`, `ui/field.rs:1085`, `ui/overlay.rs:634` (`.opacity(if disabled { 0.6 } else { 1. })`). It reaches every quad and glyph painted underneath, the custom terminal element included.

## Fix shape

1. **Border.** Delete the `.when(grouped, …border…)` at `:2345`. No placeholder border on any pane.
2. **Fade.** `pub(crate) const UNFOCUSED_PANE_OPACITY: f32 = 0.7;` and a pure `fn pane_body_opacity(grouped: bool, focused: bool) -> f32` returning the constant for `grouped && !focused`, else `1.`. Wrap `body` in a `div().flex_1().min_h(px(0.)).min_w(px(0.)).flex().flex_col().opacity(pane_body_opacity(grouped, focused))` before `.child(…)` at `:2351`, so the identity line (`header`) stays outside it. The wrapper must not add padding, background, or a hitbox of its own. Both branches of `body` sit inside it: the ended card and the empty stub dim with the grid, and their buttons keep working on the first click.
3. **Glyph.** `pane_identity_icon`: non-shell runtimes → `message-square.svg`. Header: extract the three-way choice at `:450`–`:456` into a pure `fn workspace_header_icon(grouped: bool, focused_shell: bool) -> &'static str` and return `message-square.svg` for the single-chat case; `square-split-horizontal.svg` and `square-terminal.svg` unchanged.
4. **Docs.** `docs/tests/64-terminal-as-pane-option-smoke.md` section 4 (`:46`) gains one line: in a split, the focused pane has no border and unfocused panes are dimmed, identity lines excluded. The spec stays in `docs/features/`; PR 2 archives it.

## Rules of the road

- No change to the divider, the gutter, the layout model, the picker, `⌘D` / `⇧⌘D`, the sidebar icons, the mission workspace, or the drawer. No settings. Nothing from spec phases 3–5.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the Runner app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (with `--features updater` too), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `feat(ui): split panes — no focus border, unfocused panes faded, chat glyph (570, PR 1)`, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `pane_body_opacity`: `(true, false)` → `UNFOCUSED_PANE_OPACITY`, `(true, true)` → `1.`, `(false, false)` → `1.`; the constant pinned at `0.7`.
- `pane_identity_icon(Some("codex"))` and `Some("claude-code")` → `message-square.svg`; `Some("shell")` → `square-terminal.svg`; `None` → `square-dashed.svg` (update `:2799`).
- `workspace_header_icon`: `(true, _)` → `square-split-horizontal.svg`, `(false, true)` → `square-terminal.svg`, `(false, false)` → `message-square.svg`.
- Existing suites stay green: `cargo test -p runner-app`.

## Jason's smoke test (after landing)

1. Two live chats in a split: no border on either pane; the unfocused grid, its cursor and any selection are dimmed; the focused pane is at full brightness; clicking the dimmed pane swaps the fade at once.
2. Both identity lines show the chat glyph at full strength; the unfocused pane's status dot stays bright while its agent works.
3. Three panes: two dimmed. Single-pane tab: nothing dims, and its header title shows the chat glyph.
4. A pane on its ended card, unfocused: the card dims with the grid; Resume works on the first click. An empty stub, unfocused: dimmed; New chat works on the first click.
5. Runner Light: the fade reads as a light haze. A terminal-only tab and every sidebar row look exactly as before.
