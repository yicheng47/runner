# 575 — Splits and new terminals inherit the shell's live cwd

Implement [P3 #575](https://github.com/yicheng47/runner/issues/575), milestone 0.14. Jason asked for a claude pair crew mission on 2026-09-24 that ends in an open PR, not a merge; he reviews it later.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-575-live-cwd`, on branch `feat/575-live-cwd`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `19c1238`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Other missions' worktrees exist; ignore them. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- The issue body: scope and non-goals are binding. Its dependency, #574, has shipped: `docs/features/archive/574-terminal-tab-split.md`. Also read `docs/features/archive/469-terminal-drawer.md`, the other place terminals spawn.
- `crates/runner-terminal/src/terminal.rs`: the OSC stream handling, `file_target_from_uri` (it already parses `file://` URIs for links), and `link_cwd` / `resolve_link_cwd`, which resolves relative file links against the spawn cwd once per session.
- `crates/runner-app/src/pane_layout.rs` and `crates/runner-app/src/surfaces/panes.rs`: Split Right, Split Down and New terminal on a terminal tab.
- `crates/runner-backend/src/session/launch.rs`, `session/runtime.rs` and `shell_path.rs`: how Runner spawns shell terminals and builds their environment.

## Step 1: a short spec, reviewed before any code

Write `docs/features/575-live-cwd.md`: motivation, the behavior, decisions, open items, and verification. Settle these with evidence:

- **Parsing.** OSC 7 terminated by both BEL and ST, `file://host/path` with percent-decoding, reports whose host is not this machine, and non-directory paths. The live cwd is kept per session beside the spawn cwd, which stays the fallback.
- **Where it applies.** Split Right, Split Down and New terminal on a terminal tab, as the issue says. Decide whether the terminal drawer's new terminal follows it too, and whether relative file links should resolve against the live cwd instead of `link_cwd`'s spawn cwd. Pick the answer that keeps Runner consistent, and say why.
- **Shell integration.** The issue leans toward injecting it, as Ghostty does. Decide how for zsh (for example a `ZDOTDIR` wrapper that sources the user's real startup files first) and bash (for example `--rcfile` or `PROMPT_COMMAND`). The user's own configuration must still load unchanged, and a shell that already emits OSC 7 must not end up with duplicated hooks. Never edit the user's rc files. Other shells get no injection and fall back to the spawn cwd. On Windows, decide between a PowerShell prompt hook and a documented follow-up. Agent sessions are never touched; this is terminal tabs only.
- **Out of v1:** showing the live cwd in the UI (the issue marks it optional, and it would need a design first), and following cwd for chat sessions.

Probe with throwaway shells only. Run `zsh` and `bash` with a temporary `HOME` and `ZDOTDIR`, and never read or change Jason's real rc files beyond checking that they exist. Hand the spec to the reviewer through Runner and wait for `SPEC OK` or must-fix findings before writing code.

## Step 2: implementation

Build everything the reviewed spec puts in v1, with tests: the parser (both terminators, encoding, a foreign host, garbage), the per-session live cwd and its fallback, the spawn cwd chosen for each split and new-terminal path, and the injected integration. For the integration, add a test that spawns a real zsh and bash in a temp home with the injection, runs `cd` in them, and asserts that the OSC 7 arrives and that the user's rc still loaded. Update `docs/arch/arch.md` (and `docs/arch/windows.md` if the Windows answer changes anything there), plus `README.md` and `README.zh-CN.md` together if the behavior deserves a line. Write a short smoke checklist at `docs/tests/575-live-cwd-smoke.md` for Jason.

Out of scope: UI for the live cwd, chat sessions, editing rc files, and `.pen` files.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-terminal --profile ci --no-fail-fast`
- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions: Jason smoke-tests the UI. Native Windows is unavailable, so say what is unverified there.

## Crew handoff and authorization

The coder owns the spec, implementation, tests and fixes. The reviewer waits for an explicit Runner handoff: first the spec, then the implementation. It reviews the whole working-tree diff against the issue, the spec and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that the injection loads the user's own zsh and bash configuration unchanged, including `ZDOTDIR` users, and never double-hooks;
- that agent sessions' spawn environment and argv are unchanged;
- that a missing, foreign or stale live cwd falls back to the spawn cwd, and a directory that has since been deleted does not break the split;
- that no OSC 7 parsing runs in a way that slows the output path.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES` on the implementation. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: the brief commit first, then the spec, the code and the docs. Each gets an imperative subject and scope `terminal`, `session`, `ui` or `docs`, and no co-author trailers.
- **Push** with `git push -u origin feat/575-live-cwd`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #575`, a summary, the spec's decisions, test evidence, what waits for Jason's smoke, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, spec decisions and open items, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
