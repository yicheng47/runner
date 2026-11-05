# 1118 — Host button in the sidebar footer

Implement [P2 #1118](https://github.com/yicheng47/runner/issues/1118). Jason asked on 2026-11-05 for a claude trio mission (coder, reviewer, qa) that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1118-host-button`, on branch `feat/1118-host-button`, cut from `origin/main` at `2ae1474b`. Its tip is the spec and this brief.

## Read first

- **`docs/features/1118-host-button.md`.** It wins over the issue and this brief on behaviour; the design wins on visual detail. It also settles the issue's three open questions (remote footer, connection trouble, keyboard).
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/`: `1-this-computer-popup-open.png`, `2-after-choosing-mac-studio.png`, `3-back-to-this-computer.png`, `m1-popup.png`, the full boards `option-a-board.png` and `m1-board.png`, and today's screens `today-local.png` and `today-remote.png`. Match layout, spacing and colours with the existing theme tokens; sample names and usage figures are illustrative. Do not delete them, and do not open or edit `.pen` files.
- `crates/runner-app/src/surfaces/remote_client/picker.rs`: `render_computer_trigger`, `toggle_computer_picker`, `render_computer_picker`, `computer_detail`, `attention_dot`, and the anchor stored in `ui.anchor`.
- `crates/runner-app/src/surfaces/app_shell/sidebar.rs`: the brand-area trigger placement and the footer row (usage, `render_remote_connection`, settings).
- `crates/runner-app/src/surfaces/app_shell/settings_button.rs`: the gear, hidden in remote windows today; the host button should share its size and hover treatment.
- `crates/runner-app/src/surfaces/remote_client/workspace/render.rs`: `render_remote_connection` and its `Connection` → label/glyph/colour mapping, which moves to the popup's current row and the host button's colour.
- `crates/runner-app/i18n/en.toml` and `zh-hans.toml`.

## Deliverable

1. Everything in the spec's Scope: the footer host button, the popup anchored above it, the M1 marking of this computer, the current computer's check and connection subtitle, the same footer in remote windows, connection-state colours on the button, and the keyboard behaviour.
2. Tests in `runner-app` covering the spec's Verification list. Update assertions the change invalidates (for example tests that look up `COMPUTER_PICKER_TRIGGER` or `REMOTE_CONNECTION_ROW`). Gate any helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.
3. Docs in the same diff: the guide and regression cases the spec lists, and English and Chinese strings. If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope: runnerd, CLI, protocol and `runner-core` changes; pairing flows and row actions beyond restyling; `.pen` files; README screenshots.

## Validation

The coder runs each of these and reports its exit code, logging each to a file and reading `$?` from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked -p runner-app --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Live checks are required: the change alters what a person sees and operates in the sidebar. qa runs them on the reviewed candidate in the development app with `make run` from this worktree, under the standing development-app authorization in `AGENTS.md`, with screenshots for each:

- **Signed out (qa runs this).** The dev app is signed out today. The footer shows usage and the gear with no host button, and nothing appears under the brand; check dark and light, Glass and Solid. Reason: the button must not appear without an account, and the footer layout changed.
- **Signed in (needs the hosted account and a second paired computer).** The host button between usage and the gear, its dot, the popup above the footer (this computer first, green tile, divider, "This computer", check on the current computer), switching to another computer and back, the remote footer (usage, green button, gear, no connection line), Reconnecting/Unreachable colouring with the current row's subtitle, keyboard open/Tab/Escape and focus return, in English and Chinese. Reason: these are the feature's own surfaces; REMOTE-PICK-01/02/03 and REMOTE-WIN-01 cover the same ground. Crew slots never sign in to or contact the hosted service themselves. qa runs these only if Jason has signed the dev app in and posts authorization on the mission feed; otherwise qa records them as not run, for Jason.

Native Windows and Linux are not available to the crew; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for the coder's handoff through Runner, then reviews the whole diff against the spec, the PNGs and this brief, must-fix findings first with file:line pointers, checking in particular: no trigger remains under the brand; the dot's rule is unchanged from today's trigger; the host button never renders signed out; the remote footer keeps usage and the gear and has no connection line; every `Connection` state has a label in the popup and a colour on the button; this computer is first and marked only by tile, position and subtitle. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES` and names the live checks. Then the coder hands the reviewed commit to qa, which changes no code; a failure goes back to the coder, and the fix is reviewed again before qa reruns the affected checks. No extra agents, crews or subagents.

The crew may commit, push and open the PR against `main` once the review is clean; it does not wait for the signed-in live checks to open the PR. Label it `frontend-only`, `risk-medium`, and `needs-live-test` while any selected check has not passed; the reviewer adds `reviewed` with its clean review. The PR body carries `Closes #1118`, a summary, test evidence with exit codes, qa's per-check results, the signed-in checks left for Jason if qa could not run them, and what is unverified on Windows and Linux.

The final handoff goes to everyone through Runner: the PR URL and CI result on both platforms, changed files, checks with exit codes, qa's verdict, any spec deviation, and what is unverified.
