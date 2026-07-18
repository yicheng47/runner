# 0031 — Rust-native UI rewrite: approach plan

## Status

Planning. Tracking issue [#307](https://github.com/yicheng47/runner/issues/307). Work happens on the persistent `rust-ui-nightly` branch during the exploratory phase, then folds into `main` in-tree (see [Branch strategy](#branch-strategy)).

## End state

One native Rust binary. No webview, no JS, no Tauri. The UI is rendered by a Rust GUI framework with `alacritty_terminal` as both the terminal model and the renderer's buffer — one parser, one grid, no serialization boundary. This dissolves the JS/Rust parity tax, the WebGL glyph-atlas bug class, and the WKWebView GPU-context limits by construction (#307).

"Drop Tauri" is the end state, not step one. Tauri currently provides, beyond the webview: app packaging/codesigning/notarization, the updater, native dialogs (folder picker), multi-window management + position restore, logging, process restart, and URL opening. Each needs a native replacement (Phase 5) before Tauri can actually be deleted.

## What we keep vs. rewrite

Keep (~33k LOC Rust, UI-agnostic already):

- `src-tauri/src/session/` — PTY session manager (`portable-pty`).
- `src-tauri/src/router/`, `src-tauri/src/event_bus/`, `src-tauri/src/repo/` + `db.rs` (SQLite), `src-tauri/src/mcp/`, `model.rs`, `error.rs`.
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

- **Phase 1 (spike)** lives on `rust-ui-nightly`. Messy, throwaway-friendly, rebase allowed while private.
- **Phase 2 (core extraction)** lands on `main` — it is a behavior-preserving refactor the current app benefits from (testability, thinner command layer) and it shrinks the branch's conflict surface to near zero.
- **Phase 3+** adds the native binary as a new workspace member (`crates/runner-native/` or similar) — additive code. Fold it into `main` behind a second binary target as soon as the walking skeleton is real, per the pattern-2 discussion: `rust-ui-nightly` then becomes a release channel (nightly builds of the native binary from main), not a diverging code line.
- While the branch exists: merge `main` → branch on cadence, never the reverse until a phase is complete; cherry-pick incidental fixes to `main` immediately.

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

### Phase 2 — Extract the app core (on main)

Create an app-core crate (name TBD, e.g. `crates/runner-app/`): move the 85 command bodies to plain async fns over `Db`/`SessionManager`/etc.; `#[tauri::command]` wrappers become one-liners. Swap event-bus fanout to a broadcast channel with the Tauri emitter as a subscriber. No behavior change; existing tests are the contract (same shape as impl 0021's repo-layer move).

This is the strangler seam: after it, the native UI and the Tauri app are two thin frontends over one core, and the rewrite stops touching anything `main` churns.

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

- **Parity treadmill** — the killer risk. `main` keeps growing while nightly chases it. Mitigations: Phase 2 seam (backend features land once, in the core, both UIs get them), dogfood-ordered slices (pressure to finish is intrinsic), and a soft feature-freeze on new *frontend* surface once Phase 3 lands.
- **Scale honesty**: this replaces 28k LOC of UI. Solo with agent crews, expect months of part-time sessions, not weeks. The phase gates exist so the project survives motivation dips — every phase ends with something you use.
- **GPUI churn/docs**: mitigated by the spike and by Zed's terminal code as a living reference; iced is a genuine fallback, not a fig leaf (cosmic-term proves it).
- **Packaging/notarization** first-time cost: mitigated by doing it once in Phase 1, not at the end.
- **Accessibility/native-behavior gaps** (VoiceOver, standard text-editing shortcuts) are real regressions vs. a webview; accepted for a personal-IDE product, noted for honesty.

## Non-goals

- Windows/Linux support (premature for a personal IDE; GPUI choice reflects this).
- Feature additions during the rewrite — parity only, divergence recorded as issues.
- Rewriting the backend, CLI, event-log format, or SQLite schema — explicitly frozen interfaces for this effort.
