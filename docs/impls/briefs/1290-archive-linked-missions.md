# 1290 — Archive linked missions with chat

## Starting point

- Issue: [#1290](https://github.com/yicheng47/runner/issues/1290)
- Branch: `feat/1290-archive-linked-missions`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1290-archive-linked-missions` (keep its own `target/`; never touch the root checkout on `main`)
- Base revision: `36d22b0d` (`origin/main`)

## Problem and outcome

Today under #748, archiving a direct chat leaves its linked missions unarchived, dissolves the sidebar group, and turns each mission's back-link into `linked to <chat> (archived)`. Choosing **Archive** on a chat tab/group in the sidebar or clicking **Archive** on a stopped chat card skips confirmation completely when the tab has no shell terminals.

Implement the signed-off **Option A** confirmation prompt across all desktop and remote UI archive entry points so that archiving any chat with unarchived linked missions prompts first (listing those missions in `ConfirmDialog.note()`) and archives them together on confirm, and add `runner session archive --with-missions` in the CLI to stop and archive linked missions non-interactively with no confirmation prompt.

Authoritative spec: `docs/features/1290-archive-linked-missions.md` (wins on behavioral detail). Signed-off Pencil canvas: `runner/desktop/specs/1290-archive-linked-missions.pen` at revision `0610154` in `YeelinksCo/yeelinks_design` (`~/repos/yeelinks/yeelinks_design`), frames `Zyp5L`, `RRpz0`, `B8grI8` (`HTXqA`), and `g26XDF` (`i6GC2j`, `nnozr`, `LsGiX`, `FPVfe`).

## Read first

- `docs/features/1290-archive-linked-missions.md` — full behavioral contract, copy states, CLI rules, and verification matrix.
- `crates/runner-app/src/ui/overlay.rs` (`ConfirmDialog`) — existing `.icon()`, `.note()`, and `.detail()` slots that Option A reuses without new dialog chrome.
- `crates/runner-app/src/surfaces/sidebar/archive.rs` (`archive_chat_sessions`, `archive_all_confirmation_body`, `archive_workspace_sessions`), `crates/runner-app/src/surfaces/chat/close.rs` (`request_close_chat_pane`, `request_archive_all`), and `crates/runner-app/src/surfaces/panes/{dialogs.rs,mod.rs,pane.rs}` (`render_terminal_close_confirm`, `archive_chat_confirm_body`, `StoppedChatCard`).
- `crates/runner-app/src/surfaces/front_page/client.rs` and `crates/runner-daemon/src/companion/actions.rs` — local and remote/companion archive execution.
- `crates/runner-daemon/src/ops/{session.rs,mission.rs}` (`session_archive_direct`, `mission_archive_impl`), `crates/runner-core/src/protocol/api.rs`, and `crates/runner-cli/src/{command/cli.rs,command/session.rs,client.rs}`.

## Deliverables and boundaries

1. **Backend & CLI (`runner-core`, `runner-daemon`, `runner-cli`):**
   - Keep default `session_archive_direct` / `runner session archive <session>` behavior chat-only so existing callers and #748 link tests behave identically when `with_missions` is false/omitted.
   - Add additive `with_missions` support (preserving wire compatibility for older protocol/companion peers) that stops and archives the direct chat plus every unarchived mission linked to it (`missions.linked_session_id = session_id AND archived_at IS NULL`) via `mission_archive_impl` teardown (closing mission drawer shells, killing live mission PTYs, recording `mission_stopped`, setting `archived_at`, tearing down bus/router). Preserve `missions.linked_session_id`.
   - Add `--with-missions` to `runner session archive`: non-interactive, no double confirm. Include `archived_mission_ids` in JSON output when `--with-missions` is used (while keeping default `runner session archive` output backwards-compatible), update `crates/runner-cli/tests/golden.rs`, and regenerate CLI docs with `make guide-cli`.
2. **Desktop & remote UI (`runner-app`):**
   - Resolve deduplicated unarchived linked missions (in sidebar group order, with running status) for targeted chat session(s) across both local (`AppStore.missions`) and remote (`RemoteClient`) state.
   - Gate `request_close_chat_pane`, `archive_chat_sessions` (including `StoppedChatCard`), and `request_archive_all` (including sidebar `ArchiveTab`) through Option A's `ConfirmDialog` whenever targeted chats have unarchived linked missions. Keep unlinked single-chat sidebar/stopped-card archives instant when `terminals == 0`.
   - In a split tab, closing a single chat pane scopes the prompt and archive strictly to that pane's chat and its own linked missions; archiving the whole tab combines all chats, linked missions, and closed shell terminals (`Archive tab?`).
   - Add matching `en.toml` and `zh-hans.toml` strings covering singular/plural missions, running chat/mission stop warnings, and split-tab shell combinations.
   - On confirm, archive the targeted chat(s) and their unarchived linked missions (both local and remote), updating sidebar archiving state and leaving any open mission view for an archived mission cleanly.
3. **Documentation & history:**
   - Update `docs/guide/guides/chats-and-windows.md`, `docs/guide/concepts/missions.md`, `docs/guide/zh-CN/guides/chats-and-windows.md`, `docs/guide/zh-CN/concepts/missions.md`, and `docs/arch/arch.md` §9.4 where chat-archive behavior on linked groups is described, plus `make guide-cli`.
   - Fold `docs/features/1290-archive-linked-missions.md`, `docs/features/README.md`, and this brief into the implementation commit(s) so the branch history lands cleanly on `main`.
   - Non-goals: no 3-button `Chat only` footer (Option C was rejected; `Unlink` covers keeping a mission), no changes to `Settings → Archived` independent restore, and no changes to unlinked chat/mission archiving.

## Validation and handoffs

- `coder` implements the change, adds daemon, CLI golden, app/remote GPUI, and i18n parity tests, runs `make guide-cli`, `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` (or `make test`), `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings`, and `make fmt`, then commits a local snapshot and hands the SHA to `reviewer`.
- `reviewer` critiques the unpushed snapshot against `docs/features/1290-archive-linked-missions.md` and confirms the live checks (`ARCH-LINK-01` through `ARCH-LINK-04`).
- `qa` acquires `script/qa-lock acquire --timeout 540`, builds and runs the reviewed candidate in `com.wycstudios.runner-dev`, executes `ARCH-LINK-01` through `ARCH-LINK-04` (defined in `docs/features/1290-archive-linked-missions.md` under Verification) with `/tmp` folders and tiny low-effort prompts, cleans up only its own test chats/missions, records results and screenshot paths in `docs/tests/runs/2026-11-13-mac.md`, and releases `script/qa-lock`.
- Once both `reviewer` and `qa` are clean on the current head, `coder` pushes `feat/1290-archive-linked-missions`, opens the PR against `main` with `risk-medium` (keep automatic `core-review`; omit `needs-live-test` once all four selected checks pass, and `reviewer` applies `reviewed` on the current head), and drives CI green.

## Authorization

Jason approved the `agy trio` crew (3 agents: `coder`, `reviewer`, `qa`) on 2026-11-13 and authorized committing on `feat/1290-archive-linked-missions`, pushing the branch, and opening a pull request against `main` once code review and live QA are clean. Do not merge.
