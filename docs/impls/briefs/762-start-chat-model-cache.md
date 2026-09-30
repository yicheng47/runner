# 762 — Start chat modal model cache inconsistency

Fix [#762](https://github.com/yicheng47/runner/issues/762). Jason explicitly asked on 2026-09-30 for a `codex pair` crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-762-start-chat-model-cache`, on branch `fix/762-start-chat-model-cache`. The mission's directory is this worktree. Its tip is this brief, on top of `origin/main`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If `main` moves and you need it, rebase onto `origin/main`; never merge `main` into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **Issue [#762](https://github.com/yicheng47/runner/issues/762)**: `bug: start chat modal model is fetched from 10-minute local cache, causing potential inconsistency`.
- `crates/runner-backend/src/runtime_status/models.rs`: `REFRESH_SECONDS` (10-minute TTL), `ModelDiscovery::begin`, `request`.
- `crates/runner-backend/src/ops/runtime.rs`: `runtime_request_models`, `runtime_catalog`.
- `crates/runner-app/src/surfaces/start_chat.rs`: `open_start_chat`, `model_placeholder`, `model_field`, `sync_model_marker`, `build_start_request`, and tests.

## Deliverable

1. **Resolve the model inconsistency in the Start Chat modal.**
   - In `model_placeholder` (`crates/runner-app/src/surfaces/start_chat.rs`), when starting a chat without an explicit role model or user override, the launch request sends no model override (`model: None`), leaving the CLI agent runtime to pick its own default. However, `model_placeholder` currently displays `runtime.default_model` from the local cache. If that cached value is stale (up to 10 minutes old) or differs from the CLI's actual configuration, displaying a specific model name is misleading.
   - Change `model_placeholder` so that when there is no explicit role model, it displays `"default"` (dimmed placeholder), accurately communicating that the agent runtime's own default will be used without pinning to a potentially stale cached name.
   - Additionally, evaluate runtime model discovery on modal start / runtime selection: determine whether `open_start_chat` or `runtime_request_models` should trigger background model refreshes without being blocked by the 10-minute TTL, or whether the 10-minute cache should apply only to routine periodic polling rather than explicit user interactions.
   - Keep the solution simple and robust: avoid blocking the UI thread on CLI queries, preserve fast modal opening, and ensure consistent behavior across Role mode and Direct mode.

2. **Tests.**
   - Update and add focused tests in `crates/runner-app/src/surfaces/start_chat.rs` covering `model_placeholder` behavior in Role mode (with and without explicit role model) and Direct mode.
   - Verify that blank model input continues to produce `model: None` in `build_start_request`.
   - Add/update any relevant backend tests in `crates/runner-backend` if discovery or catalog functions are adjusted.

3. **Out of scope.**
   - Unrelated settings or role pages.
   - Changes to CLI agent arguments other than the model cache/placeholder handling.

## Validation

Run each of these in the worktree and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`).

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against issue #762 and this brief, must-fix findings first with file:line pointers.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief included, into a single commit on top of `main`: imperative subject naming the change (e.g. `fix(ui): avoid stale model placeholder in start chat modal`), no co-author trailers.
- **Push** with `git push -u origin fix/762-start-chat-model-cache`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #762`, a summary, test evidence, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.
- The final Runner handoff states the PR URL, review verdict, CI results, changed files, checks and remaining manual verification.
