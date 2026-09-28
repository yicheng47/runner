# 724 — Fix wheel coordinates for fullscreen transcripts

Fix [P2 #724](https://github.com/yicheng47/runner/issues/724) according to `docs/features/724-terminal-wheel-coordinates.md`. Jason requested this spec and implementation mission on 2026-09-28. Use the codex pair's coder/reviewer loop and finish with an open PR and green CI.

## Workspace and source of truth

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-724-terminal-wheel-coordinates`, on the existing branch `fix/724-terminal-wheel-coordinates`. The branch begins with this brief, the spec and its index entry on top of `origin/main` `b9a0d6b`. Read `AGENTS.md` and the full spec before editing. Do not create another checkout or branch, edit the root checkout or other worktrees, or share a target directory. If main moves, rebase onto `origin/main`; never merge main into this branch.

## Problem and implementation

Codex 0.157.1's fullscreen transcript ignores wheel events outside its content area. When its pinned prompt header is visible, that area starts below the first row. Runner's `encode_scroll` always sends cell `(1,1)`, so a wheel anywhere falsely targets the header. Ordinary scrollback works because it uses the local buffer without mouse reporting. macOS OS-window fullscreen is separate. The matching Codex source and limitations of the investigation are in the spec; do not claim a live reproduction from those findings.

Forward the actual event position as terminal viewport coordinates for SGR and legacy mouse reports. Reuse the displayed element's existing geometry and hit-testing, including pane origin, padding and cell metrics. Preserve local scrollback, Shift bypass, alternate-screen/application-cursor arrows, fractional accumulation and input-ownership rules. Keep the child TUI responsible for deciding whether an actual header event should scroll. Do not change Codex launch args, disable fullscreen or add a runtime-specific workaround.

Trace all callers: `surfaces/chat.rs`, `surfaces/panes.rs`, `surfaces/mission_workspace/{input,view,terminal_pane}.rs`, `surfaces/agent_update.rs`, `terminal/element.rs`, and `runner-terminal/src/{terminal,mappings}.rs`. If wheel ownership moves into the shared terminal element, remove duplicate surface handlers and preserve stopped/read-only scrolling and local selection autoscroll. Use existing legacy coordinate limits without overflow or accidental local-scroll fallback. Keep the change small and avoid unrelated input refactors.

## Review and validation

The coder implements and validates, then explicitly asks the reviewer through Runner to review the whole working-tree diff against the spec before committing code or opening a PR. The reviewer stays idle until that request, then returns must-fix findings with file:line references. Iterate until it reports `NO REMAINING MUST-FIX ISSUES`. Do not spawn extra agents, crews or subagents.

Cover non-origin SGR and legacy reports, pointer-to-cell conversion with offset panes and changed metrics, one delivery per gesture, actual header coordinates, protocol bounds, and all preserved scroll modes. Tests must catch the former fixed-coordinate behavior and exercise the UI-to-terminal boundary where practical. Check every wheel surface and local-only scroll caller rather than fixing only direct chats.

Run and report the exit codes of:

- `cargo test --locked -p runner-terminal -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Keep the actual check exit status; do not hide failures behind a pipe to `tail` or `grep`. CI must pass on macOS and Windows. Do not start, stop or interact with Jason's live Runner apps, sessions or database, or launch live agent sessions as test fixtures. Use isolated synthetic fixtures. Jason performs the spec's UI smoke checks after handoff; report unperformed UI and native Windows checks explicitly.

## Authorization and handoff

After clean working-tree review, squash the spec, index, brief and implementation into **one commit** on top of current `origin/main`, using an imperative subject naming the fix, such as `fix(terminal): report wheel events at the pointer cell`. Do not use destructive reset or discard unrelated changes. No co-author trailers or agent session links.

Push `fix/724-terminal-wheel-coordinates` and open a PR against `main` with `Closes #724`, the root cause, resulting behavior, review verdict, test evidence and remaining manual checks. Watch `gh pr checks <n> --watch` until CI is green on both platforms. Fold fixes into that single commit with `git commit --amend` and `git push --force-with-lease`; send non-trivial fixes back through review. Explain external CI blockers with evidence.

Do not merge, delete the branch or worktree, or cut a nightly or release. Final handoff through Runner to everyone: PR URL, macOS/Windows CI status, review verdict, changed files, checks with exit codes and Jason's remaining smoke steps. Then stand by.
