# #1489 — Kitty graphics, pixel geometry and SGR-Pixels mouse, Phase 1

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-26. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1489-kitty-graphics-protocol` on the branch `feat/1489-kitty-graphics-protocol` (rebased onto `origin/main` `6ae678d7`, its own `target/`). The branch's draft PR is [#1495](https://github.com/yicheng47/runner/pull/1495), which holds the spec. Never touch the root checkout or sibling worktrees, including `fix/818-terminal-round-trips`.

## Problem and outcome

`vte` drops every APC, so Kitty graphics commands never reach the terminal; the PTY's pixel size is zero, `CSI 14 t` answers zero cell sizes and `CSI 16 t` gets no reply; mode 1016 is unknown; and motion without a button is never reported, even under mode 1003. Outcome: Phase 1 of the spec. Programs such as `kitten icat` and `terminal-browser` show images in local terminal panes, the images ride in attach snapshots, programs see real pixel geometry and SGR-Pixels mouse reports, and companion clients receive no graphics but do receive the session's cell size.

The spec is authoritative and was reviewed clean against the code and Ghostty v1.4.0: [`docs/features/1489-kitty-graphics-protocol.md`](../../features/1489-kitty-graphics-protocol.md). There is no Pencil design: the change adds no UI chrome. Code references in the spec are to `main` at `9d9c50b6`; nothing they cite has changed since.

## Read first

- The spec, in full: Decisions, Design, Implementation Phases and Verification.
- `vendor/alacritty_terminal/RUNNER_PATCH.md`: the vendored crate carries read-only accessors only. Anything the spec needs beyond that lives outside it, as mode 2031 does.
- `runner-terminal`: `terminal/model.rs` (`TerminalModel::feed_output`, `flush_sync_update`, `sync_session_title`, the event worker's `TextAreaSizeRequest`), `terminal/find.rs` (`ScrollTracker` and its fixture test), `terminal.rs` (`TerminalMirror::restore`, `feed_output`, the 2031 handling), `snapshot.rs` (`serialize_with_history`, `BoundaryScanner`), `mappings.rs` (the mouse encoders).
- `runner-daemon`: `session/manager/output.rs` (`start_forwarder_thread`, `ingest_output_for_launch`, `resize_controlled`), `session/pty_runtime.rs`, `session/manager/terminal.rs` (`FrameQueue`, `push_frame`, `push_controller`, `take_back_terminal`), `session/manager/attach.rs` (`install_handle_state`), `companion/mod.rs`, `remote/client/terminal.rs`.
- `runner-core`: `protocol/{wire,socket,api}.rs`, `remote/client.rs`, `companion/messages.rs` and its golden fixtures.
- `runner-app`: `terminal/element.rs` (`DragKind`, `mouse_move`, the capture-phase move listener, prepaint and paint), `terminal/resize.rs`, `surfaces/files/image.rs` (`release_render_images`).
- `vte` 0.15.0 (`lib.rs` APC state, `ansi.rs` synchronized updates and window ops) in `~/.cargo/registry/src/`.
- Ghostty v1.4.0 `src/terminal/kitty/*.zig`, `src/terminal/Terminal.zig` and `src/input/mouse_encode.zig`, fetched with `gh api 'repos/ghostty-org/ghostty/contents/<path>?ref=v1.4.0' --jq .content | base64 -d`. Read them for behavior and test vectors (MIT); write Runner's code separately.

## Deliverables

Follow the spec's Implementation Phases as commits that each build and pass their tests, and hand each to `@reviewer` as it is ready rather than at the end:

1. **1a, `runner-terminal`:** the `kitty` module, the stream handler that replaces `ScrollTracker` at every site that feeds `Term`, mode 1016 with DECRQM, `CSI 14 t` and `CSI 16 t`, the encoders' pixel and motion paths, and snapshot graphics.
2. **1b, `runner-core` and `runner-daemon`:** the cell size on every size path and in the companion messages, PTY pixel sizes, local media on the forwarders, parsing before publishing for bursts with graphics, frame splitting and the queue's byte limit, the daemon-wide count, and the companion filter.
3. **1c, `runner-app`:** pushing the cell size, the texture cache and its releases, the paint bands, pixel mouse reports and motion without a button, then the 30 fps profile the spec describes.
4. **Docs:** the spec's Guide pages in both locales, and the tech and arch pages it lists.

Not in scope, as the spec lists: Phase 2 (Unicode placeholders, animation, companion graphics), #1492's Launcher work, Sixel, iTerm2 images and tmux passthrough. The spec describes only what companion clients send and receive; keep any client's internals out of this repo. If the design turns out not to work as written, stop and report to the operator with the evidence instead of improvising a different design.

## Validation and handoffs

- **@coder:** the spec's Automated section, ending with `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check` and `python3 script/check-guide-translations.py`. Update the companion golden fixtures through their existing workflow and inspect the change. Any new test that waits on daemon threads, a watcher or a timer runs `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'`, and the handoff states the count. Run heavy suites one at a time with other worktrees.
- **@reviewer:** reviews each handed-off commit against the spec and Ghostty v1.4.0. Look hardest at: output without a graphics command keeping today's order and cost; the stream handler leaving `Term` exactly as a plain parse would; synchronized-update flush and reopen keeping the original deadline; frame sizes against `wire::MAX_FRAME` and the queue limits; local media safety and reading before the state lock; the daemon-wide count never crossing sessions or deadlocking; the snapshot budget; texture release; companion stripping and protocol compatibility in both directions; and the vendored crate staying accessor-only. The clean review names the live checks and the candidate commit, adds `reviewed` and names that commit in a PR comment.
- **@qa:** the spec's Live checks on the macOS development app, holding `script/qa-lock`, with the remote checks on the same-Mac two-host relay fixture. Write the emitter and receiver programs in an owned folder under `/tmp`; do not install software to get `kitten icat`, `chafa` or `terminal-browser`; use them only where they are already installed. Record pass or fail per check with screenshots in `docs/tests/runs/1489-kitty-graphics-protocol-mac.md`. A check whose prerequisite is missing, such as a second display at another scale, is Blocked, not passed. Linux and Windows ConPTY are not required for this pull request: the PR claims macOS only and lists them as not run. Failures go back to `@coder`, and the fixed candidate is reviewed again before qa reruns the affected checks.

## Authorization

Approved crew: the neon trio. The crew may commit, push `feat/1489-kitty-graphics-protocol` and use the existing PR #1495: mark it ready for review once review is clean, retitle it `feat(terminal): Kitty graphics, pixel geometry and SGR-Pixels mouse (#1489 phase 1)`, keep `Refs #1489` because Phase 2 remains, and label it `core-review`, `risk-high` and `needs-live-test`. Fold this brief and the spec into the implementation commits, dropping the co-author trailer the spec commit carries. Remove `needs-live-test` once every selected macOS check has passed, and drive CI green on all platforms. Do not merge: Jason reviews `core-review` pull requests.
