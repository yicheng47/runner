# 777 PR 2 — Runtime adapter: spawn hooks and status watchers

Implement PR 2 of 3 for [P1 #777](https://github.com/yicheng47/runner/issues/777): phase 2 of the spec. PR 1 ([#779](https://github.com/yicheng47/runner/pull/779)) moved identity and argv into `runtimes/`. This PR moves the side effects of a spawn, which still branch on the runtime. Jason asked on 2026-10-01 for a crew mission that ends in an open PR, not a merge. This is a refactor: behavior stays byte-identical.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-777-runtime-adapter-spawn`, on branch `refactor/777-runtime-adapter-spawn`. The mission's directory is this worktree. Its tip is this brief and a spec edit, on top of main `d8e5919e`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/777-runtime-adapter.md`**: Proposal, Rules, Decisions, and phase 2. The branch's spec edit settles `key_capture` as a mechanism enum. The spec wins over the issue and over this brief on any detail.
- `runtimes/mod.rs`: the trait as PR 1 left it, `adapter`, `for_key` and `NoAgent`. `runtimes/*/mod.rs`: where the hook-arg builders already live, still gated through `hook_feed::hooks_supported` and `claude_status::hooks_supported`.
- `session/manager/spawn.rs`:
  - `agent_env` :41, `enter_claude_launch_gate` :420, `seed_runtime_project_trust` :439;
  - in `apply_runtime_args` :627: `codex_pending_turn`, the rekey drop clear, and the five status env blocks;
  - `codex_capture_prompt_marker` :777, `start_antigravity_capture` :800;
  - the Codex and TRAE capture contexts at :1188, :1753 and :2895;
  - the agent lists at :1844 and in `queue_windows_batch_first_turn` :1877;
  - Pi's fork prompt at :2101.
- `session/pty_runtime.rs`: `HookStatusWatcher` :97 and the start chain from :243.
- `session/manager/mod.rs` :829–856: startup cleanup, the hook installs, and the rekey watcher.
- `session/hook_feed.rs:17` `hooks_supported`, `session/codex_capture.rs:67` `sessions_root_for`.
- The goldens PR 1 added: `session/manager/tests/golden.rs` and `expectations/`.

## Deliverable

### Step 1: more goldens, before any code moves

1. **Extend the goldens to cover this PR's surfaces,** recorded on untouched `d8e5919e`. Add new expectation files and never edit PR 1's. At minimum:
   - the values, not just the keys, of every env var Runner sets, normalized;
   - `SpawnSpec.codex_pending_turn`;
   - which status watcher would start and which key capture runs, for each runtime and launch shape, including role args that turn hook injection off (Claude with its own `--settings`, Codex when `inject_codex_hooks` declines);
   - `hooks_supported` for every runtime with `windows` true and false (it takes the flag, so Windows is recordable here);
   - the files the startup installs write to app data;
   - what trust seeding writes into a temp home for each runtime;
   - whether the Claude launch gate applies.
2. **Coverage map.** Every per-runtime decision this PR moves must be covered by a golden or an existing test before it moves. Put a table in the handoff mapping each moved site to the test that covers it.
3. **Commit this step alone, locally,** green on untouched `d8e5919e`. From then on, every expectation file stays byte-identical. If one has to change, stop and ask through Runner.

### Step 2: the move (phase 2)

4. **New trait members:**
   - `launch_env`: Claude's and pi's env from `agent_env`.
   - `launch_gate`: Claude's 1.5 s spacing, so the gate runs whenever an adapter returns one.
   - `seed_trust`: Codex, Copilot (honouring `COPILOT_HOME`) and Antigravity. TRAE stays unseeded, as today.
   - `key_capture`: returns a `KeyCapture` enum: `None`, `RolloutScan { sessions_root }` (Codex, TRAE, with the prompt marker), `LogTail` (Antigravity) or `RekeyDrop` (Claude, pi). The manager runs the mechanism and never checks the runtime. `sessions_root_for` goes away.
   - `status_hooks`: `Option<&'static dyn StatusHooks>`, with `supported(windows)`, the startup install, the per-spawn env, and `start_watcher`, which returns a `Box<dyn HookWatcher>`.
5. **The adapters decide which hook arguments to emit.** Their builders call their own `status_hooks().supported(..)`. `hook_feed::hooks_supported` and `claude_status::hooks_supported` go away.
6. **`HookStatusWatcher` becomes `Box<dyn HookWatcher>`.** `interrupt_signal` and `drain_observations` become trait methods, and the Codex session-start callback keeps working. `SpawnSpec` gains `agent_runtime: Option<Runtime>`. `PtyRuntime` asks that runtime's adapter for a watcher instead of probing five env-var pairs in order. The watcher still starts only when its env pair is in `spec.env`, as today: the runtime alone is not enough, because `apply_runtime_args` skips the env when role args turn the hooks off, and a watcher started then would wait for events that never come. The fallback is unchanged: when a watcher fails to start, it logs and keeps the baseline.
7. **Codex's pending-turn tracking** becomes a capability of its status hooks. The field keeps its name, `codex_pending_turn`.
8. **Agent checks.** The agent lists at :1844 and :1877 become "the key parses to a runtime that is not Shell", which matches today for every key, unknown ones included. Pi's fork prompt at :2101 reads `prompt_channels()`.
9. **Startup.** `manager/mod.rs` loops over the adapters for installs and cleanup. The rekey watcher still starts once.
10. **File moves,** with `git mv` so history follows. Item names do not change; only module paths do.
    - `agy_capture.rs`, `agy_status.rs` and `agy_trust.rs` go into `runtimes/antigravity/`.
    - `claude_status.rs` goes into `runtimes/claude_code/`.
    - `codex_status.rs` and `codex_trust.rs` go into `runtimes/codex/`.
    - `copilot_status.rs` and `copilot_trust.rs` go into `runtimes/copilot/`.
    - `pi_status.rs` goes into `runtimes/pi/`.
    - The shared mechanisms stay in `session/`: `codex_capture.rs`, `claude_rekey.rs`, `hook_feed.rs` and `status.rs`.
    - The 110 tests in the moved files move with them, with their expected values untouched.
11. **`cfg(unix)` gating.** Gate any import or helper used only by `cfg(unix)` tests with `cfg(unix)`, because Windows clippy fails on unused ones. Watch the `cfg(windows)` paths in the moved files: a lost `cfg` compiles on macOS and breaks the Windows job.
12. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope:
- **PR 3:** Codex Speed (:871, :990, :1575 and `ops/`), native defaults, model discovery, versions, usage, skills, MCP, and the app's tables.
- **Not at all:** renamed items or fields, and any behavior change, including bugs found on the way. List those bugs in the handoff.

## Validation

Run each of these and report its exit code:

- `cargo test --locked --workspace --profile ci --no-fail-fast`, recording the passed and ignored counts at `d8e5919e` and at the end. The count only grows by the new goldens.
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check` and `git diff --check`
- `git diff <step-1 commit> -- <all expectation files>` prints nothing, with both `TMPDIR=/tmp` and `TMPDIR=/private/tmp`.
- `rg 'Runtime::(Codex|ClaudeCode|Antigravity|Pi|Copilot|Trae)\b' crates/runner-backend/src/session` matches only tests, apart from the Codex Speed sites left for PR 3.
- `git log --follow` on two moved files shows their history.

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Do not launch real agent CLIs. Jason smoke-tests. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole branch diff against the spec and this brief, must-fix findings first with file:line pointers. It checks in particular that:

- the new goldens were committed first, pass on untouched `d8e5919e`, and no expectation changed afterwards;
- the coverage map leaves no moved site uncovered;
- every runtime's watcher, capture mechanism, trust seed, env and launch gate matches today's, including Windows-only and macOS-only paths;
- no `cfg` attribute was lost in the moves, and no moved test changed or disappeared;
- Shell and unknown keys behave exactly as today.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, the goldens commit, this brief and the spec edit included, into a single commit on top of `main`. Use an imperative subject such as `refactor(runtime): move spawn hooks and status watchers into runtime adapters`, with no co-author trailers.
- **Push** with `git push -u origin refactor/777-runtime-adapter-spawn`.
- **Open the PR** with `gh pr create --base main`. The body carries:
  - `Refs #777, PR 2 of 3`, not a closing keyword;
  - a summary;
  - test evidence: the new golden matrix, the coverage map, and the before and after counts;
  - a manual check for Jason: on Codex, Claude Code, Antigravity and pi, a chat whose status goes Working then Idle, an interrupt, `/new` or `/clear` followed by a resume, and one mission slot in an untrusted temporary folder, with nothing different;
  - what is unverified;
  - no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It gives the PR URL and CI result, changed files, checks with exit codes, any spec deviation, bugs found but not fixed, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
