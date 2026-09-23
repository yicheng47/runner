# 592 — OpenCode runtime

Implement [P2 #592](https://github.com/yicheng47/runner/issues/592), milestone 0.12. Jason asked for a claude pair crew mission on 2026-09-23 that ends in an open PR, not a merge; he reviews it the next day.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-592-opencode-runtime`, on branch `feat/592-opencode-runtime`. The mission's directory is this worktree. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory.

**This branch is stacked on #644.** It starts from `feat/644-antigravity-runtime` at `5fec710` (open PR #714, unmerged), because Antigravity and OpenCode touch the same exhaustive matches, catalog lists, README columns and arch enumerations. Append OpenCode after Antigravity everywhere. Do not edit Antigravity's code except where a shared site needs a new arm. If `origin/feat/644-antigravity-runtime` moves, rebase onto it; if #714 merges, rebase onto `origin/main`. Never merge either branch into this one.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- The issue body: a probe of OpenCode 1.18.30 from 2026-09-14 and the v1 scope. OpenCode 1.18.30 is installed at `~/.opencode/bin/opencode`; npm has `opencode-ai` 1.18.32.
- `docs/features/644-antigravity-runtime.md`, and the Antigravity diff on this branch (`git log origin/main..5fec710`): the newest runtime, and the template for both the spec and the code.
- `docs/features/archive/540-copilot-cli-runtime.md`, "What a new runtime touches": the site inventory.
- `docs/features/533-agent-cli-updates.md`: `update_args` and `npm_package`. `docs/features/archive/60-fork-chat-to-pane-or-tab.md` and `527-mission-permission-mode.md`.
- `crates/runner-backend/src/session/codex_capture.rs`: post-spawn key capture, the closest shape to reading OpenCode's SQLite session table.

## Step 1: the spec, reviewed before any code

Write `docs/features/592-opencode-runtime.md` in the 644 spec's shape: motivation, probe evidence, what a new runtime touches with OpenCode's values per site, scope, decisions, open items, phases and verification. Settle each point below with evidence, not guesses:

- **Key capture.** When the `session` row appears (at spawn or at the first message), and how to pick this spawn's row (cwd, `time_created`, `parent_id`) without racing another OpenCode session in the same folder. Also the conversation-exists `SELECT`, and `OPENCODE_DB`.
- **First turn and persona.** `--prompt <body>`, with the persona folded into it.
- **Permissions.** Default → nothing, Bypass → `--auto`, and whether AcceptEdits through `OPENCODE_CONFIG_CONTENT` really merges. If it does not, AcceptEdits stays hidden.
- **Model.** `--model provider/model`, with no effort flag. Say whether the default comes from `opencode.json` `model`, and whether its value is an id Runner can pass back.
- **Fork** with `--session <id> --fork`, and **resume** with `--session <id>`.
- **Settings → MCP:** OpenCode as a catalog client, writing `opencode.json` `mcp` in OpenCode's entry shape while preserving JSONC. Since #648 there is no default registration.
- **Skills roots,** and whether any are shared with Claude Code and `.agents`.
- **#533.** `opencode --version`, with `upgrade` as the update argument and `opencode-ai` as the npm package.
- **Terminal behavior** (alternate screen, mouse reporting, title) for the fixture and the wheel.

How to probe. The `--help` and `--version` of every subcommand are fine. You may read the SQLite *schema* of `~/.local/share/opencode/opencode.db` with `sqlite3 -readonly … .schema`, but never its rows. Reading OpenCode's source on GitHub is fine. **Any run of the OpenCode TUI or `opencode run` happens only in a throwaway environment:** `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `OPENCODE_DB` and `OPENCODE_CONFIG_DIR` all point into a scratch directory. It then has no sign-in, so no model is ever called. Never run OpenCode against Jason's real config, data or auth, and never run `opencode upgrade` or `opencode auth`.

Hand the spec to the reviewer through Runner and wait for its verdict before writing code. The reviewer checks it against the probe evidence and against the 644 and 540 inventories, then posts `SPEC OK` or must-fix findings.

## Step 2: implementation

Everything the reviewed spec puts in v1, except:

- **The mark is a placeholder.** Use a neutral glyph already in the asset set, as #644 did, and keep the icon tests exhaustive. Do not draw or source a logo. Do not open or edit `.pen` files.
- **No hook status.** OpenCode's plugin events are a follow-up. OpenCode sessions use baseline status.
- **Windows:** `default_enabled` stays off there until its smoke.

Also write tests for every verification item that does not need a signed-in OpenCode, and a smoke checklist at `docs/tests/592-opencode-smoke.md` for Jason. It covers the fixture `crates/runner-terminal/fixtures/opencode-first-turn.ndjson`, the wheel check, live resume and fork, the permission modes, and JASONPC. Update `README.md` and `README.zh-CN.md` together, plus `docs/arch/arch.md` and `docs/arch/windows.md`.

Out of scope: ACP, `serve`/`attach`, plugin management, a provider/model picker, installing or authenticating OpenCode, `.pen` files, and README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-terminal --profile ci --no-fail-fast`
- `cargo test --locked -p runner-cli --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions: Jason smoke-tests the UI. Do not edit `~/.config/opencode`, `~/.local/share/opencode`, or any other agent's configuration, except in tests' temp dirs and the throwaway probe environment. Native Windows is unavailable, so say what is unverified there.

## Crew handoff and authorization

The coder owns the spec, implementation, tests and fixes. The reviewer waits for an explicit Runner handoff: first the spec, then the implementation. It reviews the whole working-tree diff against the spec and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that every `(runtime, mode)` arm and enumerating test covers `OpenCode`, and that the other runtimes' argv are unchanged, Antigravity's included;
- that key capture picks this spawn's session and cannot adopt another session in the same cwd;
- that the MCP write preserves JSONC and every other key;
- that no code path or probe touched Jason's real OpenCode data.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES` on the implementation. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: the brief commit first, then the spec, the code and the docs. Each gets an imperative subject and scope `runtime`, `session`, `ui`, `mcp` or `docs`, and no co-author trailers.
- **Push** with `git push -u origin feat/592-opencode-runtime`.
- **Open the PR** with `gh pr create --base main`. CI runs only on pull requests into `main`. Until #714 merges, the diff also shows #714's commits; they keep their SHAs through Jason's merge commit and drop out of this PR's diff when #714 lands. The body opens with "Stacked on #714: review from `<first OpenCode commit>` on; #714's commits drop out when it merges." It then carries `Closes #592`, a summary, the key changes, test evidence, the placeholder mark, what waits for Jason's smoke, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, touch PR #714 or its branch, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, spec decisions and open items, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
