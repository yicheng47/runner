# Mission brief — M6.13: feed text fidelity — font weights, Markdown rhythm, selection/copy

Drafted 2026-08-22 15:40 from Jason's screenshot of the M6.9 mission goal in the feed ("too compacted", "should use markdown", "cannot select the feed's text"). Runs **after M6.9 lands** — same crew (`codex peer`, `01K000DEFAULT000PEERCODING01`), project `runner` (`01KZWQA969B26J6WSP68RZP4Y9`), cwd the `runner-gpui` worktree, title `M6.13 — Feed text: font weights, Markdown rhythm, selection`, the text below as `goal_override`.

---

Implement **M6.13** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.13 — read it first). App crate only, one feature branch in this worktree. Standing GPUI rules in `impl_log.md` apply (notify scope, no stateful `cx.new` in render, `current_view()` only while rendering — the M4.8 nightly crashed from a `current_view()` inside a feed-Markdown hover listener, so the listeners here take the owning `EntityId`; content-sized children inside `overflow_y_scroll` containers). **`runner-app` is the binary, `runner-backend` the core.** `main` is the source of truth for rendering (decision 1): `src/components/MessageBody.tsx`.

Three items, in this order — A is the root cause behind most of what the screenshot shows.

## A — Font weights never render (app-wide)

`assets/fonts/Inter-Variable.ttf` is registered through `cx.text_system().add_fonts` (`crates/runner-app/src/main.rs` ~949). gpui-ce 0.3.3's macOS text system (`~/.cargo/registry/src/*/gpui-ce-0.3.3/src/platform/mac/text_system.rs:112-148`) resolves a `Font { weight, style }` by `font_kit::matching::find_best_match` over the **static properties of the loaded faces**; a variable TTF loaded via `CGFont::from_data_provider` is one face at its default instance (Regular 400), and GPUI never sets the `wght` axis. So `FontWeight::MEDIUM / SEMIBOLD / BOLD` all resolve to Regular: feed headings, `**bold**`, the sidebar brand "Runner" (600 in the design), dialog titles, section labels — all flat. Visible in Jason's screenshots of the running nightly.

Fix: replace the variable file with Inter 4.x **static instances** — Regular, Medium, SemiBold, Bold and their Italics (rsms/inter release, `extras/ttf/`; OFL already bundled as `OFL.txt`) — registered together so `find_best_match` has real candidates. Keep the family name `Inter` so `theme`, `app_font_family` and the Appearance picker keep working; check the picker does not list duplicates. Grep `FontWeight::` to confirm which weights are actually used and drop unused files if the bundle matters (~300 KB each). Prove it: a test that `text_system().font_id(Font { family: "Inter", weight: SEMIBOLD, .. })` differs from the Regular face's id, or a documented manual check if the test text system cannot load CoreText faces.

## B — Markdown rhythm and inline nesting (`surfaces/mission_markdown.rs`)

Match `MessageBody.tsx` (Tailwind values, 13 px body): `p` `my-1.5` (6 px top and bottom — native has `gap_1`, 4 px, between all blocks and no paragraph margin), `leading-relaxed` 1.625 (native 20 px is close enough); `h1` `mt-3 mb-1.5 14px semibold fg`, `h2` `mt-3 mb-1 13px semibold fg`, `h3` `mt-2 mb-1 13px semibold fg-2` (native: every heading `mt_2 mb_1`, h2/h3 identical); `ul`/`ol` `my-1.5 pl-6 space-y-1` with `li leading-relaxed` (native `pl_4`, no gap between items); `blockquote` `my-2 border-l-2 pl-3 fg-2`; code block `my-2`, inner `px-3 py-2 12px leading-relaxed`; inline code `rounded bg-raised px-1 py-px 12px accent`; `hr` `my-3 border-line` (parser has no `hr` — add `---`/`***`); `table` `my-2 12px`; `strong` semibold `fg`; `em` italic `fg`. Collapse adjacent margins the way CSS does (a heading after a paragraph gets `max(mb, mt)`, not the sum) — easiest by giving blocks only top margins plus one bottom pad on the body.

