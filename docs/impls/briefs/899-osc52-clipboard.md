# #899 — OSC 52 clipboard writes from terminal programs

Jason asked for a claude trio mission on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-899-osc52-clipboard` on `feat/899-osc52-clipboard`, based on `origin/main` at `a06bd8a8`. Issue: [#899](https://github.com/yicheng47/runner/issues/899), P2.

## Problem

A yank in Neovim, Helix or tmux inside a Runner pane copies nothing. These programs set the clipboard with OSC 52; the vendored `alacritty_terminal` decodes it and emits `Event::ClipboardStore`, and the terminal model's event worker in `crates/runner-terminal/src/terminal/model.rs` drops it in its `_ => {}` arm.

## Spec

[`docs/features/899-osc52-clipboard.md`](../../features/899-osc52-clipboard.md) is the contract: the daemon forwards the write as a `session/clipboard` event, the app writes the clipboard once when one of its windows shows the session in an active tab and drops it otherwise, no UI feedback, reads stay refused, 1 MiB cap. Where this brief and the spec disagree, the spec wins.

## Read first

- `AGENTS.md`, including Test Scope and Crew Missions.
- The spec.
- `crates/runner-terminal/src/terminal/model.rs`: `TerminalHost` and the `native-term-events-*` worker.
- `vendor/alacritty_terminal/src/term/mod.rs`: `clipboard_store`, `clipboard_load`, `Osc52`.
- `crates/runner-daemon/src/session/manager/terminal.rs` (`Host`) and `session/manager/events.rs` (`SessionEvents`, `CoreSessionEvents`): follow how `session/input-error` is emitted.
- `crates/runner-daemon/src/remote/client/route.rs`: how `session/input-error` is routed to a remote viewer.
- `crates/runner-app/src/surfaces/root/init.rs` and the chat and mission-workspace event handlers: how a session event reaches windows.

## Deliverables and boundaries

- The implementation, unit tests and guide paragraph the spec lists.
- The copied text never reaches a log, the event log or the database.
- Non-goals: OSC 52 reads, a Settings toggle, Runner's own copy and paste.

## Validation and handoffs

Run the affected crates' tests with `--locked`, workspace Clippy with warnings denied, `cargo fmt --all --check` and `git diff --check`. @coder implements and hands each snapshot to @reviewer. Live checks are required: the spec's Live checks section names them; @qa runs them on macOS in the development app after review is clean and records the results in `docs/tests/899-osc52-clipboard.md`.

## Authorization

Jason approved this claude trio mission on 2026-11-05: commit, push this branch and open one PR against `main`, labelled per AGENTS.md. Stop at the open PR with green CI and the QA verdict, and report through Runner with the PR URL, final SHA, changes and validation.
