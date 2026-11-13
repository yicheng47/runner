# 1290 — Archive linked missions with chat

Tracking issue: [#1290](https://github.com/yicheng47/runner/issues/1290).

## Design decision

Jason approved **Option A** (explicit mission list inside a standard two-button `ConfirmDialog`) and `runner session archive --with-missions` (non-interactive, no double confirm) on 2026-11-13. Design: `runner/desktop/specs/1290-archive-linked-missions.pen` in the private `YeelinksCo/yeelinks_design` repo, revision `0610154`. Approved frames: full-window context `Zyp5L` (`tE7NQ`), entry-point matrix `RRpz0`, Option A `B8grI8` (`HTXqA`), and state frames `g26XDF` — single settled mission `i6GC2j` (`V6hv8T`), running chat/mission `nnozr` (`DDjYM`), split tab with chats + linked missions + shell terminal `LsGiX` (`H9pw8N`), and Runner Light palette `FPVfe` (`L2Iwe`). Options B (`YVwdk`) and C (`yWZ5c`) are not selected.

## Motivation

Under [#748](./archive/748-mission-link.md), archiving a direct chat leaves its linked missions unarchived, dissolves the linked group in the sidebar, and turns each mission header's back-link into `linked to <chat> (archived)`. In addition, choosing **Archive** on a chat tab/group in the sidebar or clicking **Archive** on a stopped chat card currently skips confirmation altogether whenever the tab has no shell terminals. Once a frontman chat finishes, the person almost always wants the missions it drove archived alongside it rather than cleaning up leftover mission rows one by one.

## Scope and behavior

Whenever a desktop or remote UI archive action targets one or more direct agent chats that have unarchived linked missions, Runner opens a confirmation dialog before archiving anything and archives those unarchived linked missions together with the chat(s) on confirm. Keeping a mission active uses the existing `Unlink` action before archiving the chat.

### UI entry points

| Surface | No linked missions | Has unarchived linked missions |
| --- | --- | --- |
| Sidebar **Archive** on tab / linked group (`SidebarMenuAction::ArchiveTab` → `request_archive_all`) | Unchanged: instant archive when the tab has no shell terminals; `Archive tab?` when closing terminals. | Always opens confirmation (`Archive chat and mission?` / `Archive chat and {count} missions?` for a single chat without shells, or `Archive tab?` when combined with multiple chats or shells). |
| Stopped chat card **Archive** (`StoppedChatCard` → `archive_chat_sessions`) | Unchanged: instant archive unless the tab's shells also close. | Always opens confirmation and archives the chat's unarchived linked missions with it. |
| `⌘W` / pane header **Archive chat** (`request_close_chat_pane`) | Unchanged: opens `Archive chat?`. | Opens the combined chat + linked missions confirmation for that pane's chat; other panes in a split tab and their linked missions stay untouched. |
| Split-pane tab **Archive** (`request_archive_all` across multiple chats/shells) | Unchanged. | Collects deduplicated unarchived linked missions across every chat being archived in the tab and includes them in `Archive tab?`. |
| Remote window (companion) | Unchanged. | Resolves linked missions from the remote store (`mission.linked_chat`) so the same prompt opens and archives each linked mission alongside the chat/tab. |

### Confirmation copy and states (Option A)

Reuses `ConfirmDialog`'s existing `.icon()`, `.note()`, and `.detail()` slots (`crates/runner-app/src/ui/overlay.rs`) with two footer buttons (`Cancel` on `Escape`, destructive confirm on `Enter`):

- **Single chat + settled mission(s), no shells (`B8grI8`, `i6GC2j`):**
  - Icon: `archive.svg` (danger variant).
  - Title: `Archive chat and mission?` (1 mission) or `Archive chat and {count} missions?` (2+).
  - Body: `Archiving {label} also archives its linked mission.` / `Archiving {label} also archives its linked missions.`
  - Running override (`nnozr`): when the chat or any linked mission is running, body states what will be stopped first (chat only running, mission(s) only running, or both — for example `{label} and {running_count} linked mission(s) are still running. Archiving stops them first.`).
  - Note (`.note()`): compact raised container listing the unarchived linked missions in sidebar group order, each with a `flag` icon, mission title, and trailing state (`Settled` / `Ended` in muted text, or amber dot + `Running · stops crew` when the mission is running or has live sessions).
  - Detail (`.detail()`): `You can restore both from Settings → Archived.` (singular) or `You can restore chats and missions from Settings → Archived.`
  - Confirm action: `Archive chat & mission` / `Archive chat & {count} missions`, or `Stop & archive all` when stopping running processes (busy label `Archiving…`).
- **Tab archive (`Archive tab?`, `LsGiX`) with multiple chats and/or shell terminals:**
  - Keeps title `Archive tab?` and action `Archive tab` (`trash.svg` when closing terminals, `archive.svg` when archiving multiple chats + missions with 0 terminals).
  - Body combines reversible counts (`{chats} chat(s)` and `{missions} linked mission(s)`) with any irreversible shell count (`permanently closes {terminals} terminal(s)`), plus a running-stop sentence when any targeted chat or linked mission is running.
  - Note lists the linked missions; detail states `Archived chats and missions can be restored from Settings → Archived; closed terminals cannot.` (or omits the terminal clause when `terminals == 0`).

### CLI (`runner session archive`)

- `runner session archive [OPTIONS] <SESSION>` gains `--with-missions` (`crates/runner-cli/src/command/cli.rs`, `session.rs`, `client.rs`):
  - **Default (no flag):** Unchanged — non-interactive, stops (if running) and archives only the direct chat; linked missions stay unarchived as before; output remains `{"session_id": "<id>"}` (and unchanged plain output).
  - **With `--with-missions`:** Non-interactive with **no confirmation prompt**. Stops (if running) and archives the direct chat together with every unarchived mission linked to it (`missions.linked_session_id = session_id AND archived_at IS NULL`, archived via `mission_archive_impl` so running slots and mission drawer shells stop cleanly). JSON output includes `archived_mission_ids` alongside `session_id` (plain output reports archived mission count/IDs when any were archived; succeeds cleanly with empty `archived_mission_ids: []` when the chat has no unarchived linked missions).
- Regenerate the CLI reference with `make guide-cli` (`docs/guide/reference/cli.md` and `docs/guide/zh-CN/reference/cli.md`) and update `crates/runner-cli/tests/golden.rs` help/command fixtures.

### Execution, link retention, and restore

- Archiving a mission through this flow uses the same `mission_archive_impl` teardown as direct mission archive: closes mission drawer shells, kills live mission PTYs, writes `mission_stopped`, sets `archived_at`, unmounts the event bus and router, and emits mission update events.
- Extend the daemon/protocol additively (for example an optional/defaulted `with_missions` flag or dedicated request, keeping wire compatibility with older clients/companions) so CLI `--with-missions` and UI confirm share the same backend behavior where practical; companion remote windows can also fall back to archiving linked missions explicitly when talking to an older host if needed.
- `missions.linked_session_id` is preserved when both sides are archived, matching #748 ("archiving either side keeps it"). Only permanent deletion of the chat clears `linked_session_id`.
- Already-archived missions linked to the chat are ignored by the UI prompt and by `--with-missions`.
- `Settings → Archived` continues to list and restore chats and missions independently.

### Affected documentation

- Guide: `docs/guide/guides/chats-and-windows.md`, `docs/guide/concepts/missions.md`, `docs/guide/guides/drive-runner-from-agents.md` (if it mentions `session archive`), plus `docs/guide/zh-CN/...`, and `make guide-cli` for `docs/guide/reference/cli.md` (+ `zh-CN`).
- Architecture: `docs/arch/arch.md` §9.4 where linked-chat archive behavior is described.

## Implementation phases

1. **Daemon, protocol, and CLI (`runner-core`, `runner-daemon`, `runner-cli`).** Add additive `with_missions` support to direct-session archiving so a single call stops and archives a direct chat plus its unarchived linked missions (reusing `mission_archive_impl` per linked mission). Wire `runner session archive --with-missions`, update CLI help/golden tests, and run `make guide-cli`.
2. **Desktop & remote UI confirmation (`runner-app`).** Add a shared helper that resolves unarchived linked missions (in sidebar group order, with running status) for one or more chat session IDs across local (`AppStore.missions`) and remote (`RemoteClient`) state. Route `request_close_chat_pane`, `archive_chat_sessions`, and `request_archive_all` through Option A's `ConfirmDialog` whenever linked missions exist, add `en.toml` / `zh-hans.toml` strings, and archive the linked missions together on confirm (updating sidebar archiving state and leaving any open mission view for those missions).
3. **Tests, docs, independent review, and live QA.** Add unit, GPUI, daemon, CLI golden, and localization tests; update English and Chinese guide/arch pages; pass independent code review and run the selected native macOS checks below.

Starting points: `crates/runner-daemon/src/ops/{session.rs,mission.rs}`, `crates/runner-core/src/protocol/api.rs`, `crates/runner-cli/src/{command/cli.rs,command/session.rs,client.rs}`, `crates/runner-app/src/surfaces/sidebar/archive.rs`, `crates/runner-app/src/surfaces/chat/close.rs`, `crates/runner-app/src/surfaces/panes/{dialogs.rs,mod.rs,pane.rs}`, `crates/runner-app/src/surfaces/front_page/client.rs`.

## Verification

Before running checks, list the changed behaviors and selected checks. This is a feature pass, not a full regression run.

### Automated checks

- Daemon tests (`crates/runner-daemon/src/ops/`): `session_archive_direct` without `with_missions` leaves linked missions unarchived (preserving #748 unit behavior for default CLI callers); with `with_missions`, stops and archives running and completed linked missions, skips already-archived linked missions, preserves `linked_session_id`, and leaves other chats' missions untouched.
- CLI & golden tests (`crates/runner-cli/`): `runner session archive` with and without `--with-missions` (help text, plain output, `--json`, and zero-linked-mission no-op) plus `make guide-cli` freshness check.
- App & remote tests (`crates/runner-app/`): confirmation gating and Option A copy/mission rows across 0, 1, and 2+ linked missions, running vs. settled states, split-tab single-pane close scoping, split-tab `Archive tab?` with shell terminals, `Cancel` leaving chat and missions untouched, confirm archiving both locally and via `RemoteClient`, and `en` / `zh-hans` catalog parity.
- Required workspace checks: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` (or `make test`), `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, macOS `--features updater` Clippy, and `make fmt` check.

### Selected live checks (macOS development app)

QA runs these in `com.wycstudios.runner-dev` through computer use after acquiring `script/qa-lock`, using inexpensive models, low effort, tiny prompts, and fresh folders under `/tmp`, archiving only sessions/missions created for the check.

| ID | Selected check and reason |
| --- | --- |
| ARCH-LINK-01 | Start a chat in `/tmp`, have it start a tiny mission (or link an ended test mission to it), then click **Archive** on the group in the sidebar. Verify Option A's confirmation dialog opens listing the linked mission, `Cancel` keeps the group intact, and confirming archives both the chat and the mission (both disappear from the sidebar and appear in `Settings → Archived`). Covers the previously silent sidebar path. |
| ARCH-LINK-02 | With a chat linked to a still-running mission, press `⌘W` on the chat tab. Verify the dialog marks the running mission and warns that archiving stops it, and confirming stops and archives both cleanly with no leftover live PTYs. Covers the running-process warning and teardown path. |
| ARCH-LINK-03 | In a 2-pane split tab holding Chat A (with a linked mission) and Chat B (with its own or no linked mission), close only Chat A's pane. Verify only Chat A and Chat A's linked mission are prompted and archived while Chat B stays open. Covers split-pane scoping. |
| ARCH-LINK-04 | Archive an unlinked chat from the sidebar menu (no shell panes), and run `runner-dev session archive --with-missions <chat>` on a test chat with a linked mission. Verify the unlinked sidebar archive still happens immediately with no dialog, and `--with-missions` archives both chat and mission with no interactive prompt. Covers the preserved fast path and the new CLI flag. |

Unchanged IME, authentication, role/crew editing, updater, and non-archive mission report delivery are outside scope. Because the change touches daemon session/mission archive ops and the client protocol, its PR takes `core-review`, `risk-medium`, and `needs-live-test` until `ARCH-LINK-01` through `ARCH-LINK-04` pass.
