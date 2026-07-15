# Armed tab completions and topbar pill removal

## Status

Planned. Tracks issue [#296](https://github.com/yicheng47/runner/issues/296) / spec [39](../features/39-chat-working-unread-indicators.md). No design gate: the sidebar indicator is unchanged and the topbar change is a removal.

## Problem

Feature 39's unread dot fires on every busy→idle settle of a tab member (`TauriSessionEvents::status` → `record_session_completion`), and busy is inferred from any PTY byte with a 750ms silence threshold (`IdleDetector`). That makes "completion" mean "the PTY emitted at least one byte and then went quiet", so two classes of non-activity mint unread dots on tabs the user already viewed: spontaneous background output (CLI notification lines, MCP reconnect messages, statusline redraws, OSC title updates, bells) and the busy seeding on spawn/resume (`spawn.rs:801`, `:1187`) settling after the user looks away. Local keystroke echo and resize repaints are already guarded (`suppress_local_input_busy`, resize grace); spontaneous output is the unguarded path.

Separately, the chat topbar still renders a per-session busy/idle status pill (plus a `n/m busy` group variant while split). Every state it shows is covered by a more authoritative surface — the terminal's own UI for busy/idle, the per-pane overlays and `exit N` meta for stopped/crashed/resuming, the sidebar attention indicator across tabs — and it runs on the same byte-silence heuristic, so it reads "idle" while an agent is blocked on a permission prompt. Feature 39 replaced always-on lifecycle status with attention-only signaling in the sidebar (key decision 1); the pill survived only because that spec scoped the topbar out.

## Key Decisions

1. **Completions require an armed episode.** A busy episode records a completion only if a user turn started it. Arming points, all in `SessionManager`: `inject_direct_stdin` when the write is a submit (`bytes == b"\r"`, the xterm Enter path already tagged `input-submit`), `inject_paste` (paste-then-Enter, the first-turn fallback and router delivery), and spawn when the first turn is delivered via argv (`delivered_via_argv`). State is one `completion_armed: bool` on `SessionState`, beside `suppress_local_input_busy`.
2. **Consume at the tab level, at the record point.** `record_session_completion` keeps its shape: resolve the owning tab, return early while any member is busy. At the record point, take-and-clear `completion_armed` across all members (`SessionManager::take_completion_armed(&member_ids) -> bool`); when none were armed, commit the transaction without touching watermarks and emit nothing. Taking only at the record point is load-bearing: in a multi-pane tab, an armed member that settles early while a sibling is still busy must keep its flag so the tab's final settle still dots.
3. **Spinner semantics unchanged.** Busy/idle inference, thresholds, spawn/resume seeding, the `session/status` event, and the sidebar working spinner stay exactly as they are — only the unread watermark write is gated. Spec 39 decision 6 (idle as the settle heuristic) still holds; arming scopes which settles count as attention-worthy.
4. **Spawn and resume no longer mint completions by themselves.** Seeded busy is unarmed by construction, so resuming a chat in the background or starting one without a prompt produces no dot when the banner settles. A fresh chat that does carry a first turn is armed by the argv or paste path, so "start a chat with a prompt and switch away" still dots on completion.
5. **Accepted edges.** Enter on an empty prompt arms a no-op episode (the user just interacted with that tab, so the settle is almost always recorded viewed); Ctrl+C does not arm, so an interrupt's redraw settle stays silent; a session's armed flag dies with its manager state on exit, preserving spec 39's rule that stop/crash/archive never synthesize completions; the router's `inject_paste` arming of mission sessions is harmless because `record_session_completion` finds no owning tab for them.
6. **Remove both topbar pills, keep the dots.** Delete the single-chat busy/idle/resuming pill and the split `n/m busy` group pill from the RunnerChat header. Keep: the pane-header dots in split view (scanning aid across a grid), the sidebar attention indicator, the per-pane stopped/crashed/resuming overlays, the archived chip, and `exit N` in the meta line. `summarizeDirectChatGroupStatus` loses its only consumer and is deleted with its tests; `directChatDisplayStatus` stays (pane dots, sidebar `SessionRow`).

## Non-Goals

- Changing the 750ms idle threshold, resize grace, or any busy/idle detection mechanics.
- Runtime-specific semantic completion parsing or agent-reported completion (spec 39 out of scope).
- Sidebar indicator redesign, per-pane unread markers, or mission-row status changes.
- Removing the pane-header dots or the sidebar `SessionRow` lifecycle dot.

## Implementation Phases

### Phase 1 — backend arming

- Add `completion_armed: bool` to `SessionState` (`src-tauri/src/session/manager/mod.rs:453`), default false.
- Set it in `inject_direct_stdin` on a successful submitted write, in `inject_paste` after the Enter send succeeds, and in the spawn path when `delivered_via_argv` is true.
- Add `SessionManager::take_completion_armed(&self, session_ids: &[String]) -> bool` — read-and-clear across members, true if any was armed.
- Gate `record_session_completion` (`src-tauri/src/commands/tab.rs:158`): after the busy-member early-return, call `take_completion_armed`; when false, commit and return without recording or emitting.
- Tests (`session/manager/tests.rs`, `commands/tab.rs`): spontaneous settle records nothing; submit→settle records; armed member settling early while a sibling is busy still records on the final settle; resume-seeded settle records nothing; argv first-turn settle records; the flag is consumed (a second, unarmed settle after an armed one records nothing).

### Phase 2 — topbar pill removal

- `src/pages/RunnerChat.tsx`: remove the pill JSX (`:1563-1583`) and the now-dead derivations — `chatState`/`ChatState`, `statusLabel`, `statusBadgeClass`, `statusDotClass`, `groupStatus`, `groupStatusLabel`, `groupStatusDotClass`, `groupStatusBadgeClass`. Audit remaining `displayStatus` consumers and drop the binding if the pill was its last use; `paneStatusFor` and the overlays stay.
- `src/lib/directChatStatus.ts`: delete `summarizeDirectChatGroupStatus`; prune its cases from `directChatStatus.test.ts`.

### Phase 3 — validation

- `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace`.
- Manual pass over the verification list below in a dev build.

## Verification

- [ ] View a chat, switch tabs, trigger spontaneous output in it (notification line, MCP reconnect): the spinner may pulse, but no unread dot appears after it settles.
- [ ] Submit a prompt, switch tabs before it finishes: dot appears on settle; activating the tab clears it.
- [ ] Submit a prompt and stay on the tab: no dot.
- [ ] Resume a stopped chat from the sidebar without opening it: no dot when it settles.
- [ ] Start a new chat with a first prompt and switch away: dot on completion.
- [ ] Multi-pane tab: submit in one pane, switch away while another pane is still busy; the dot appears only after the last member settles.
- [ ] Topbar shows title + CHAT/GROUP chip + meta with no status pill, single-pane and split; stopped/crashed overlays, resuming overlay, archived chip, and `exit N` meta unchanged.
- [ ] Sidebar working spinner and pane-header dots behave as before.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` clean.

## Relevant Code

- `src-tauri/src/session/pty_runtime.rs:288` — `IdleDetector` (unchanged; context for the heuristic).
- `src-tauri/src/session/manager/mod.rs:453` — `SessionState` (new flag), `:662` — `publish_direct_activity` (unchanged dedupe/suppression), `:310` — idle-transition hook into completion recording.
- `src-tauri/src/session/manager/output.rs:190` — `inject_direct_stdin` submit detection, `:284` — `inject_paste`.
- `src-tauri/src/session/manager/spawn.rs:122` — argv first-turn delivery flag, `:801`/`:1187` — busy seeding (unchanged).
- `src-tauri/src/commands/tab.rs:158` — `record_session_completion` gate point; existing watermark tests at the bottom of the file.
- `src/pages/RunnerChat.tsx:1346-1367`, `:1425-1445`, `:1563-1583` — pill derivations and render.
- `src/lib/directChatStatus.ts`, `src/lib/directChatStatus.test.ts` — group summary deletion.

## References

- Issue #296 — bug: unread dots appear on viewed chat tabs without new activity; drop topbar idle/busy pill.
- Spec `docs/features/39-chat-working-unread-indicators.md` — the attention model this fix tightens.
- Archived spec `docs/features/archive/13-pty-silence-idle-detection.md` — origin of the byte-silence heuristic.
- Archived impl `docs/impls/archive/0014-direct-chat-response-status.md` — origin of the topbar status pill.
