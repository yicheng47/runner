# 0031 — Rust-native UI rewrite: approach plan

## Status

In progress. Phases 1 and 2 are complete; Phase 3 is implemented on `phase3-walking-skeleton` pending review, human smoke-test confirmation, and merge back to the persistent `gpui-nightly` branch. Tracking issue: [#307](https://github.com/yicheng47/runner/issues/307). See [Branch strategy](#branch-strategy).

## End state

One native Rust binary. No webview, no JS, no Tauri. The UI is rendered by a Rust GUI framework with `alacritty_terminal` as both the terminal model and the renderer's buffer — one parser, one grid, no serialization boundary. This dissolves the JS/Rust parity tax, the WebGL glyph-atlas bug class, and the WKWebView GPU-context limits by construction (#307).

"Drop Tauri" is the end state, not step one. The Tauri app on `main` currently provides, beyond the webview: app packaging/codesigning/notarization, the updater, native dialogs (folder picker), multi-window management + position restore, logging, process restart, and URL opening. Each needs a native replacement (Phase 5) before the Tauri app can be deleted from `main` at cutover; its trees are already absent from the pioneer branch.

## What we keep vs. rewrite

Keep (~33k LOC Rust, now extracted into UI-agnostic crates):

- `crates/runner-app/src/session/` — PTY session manager (`portable-pty`).
- `crates/runner-app/src/router/`, `event_bus/`, `repo/`, `db.rs` (SQLite), `mcp/`, `model.rs`, and `error.rs`.
- `crates/runner-core/`, `cli/` — untouched.
- The SQLite schema and NDJSON event-log format — the native app reads the same data; you can switch between old and new app against the same state.

Rewrite (~28k LOC TS/React):

- 7 pages (`Runners`, `RunnerDetail`, `RunnerChat`, `Crews`, `CrewEditor`, `MissionWorkspace`, `SettingsPage`), ~30 components + `settings/` + `ui/`, the xterm.js terminal, the layout/pane store, sidebar (projects / folders / tabs), modals, command palette.
- Design system: Tailwind tokens → theme constants in Rust. The `.pen` files remain the visual source of truth; Tokyo Night + light themes carry over as data, not CSS.

Adapt (the seam, Phase 2):

- 85 `#[tauri::command]` handlers → plain async fns on an app-core crate.
- 36 `emit` sites in `event_bus` → a `tokio::sync::broadcast` channel; the Tauri layer becomes one subscriber that forwards to the webview (until it's deleted), the native UI another.

## Framework decision

Spike GPUI first; iced is the fallback. Do not evaluate all four candidates in depth — two is enough, both have production terminal prior art:

- **GPUI** (Zed's framework, Apache-2.0, on crates.io): Zed's terminal *is* `alacritty_terminal` rendered by GPUI — literally the architecture #307 describes, with production-grade reference code (`crates/terminal` + `terminal_view` in the Zed repo). Metal-native on macOS, proven multi-window, proven CJK/IME text input, Tailwind-like styling API. Risks: thin docs, API churn, smaller out-of-tree community. License note: `gpui` is Apache-2.0 (fine as a dependency), but Zed's `terminal`/`terminal_view` crates are GPL — architectural reference only, no code copying (cosmic-term likewise GPL; verify crate licenses at spike start).
- **iced** (Elm-architecture, wgpu): cosmic-term is `alacritty_terminal` on iced (System76, shipping), `iced_term` crate exists. More conventional, better docs. Risks: text-input/IME maturity below Zed's, retained-widget model may fight the pane-tree layout.
- egui (immediate mode) and Slint (markup DSL): ruled out for v1 — egui's IME/text-editing maturity is the concern for a chat-input-heavy app with Chinese input; Slint's DSL adds a layer for no benefit here. Revisit only if both spikes fail.

Runner is macOS-only today and that stays true for this rewrite (non-goal below), so GPUI's macOS-first posture is not a cost.

## Branch strategy

Revised 2026-07-19 by Jason's decision, superseding the original fold-into-main-behind-a-second-binary plan: `gpui-nightly` is a **Tauri-free pioneer branch**. It shares the backend with the Tauri app on `main` and carries the native UI; the two frontends never coexist in one tree.

- **Phase 1 (spike)** lived on `phase1-terminal-spike` off `gpui-nightly`. Messy, throwaway-friendly, rebase allowed while private.
- **Phase 2 (core extraction)** is commit `461356e` on `refactor/0031-app-core`. PR #310 reads merged, but `main` was reset afterward and does not contain the extraction; the Tauri app on `main` therefore still owns the unextracted backend. Nightly composition sources `crates/runner-app` by merging the extraction branch directly.
- **Nightly composition** is delivered on `phase3-walking-skeleton` pending merge back after human verification. It merges current `origin/main` plus the Phase 2 extraction, then deletes `src/` (React), `src-tauri/`, and the JS toolchain. The branch keeps only the shared backend (`crates/runner-app`, `crates/runner-core`, `cli/`) plus `crates/runner-native/`; the superseded `crates/native-spike/` is removed.
- Merge direction stays `main` → nightly on cadence, never the reverse until cutover; cherry-pick incidental fixes (including backend fixes authored on nightly) to `main` immediately. **Accepted cost**: until the extraction re-lands on `main`, main-side backend changes originate under the deleted `src-tauri/` tree and require an explicit hand-port into `crates/runner-app` during cadence merges. Every modify/delete conflict is resolved by keeping the deletion only after checking for that backend half.
- **Cutover (Phase 6)** becomes a merge of nightly into `main` — the deletion arrives with the merge — rather than an in-place deletion commit on main.

Build decision: the pioneer line has Rust-only `fmt`, `clippy`, `test`, and `run-native` Make targets. CI is one macOS Rust-workspace job for pushes and pull requests targeting `gpui-nightly`, including the GPUI Metal Toolchain component; native release packaging remains deferred to Phase 5.

## Phases

### Phase 1 — Terminal spike + framework decision (timeboxed)

Build the riskiest 5% first: a window rendering a live claude-code session via `alacritty_terminal` on GPUI. Wire: spawn via the existing session manager → PTY bytes → `alacritty_terminal::Term` → draw the grid.

Exit criteria (all must pass, else repeat once on iced):

1. Smooth scroll/redraw under busy claude-code output (spinners, alt-screen, streaming) — no visible tearing or lag at typical window sizes.
2. Correct CJK width, emoji, box-drawing — the glyph classes that produced the xterm garble bug.
3. IME input works in a native text field (Chinese input is a hard requirement for the chat composer).
4. Resize reflow behaves (the ring-purge/reflow class from #308).
5. `cargo bundle`-style .app assembly + codesign + notarize a hello-world build once, to confirm the packaging path exists before betting on it.

Deliverable: a go/no-go decision memo appended to this doc, plus the spike code on the branch.

Also in this phase: start the **terminal fixture corpus** spec 42 promised but never built (#306 closed) — record real claude-code/codex PTY byte logs, snapshot `alacritty_terminal` grid state as the regression suite. The spike renders these fixtures; every later terminal change replays them.

### Phase 2 — Extract the app core

Implemented as `crates/runner-app/` at `461356e` on `refactor/0031-app-core`: the 85 command bodies are plain async fns over `Db`/`SessionManager`/etc.; the extraction branch's `#[tauri::command]` wrappers are one-liners. Event-bus fanout uses a broadcast channel with the Tauri emitter as a subscriber. The extraction remains off `main` after its post-merge reset and is consumed directly by the pioneer branch.

This is the intended strangler seam. The pioneer branch uses it now; the Tauri line gains the two-thin-frontends benefit only if the extraction is deliberately re-landed on `main`.

### Phase 3 — Walking skeleton

`runner-native` binary: boots the app core against the same SQLite DB + logs dir, shows a minimal sidebar listing existing chats, opens one direct chat with a live terminal and working composer (IME included). One vertical slice, end to end, dogfoodable for a single chat.

### Phase 4 — Parity slices, in dogfood order

Each slice ships when it's daily-drivable; use it for real work before starting the next:

1. Direct chats: tabs, panes, layout picker, working/unread indicators, session resume.
2. Sidebar: projects, folders, tab accordion, drag-reorder, rename, archive.
3. Missions: event feed, mission workspace terminals, signals, mission lifecycle.
4. Runners/Crews CRUD: list pages, editors, modals, pagination/search.
5. Settings: full-page settings, shortcut rebinding, themes, zoom.
6. Multi-window + command palette + polish.

The native app reads the same DB, so partial parity is usable: live in the native app for chats while missions still happen in the Tauri app.

### Phase 5 — App-shell services (replace what Tauri gave for free)

- Packaging: .app assembly, codesign, notarize, staple, DMG — scripted (`cargo-bundle` or hand-rolled; validated in Phase 1's criterion 5).
- Updater: replace `tauri-plugin-updater` — GitHub-Releases-driven check + download + swap, or Sparkle. This is real work; do not leave it for cutover week.
- Dialogs → `rfd`; opener → `open` crate; logging → `tracing` + file layer (keep the crash/panic hook); window position restore → reimplement `window_state.rs` on the native windowing layer.
- CI: nightly build + release artifacts for the native binary alongside the existing app.

### Phase 6 — Cutover

Criteria: 2+ weeks daily-driving the native app exclusively, all Phase 4 slices done, updater proven by shipping at least one native-to-native update, fixture corpus green. Then: delete `src/`, `tauri.conf.json`, the Tauri deps and adapter layer; the native binary becomes `Runner.app`; major version bump.

## Risks

- **Parity treadmill** — the killer risk. `main` keeps growing while nightly chases it. The Phase 2 seam mitigates this only after the extraction exists on both lines; while it remains off `main`, cadence merges require hand-porting backend changes from `src-tauri/` into `crates/runner-app`. The other mitigations are dogfood-ordered slices (pressure to finish is intrinsic) and a soft feature-freeze on new *frontend* surface once Phase 3 lands.
- **Scale honesty**: this replaces 28k LOC of UI. Solo with agent crews, expect months of part-time sessions, not weeks. The phase gates exist so the project survives motivation dips — every phase ends with something you use.
- **GPUI churn/docs**: mitigated by the spike and by Zed's terminal code as a living reference; iced is a genuine fallback, not a fig leaf (cosmic-term proves it).
- **Packaging/notarization** first-time cost: mitigated by doing it once in Phase 1, not at the end.
- **Accessibility/native-behavior gaps** (VoiceOver, standard text-editing shortcuts) are real regressions vs. a webview; accepted for a personal-IDE product, noted for honesty.

## Non-goals

- Windows/Linux support (premature for a personal IDE; GPUI choice reflects this).
- Feature additions during the rewrite — parity only, divergence recorded as issues.
- Rewriting the backend, CLI, event-log format, or SQLite schema — explicitly frozen interfaces for this effort.

## Phase 1 decision memo (2026-07-18; finalized 2026-07-19)

**Decision: GO on GPUI.** Human verification: criterion 3 (Pinyin IME in the native composer) confirmed explicitly; criteria 1/4 confirmed through live daily-style use with no tearing, lag, or reflow artifacts reported; criterion 2 machine-verified at the grid level with per-cell-origin rendering making fallback-advance skew structurally impossible. Criterion 5: .app assembly + Developer ID codesign proven; the notarize/staple run is **deferred to Phase 5 by explicit decision** — the failure-prone half (hand-rolled bundle of a bare cargo binary passing codesign with hardened runtime) is validated, notarization is bundle-agnostic, and the same credential flow already ships Runner's Tauri releases via CI. The iced fallback was never needed.

Original evidence table and findings follow.

Spike built on `phase1-terminal-spike` as `crates/native-spike/` (GPUI 0.2.2 + alacritty_terminal 0.26 + portable-pty, ~1.4k LOC). Verification state per exit criterion:

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Busy-output rendering | needs eyes | Live claude session renders; coalesced wakeups (PTY thread → channel → one notify/frame). Human: watch a chatty claude run for tearing/lag. |
| 2 | CJK/emoji/box-drawing width | machine-green, needs eyes | Replay tests assert WIDE_CHAR/spacer/zerowidth/leading-spacer cell semantics; renderer paints wide + non-ASCII glyphs at per-cell column origins so fallback advances can't skew the grid. Human: run `width-torture.sh` in the spike window. |
| 3 | IME in composer | needs eyes | Full `EntityInputHandler` (marked-range composition, candidate-window anchoring via `bounds_for_range`, Enter-guard during composition). Human: Pinyin typing test. |
| 4 | Resize reflow | machine-green, needs eyes | PTY + Term resize wired to element layout; tests pin alacritty's reflow semantics incl. cursor-line pinning and scrollback push (#308 class). Human: violent resize during live output. |
| 5 | .app + codesign + notarize | 2/3 done | `package.sh`: release build → .app assembly → Developer ID codesign validates. Notarize/staple scripted, needs credentials (NOTARY_PROFILE or APPLE_ID trio). |

Fixture corpus (spec 42 debt) exists and gates regressions: real claude-code interactive session, busy alt-screen `top`, glyph width-torture — recorded as raw PTY byte logs, replayed into the grid, snapshot-compared (10/10 green). Every later terminal change replays these.

Findings for the framework decision:

- GPUI worked as advertised: the terminal element (grid → quads + shaped runs) is ~350 lines; IME needed no platform code, just the input-handler protocol. API-churn risk is real (crates.io docs thin; the vendored source + bundled examples were the actual reference) but never blocking.
- One environment cost discovered: gpui's Metal shader build requires the Xcode 26 Metal Toolchain component (one-time ~3 GB download). Recorded in the spike README; CI runners will need it for Phase 3+.
- alacritty_terminal ships no default color palette; the embedder owns the 256-color table (done in `palette.rs`, Tokyo Night base 16). Its `term::test` module (public at runtime) gives `TermSize`/`mock_term` — useful beyond tests.
- No serialization boundary exists anywhere in the pipeline: reader thread locks the Term, advances the parser, UI paints from the same grid under the same lock. The #307 architecture holds.

Deviation from this plan's Phase 1 wire description ("spawn via the existing session manager"): the spike spawns through its own minimal portable-pty lifecycle, per the mission brief's explicit allowance ("PTY wiring may use portable-pty directly for the spike; reuse of the existing session manager is optional"). Consequence stated plainly: this spike validates the rendering/input/packaging risks only. The session-manager/app-core integration seam — canonical PATH/env composition, lifecycle, the query-responder behavior `pty_runtime.rs` already implements — is NOT validated here and remains Phase 2/3 work; the spike's local session code is throwaway and will be superseded by the existing runtime at integration time.

(Superseded by the finalized decision above: original provisional recommendation was go-on-GPUI pending human confirmation of criteria 1–4 and one notarize pass; IME was subsequently confirmed by hand and notarization deferred to Phase 5.)
