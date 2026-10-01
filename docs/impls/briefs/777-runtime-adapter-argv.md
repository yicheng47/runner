# 777 PR 1 — Runtime adapter: identity and the argv layer

Implement PR 1 of 3 for [P1 #777](https://github.com/yicheng47/runner/issues/777): phases 0 and 1 of the spec. Jason asked on 2026-10-01 for a crew mission that ends in an open PR, not a merge. This is a refactor: behavior stays byte-identical.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-777-runtime-adapter-argv`, on branch `refactor/777-runtime-adapter-argv`. The mission's directory is this worktree. Its tip is this brief and a spec correction, on top of main `dad891f9`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

On 2026-10-01 Jason authorized rebasing onto `f6f7cc0a` after #778 and the design sync landed. Temporary WIP commits preserved the implementation during history edits, then mixed resets restored the working-tree review. Review found a phase-0 temp-path normalization defect: the headless fixture's canonical cwd leaked `/private` into `<TMP>`. Jason explicitly approved correcting only `forks.json:80` from `cwd=/private<TMP>` to `cwd=<TMP>` and required the normalizer fix and fixture correction to be amended into the goldens-only commit. That amended commit is `76af413a` (replacing `4de64f40`); its full workspace suite passes on otherwise unmodified `f6f7cc0a` with both `TMPDIR=/tmp` and `TMPDIR=/private/tmp`, each with 1,856 passed and 3 ignored. The expectation freeze now starts at `76af413a`. Both `dad891f9` and `f6f7cc0a` have 1,851 passed and 3 ignored workspace tests; the review baseline is `f6f7cc0a`.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/777-runtime-adapter.md`**: Proposal, Rules, Decisions, and phases 0 and 1. It wins over the issue and over this brief on any detail.
- `docs/impls/briefs/582-split-test-files.md`, Rules of the road: the precedent for a move with no behavior change.
- `runner-backend/src/model.rs:164`: `Runtime`.
- `router/runtime.rs`: `RUNTIME_DEFINITIONS` :47, `model_effort_args` :191, the hook-arg builders :264–468 (`claude_settings_args`, `codex_status_args`, the Copilot, pi and agy status args), the permission functions :534–990, `system_prompt_args` :992, `first_turn_argv` :1044, `trailing_runtime_args` :1082, `mission_bus_sandbox_args` :1145, `resume_plan` :1224, `fork_plan` :1308, the conversation probes :1391 on, and its 88 tests.
- `router/prompt.rs:26`: `split_session_prompt` and pi's lead channel.
- `ops/runtime.rs`: `RuntimeDefinition` :12, `runtime_catalog` :300, `runtime_catalog_options` :420.
- `session/manager/spawn.rs`: `apply_runtime_args` :629, the conversation-missing block around :2596, the fresh-fallback persona around :2790.
- `session/manager/tests/mod.rs:125`: `FakeRuntime` and `FakeSpawn`, the capture point for the golden tests.
- `runner-core/src/lib.rs:14` `RUNNER_SKILL_ROOTS`; `runner-cli/src/command.rs:2240` `runtime_command`.

## Deliverable

### Step 1: golden tests (phase 0), before any code moves

1. **Spawn goldens** in `session/manager/tests/golden.rs`, with expectation files beside it. Drive the real `SessionManager` entry points through `FakeRuntime` and record each `SpawnSpec`'s command, args, and the env keys Runner adds. The matrix covers all six runtimes and Shell:
   - direct chat: fresh with persona, model and effort; blank; genuine resume; resume with missing history;
   - mission worker and lead under each `MissionPermissionMode`;
   - fork for each runtime with native fork;
   - Codex with Speed set, Claude role args carrying `--settings`, Codex args disabling hooks;
   - an unknown runtime key.
2. **Pure goldens:**
   - the stored role args from `apply_permission_mode` for each runtime and mode, starting from args that carry every runtime's flags;
   - `infer_permission_mode` on those rows;
   - `runtime_catalog_options()`, serialized;
   - the socket tools' input schemas, so the enum's move to core is proven schema-neutral.
3. **Determinism.** Normalize temp dirs, home, ULIDs, and UUIDs (numbered by first appearance). Expectations regenerate only with `RUNNER_UPDATE_GOLDEN=1`; otherwise a mismatch fails with a readable diff.
4. **`#[cfg(unix)]`.** Hook support differs on Windows and nobody here can record Windows output; Windows keeps its existing tests. Say so in the PR.
5. **Commit this step alone, locally,** green against unmodified code. From then on the expectation files stay byte-identical. If an expectation has to change, behavior changed: stop and report.

### Step 2: trait, registry and identity (phase 1)

6. **`Runtime` moves to `runner-core/src/runtime.rs`** with the same derives, variants, wire names, `ALL`, `key`, `parse` and `Display`. `schemars` becomes an optional core dependency behind a `schemars` feature that `runner-backend` enables. `runner_backend::model::Runtime` becomes a re-export.
   - Add `display_name`, `command`, `managed_skill_root` and `is_shell`.
   - The CLI's `runtime_command` reads them and keeps its error text.
   - `RUNNER_SKILL_ROOTS` stays a const, with a core test that it holds exactly the managed roots.
7. **`runner-backend/src/runtimes/`.** `mod.rs` holds `RuntimeAdapter`; `adapter(Runtime)`, the only exhaustive `match`; `for_key(&str)`, which returns `&NoAgent` for Shell and unknown or legacy keys; and `NoAgent`. Add one folder per runtime: `codex/`, `claude_code/`, `antigravity/`, `pi/`, `copilot/` and `trae/`.
   - This PR's members are `catalog`, `model_effort_args`, `first_turn_argv`, `prompt_channels`, `permissions`, `launch_args`, `mission_dir_args`, `resume_plan`, `fork_plan`, `conversation_exists` and `missing_conversation`.
   - Do not add the PR 2 and PR 3 members yet.
8. **Move the per-runtime logic into the adapters.** It comes out of `router/runtime.rs`, `router/prompt.rs`, the `ops/runtime.rs` catalog, and the conversation-missing and persona-resend arms in `spawn.rs`.
   - **One catalog.** `RUNTIME_DEFINITIONS`, the static parts of `runtime_catalog_options` and `ops::runtime::RuntimeDefinition` all come from `catalog()`; `ops` fills the live fields as it does now.
   - **Hook arguments.** The hook-arg builders move into their adapter and are called from its `launch_args`. Their `hooks_supported` gating and the env set in `spawn.rs` stay as they are until PR 2.
9. **Shared mechanics become helpers in `runtimes/`:** flag stripping and matching, Go boolean flags, the UUID check, and conversation-file lookups. Codex and TRAE share helpers; no adapter calls another adapter.
10. **`router/runtime.rs` keeps only shared types:** `ResumePlan`, `ForkPlan`, `PermissionMode`, `MissionPermissionMode` and `FIRST_TURN_ARGV_MAX_BYTES`. It keeps no per-runtime function and no wrapper. Callers in every crate, `runner-app` included, call the adapter; a caller holding a key uses `for_key`.
11. **Tests.** The `router/runtime.rs` tests move with their adapter. Only call paths change; expected values stay untouched. Gate any import or helper used only by `cfg(unix)` tests with `cfg(unix)`, because Windows clippy fails on unused ones.
12. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope:
- **PR 2:** env, trust, the launch gate, key capture, status watchers, and `pty_runtime.rs`.
- **PR 3:** native defaults, model discovery, versions, usage, skills, MCP, and the app's tables.
- **Not at all:** renamed columns or fields, and any behavior change, including bugs found on the way. List those bugs in the handoff.

## Validation

Run each of these and report its exit code:

- `cargo test --locked --workspace --profile ci --no-fail-fast`, recording the passed and ignored counts at `dad891f9` and at the end. The count only grows by the goldens.
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check` and `git diff --check`
- `git diff <step-1 commit> -- <golden expectation files>` prints nothing.
- `rg 'Runtime::(Codex|ClaudeCode|Antigravity|Pi|Copilot|Trae)\b' crates/runner-backend/src/router crates/runner-backend/src/ops/runtime.rs crates/runner-cli/src` matches only tests.

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Do not launch real agent CLIs. Jason smoke-tests. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole branch diff against the spec and this brief, must-fix findings first with file:line pointers. It checks in particular that:

- the goldens were committed first and pass on unmodified code (check them out over `f6f7cc0a`), and no expectation changed afterwards;
- the matrix leaves out no runtime, launch shape or mode;
- no moved test's expected value changed, and no test disappeared;
- no per-runtime `match` or wrapper is left in `router/`, the catalog, or the CLI;
- Shell and unknown keys produce today's argv.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, the goldens commit, this brief and the spec edit included, into a single commit on top of `main`. Use an imperative subject such as `refactor(runtime): move agent runtime argv into RuntimeAdapter modules`, with no co-author trailers.
- **Push** with `git push -u origin refactor/777-runtime-adapter-argv`.
- **Open the PR** with `gh pr create --base main`. The body carries:
  - `Refs #777, PR 1 of 3`, not a closing keyword;
  - a summary;
  - test evidence: the golden matrix and the before and after counts;
  - a manual check for Jason: on Codex, Claude Code, Antigravity and pi, one chat and one mission slot each, a resume, and a model and effort override, with nothing different;
  - what is unverified;
  - no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It gives the PR URL and CI result, changed files, checks with exit codes, any spec deviation, bugs found but not fixed, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
