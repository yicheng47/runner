# Mission brief — M6.10 + M6.11: sidebar overflow, first-paint pill

Drafted 2026-08-22 02:05. Start with `mission_start` on crew `codex-crew` (`01KVG1NCM4RJC23Q4ZQ5Z8SBHH`), project `runner` (`01KZGS24S5103N6B2HV72ZG1AD`), cwd the `runner-gpui` worktree, title `M6.10 + M6.11 — Sidebar overflow, first-paint pill`, the text below as `goal_override`. No Sparkle update while the crew runs.

---

Implement **M6.10 and M6.11** together (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.10, §M6.11 — read both in full first; they carry the traces with file:line). Two small, unrelated fixes in the app crate, one mission, one feature branch in this worktree (no separate worktree). `gpui-nightly` is at `49825f6`; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. Standing GPUI rules in `impl_log.md` apply (notify scope, no stateful `cx.new` in render, `current_view()` only while rendering, and the new one: inside an `overflow_y_scroll` container children must be content-sized).

## M6.10 — sidebar sections overflow into the Settings footer

Symptom: with six projects expanded, the CHATS & MISSIONS header is pushed to the bottom of the sidebar and its rows paint over the Settings footer row; no scrollbar appears.

Cause (`crates/runner-app/src/surfaces/sidebar.rs`, `render_sidebar_contents` ~1563–1916): the entity root is `min_h_0 flex_1` holding one `overflow_y_scroll` container `#sidebar-node-scroll` (native bar hidden via `scrollbar_width(0)`, custom `Scrollbar::app` overlay fed by the handle's `max_offset`). Inside it, PINNED is content-sized but the PROJECTS section and the CHATS & MISSIONS section (~1736, `.mt_5().min_h(px(0.)).flex_1()`) and the chats row scope `#root-chat-scope` (~1807, `.min_h(rems(28./16.)).flex_1()`) are `min_h_0 flex_1`. Inside a scroll viewport a `min_h_0 flex_1` child is sized to the viewport, never to its content, so the content height never exceeds the container: `max_offset` stays 0, the scrollbar hides itself, and the squeezed chats section's unclipped rows paint past the container and the footer.

Parity (decision 1: `main` is the source of truth): `main`'s sidebar (`git show main:src/components/Sidebar.tsx`, line 2085) is one `flex min-h-0 flex-1 flex-col overflow-y-auto` column; its sections are `flex flex-1 flex-col` **without** `min-h-0` (2264, 2316), so CSS keeps them at content height and the column scrolls as one list. Match that:

1. Remove `min_h(px(0.))` from the PROJECTS and CHATS & MISSIONS sections and the `min_h(rems(28./16.))` floor from `#root-chat-scope` inside the scroll container; keep `flex_1` so a short list still fills the column. Verify with `max_offset` that the scroll range equals content minus viewport once the content is taller than the sidebar.
2. Add `overflow_hidden()` on the entity root column (the `min_h_0 flex_1 flex_col pb_3` div returned by `render_sidebar_contents`) as a belt-and-braces clip so nothing can paint over the footer again whatever the content does.
3. The custom scrollbar must appear when the content overflows and track the wheel and the thumb drag; the sidebar resize handle, ⌘K search and the footer stay reachable.
4. Do not turn the sections into two independently scrolling panes — that is explicitly not parity; out of scope.

Tests: a layout-level test if the crate has a pattern for measuring elements (look for existing sidebar tests and `gpui::TestAppContext` use); otherwise a unit test on whatever pure helper you extract. At minimum prove in the handoff with numbers: content height, viewport height, `max_offset` after the fix on a short window with six projects.

## M6.11 — the Starting/Resuming pill settles before the TUI paints

Symptom: the Starting pill disappears, then the Claude session takes a few more seconds to appear over a blank alternate screen.

Cause: `chunk_indicates_tui_ready` (`crates/runner-terminal/src/terminal.rs:~706`) marks the TUI ready on the first `ESC[?2004h` / `?1049h` / `?47h`, and `chat_lifecycle::transition_should_settle` (`crates/runner-app/src/surfaces/chat_lifecycle.rs`, call sites `chat.rs:~166` and `mission_workspace.rs:~1412`) drops the pill as soon as `tui_ready_seq` passes the transition's `baseline_seq`. Claude Code's fullscreen renderer — Runner's default since M6.6 — writes `?1049h` at process start, seconds before its first frame; the default renderer's `?2004h` arrived with the first frame, so the old timing was a coincidence.

Fix — a first-paint signal from the native seam:

1. In `TerminalSession::feed_output`, after `parser.advance`, until a first paint has been recorded for the current child: check whether any visible line of the `Term` carries non-whitespace (`replay::visible_lines` or a cheaper `grid().display_iter()` scan that stops at the first non-blank cell) and record `first_paint_seq = seq`. Reset it when the `Term` is replaced on respawn (M6.8's registry replacement path) so a Resume measures its own first paint. One walk per chunk until the first paint, then no cost.
2. Expose `first_paint_seq` on `TerminalOutputActivity`; `transition_should_settle` settles `Starting` and `Resuming` on `first_paint_seen` (first paint after the baseline) instead of `ready_signal_seen`. Keep `TRANSITION_HARD_TIMEOUT` (10 s) and the output-idle rule as backstops; keep `tui_ready_seq` in place for any other reader (grep before touching it).
3. A shell chat settles on its prompt, a claude chat settles on its first frame under both renderers (`/tui default` and fullscreen), codex on its first frame; resume behaves the same. Tests in `chat_lifecycle.rs` (pure function) and a `runner-terminal` test feeding bytes: `?1049h` + `2J` alone does not set `first_paint_seq`; the first printable cell does; respawn resets it.
4. Check, do not change unless broken: the backend's first-turn delivery under the fullscreen renderer (`CLAUDE_LAUNCH_GATE_GRACE` 1500 ms plus the verified-paste capture loop in `session/manager/output.rs`) — start a mission with a claude lead and confirm the brief lands in the input box, not into a Claude that has not drawn it yet. Report what you saw.

## Gates

- `make verify` green; `git diff --check` clean.
- Handoff: one impl_log paragraph per item (the human writes the log), the M6.10 numbers, and a daily-drive checklist: six expanded projects + a dozen chats on a short window (scrollbar visible, footer clean, wheel and thumb drag work, resize handle reachable); start a claude chat under fullscreen and under `/tui default` (pill stays until the first frame, disappears on it); a shell chat; a codex chat; resume a stopped claude chat; start a mission with a claude lead (brief lands in the input box).
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) any remaining `min_h_0`/`min_h(...)` on a child of `#sidebar-node-scroll`; (2) the scrollbar overlay's `max_offset` math after the change, wheel + drag; (3) the root clip hiding something that must overflow (context menus, tooltips, the drag ghost — check they are rendered outside the clipped root via overlays); (4) first-paint false positives — a cursor-only or SGR-only chunk must not count, a line of spaces must not count; (5) first-paint not reset on respawn, or `baseline_seq` semantics broken for Resume; (6) the 10 s timeout or idle rule regressed; (7) the per-chunk grid walk surviving past the first paint (cost), or taking the `Term` lock re-entrantly; (8) first-turn paste racing the fullscreen renderer.