Inline nesting: `append_styled_segment` (~431) finds the first marker, then appends the marker's body with `append_highlighted` **without re-scanning it**, so `**\`runner-app\` is the binary, \`runner-backend\` the core.**` renders the backticks literally inside the bold span (visible in the screenshot), and `*see \`x\`*` likewise. Recurse into the body with the merged base style; inside a code span nothing is parsed; inside bold/italic, code keeps its colour/background and drops the weight. `**` vs `*` at the same index must keep resolving to `**` (the candidate order does that today — pin it with a test). Keep the registered subset deviation (no nested lists, headings 4–6, bare-URL autolinks) unless it falls out for free.

Tests: parser tests for nested inline and `hr`; a layout-level spacing test in the `SidebarScrollLayoutTest` style (two paragraphs → 12 px between them; heading after paragraph → 12 px, not 18).

## C — Text selection and ⌘C in the feed

`main` gets DOM selection for free; the native feed has none (`InteractiveText` is click/hover only). Add per-message selection:

- State: `FeedSelection { event_id, anchor: (block, byte), head: (block, byte) }` on the workspace (not a global; one window can show one mission) — survives re-render and feed scrolling because it is keyed by event id + block index, not by element.
- Input: mouse down on a message body sets the anchor and starts a drag; move extends the head; up ends. Map pointer → index with the layout primitives `TextLayout::index_for_position` / `position_for_index` / `line_layout_for_index` (`gpui-ce-0.3.3/src/elements/text.rs:483-560` — what Zed's `markdown` crate builds its selection on); a block owns its `InteractiveText` layout, so the workspace needs the per-block layouts reachable from the listener (the M4.8 `render_markdown(id, text, view, cx)` shape already threads ids and the view). Listeners `cx.notify(view)` — never `current_view()`.
- Paint: `StyledText::with_highlights` background ranges in the selection colour the terminal uses (M4.7), so the two surfaces agree. No per-frame allocation when nothing is selected — only the selected message builds ranges.
- Copy: ⌘C with a selection copies the plain rendered text (not Markdown source), blocks joined with `\n`, list markers as rendered; `cx.write_to_clipboard(ClipboardItem::new_string(..))`. Key context `MissionFeed` on the feed container so the registry entry does not collide with the `Terminal`-scoped ⌘C from M4.7 or text fields' own. I-beam cursor over message text; click elsewhere in the feed or Escape clears; switching mission or tab clears.
- Out of scope (say so in the handoff if skipped): selection across messages, double/triple-click word/paragraph selection, drag autoscroll at the feed edges — take any of them only if they fall out cheaply.

Tests: pure mapping tests (block/offset ordering, copy-text assembly across blocks and list items); a `TestAppContext` drag simulation if the harness exposes mouse events, otherwise a handoff video/checklist.

## Gates

- `make verify` green; `git diff --check` clean.
- Daily drive on the M6.9 mission goal in the feed: bold visible and headings distinct (A), paragraphs and list items breathe like `main` (B), `runner-app` inside the bold sentence renders as code (B); select a sentence, ⌘C, paste into a slot terminal (C); brand mark "Runner" semibold in the sidebar (A); a terminal's own ⌘C and a text field's ⌘C unaffected.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) a static face missing or a family-name mismatch (the Appearance picker showing two Inters, or italic falling back to synthetic slant); (2) the OFL notice not covering the new files; (3) the inline recursion looping or panicking on unbalanced markers (`**a`, `` `a**b` ``); (4) margin sums instead of collapsed margins; (5) selection highlights baked into a cached `StyledText` and stale after the text changes; (6) ⌘C stealing from a focused terminal or text field, or the feed's context swallowing other registry entries; (7) `current_view()` inside any mouse listener; (8) highlight vectors built for every message every frame.
