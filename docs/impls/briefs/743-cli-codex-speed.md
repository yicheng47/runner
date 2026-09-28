# 743 — Configure Codex Speed through the CLI

Implement [#743](https://github.com/yicheng47/runner/issues/743). Jason explicitly requested this mission. Use the codex pair coder/reviewer loop and end with an open PR and green macOS/Windows CI.

## Workspace and scope

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-743-cli-codex-speed`, on the existing branch `feat/743-cli-codex-speed`, based on `origin/main` `bcb91fe` (merged #741). This brief is the initial branch commit. Do not create another branch or checkout, edit other worktrees or the root checkout, or share a Cargo target directory. Rebase onto current `origin/main` if needed; never merge main into this branch.

#741 added structured Codex Speed to roles, crew slots and Start Chat, but the CLI still exposes only model and effort. Complete CLI parity using those existing backend semantics. This is CLI/backend work; no new desktop UI or Pencil design is needed. Read the issue and `docs/features/740-codex-role-speed.md` before implementation.

## Required behavior

1. Add `--speed inherit|standard|fast` to `runner role create/update`. New roles default to Inherit. An omitted update preserves the saved value; explicit `inherit` clears it.
2. Add the same flag to `runner crew add/set` for the slot override. Omitted `set` preserves the override; explicit `inherit` clears it so the role's choice applies. New slots without an override inherit.
3. Add the same flag to `runner chat start`, for both role-backed and runtime-only Codex chats. Omitted or explicit `inherit` follows the role or Codex default. Standard/Fast is a per-chat override and survives Resume.
4. Show saved or effective Speed in the corresponding human CLI show output, including role, crew slots and chat/session details as appropriate, with unambiguous inheritance semantics. Retain Speed in JSON readback. Use the existing output patterns, and do not misrepresent an inherited choice as an explicit saved override.
5. Preserve non-Codex behavior and existing runtime, model, effort, Args and permission flags. Carry structured fields; do not rewrite raw `-c service_tier` Args. Follow #741's existing precedence, persistence and runtime-change semantics rather than introducing a second source of truth.

## Starting points

- `crates/runner-cli/src/command.rs`: RoleFields, CrewCommand Add/Set, Chat Start, structured requests, and CLI parser/request tests.
- `crates/runner-cli/src/output.rs`: role, crew-show and session-show rendering and tests.
- `crates/runner-backend/src/mcp/tools/session.rs`: StartDirectSessionArgs currently has no Speed and session_start_direct dispatches through paths that omit it. Extend the structured socket input and wire both role-backed and runtime-only starts into existing Speed-aware operations.
- `crates/runner-backend/src/ops/session.rs`: session_start_direct_with_speed and session_start_runtime_with_speed; follow existing session seed and Resume behavior.
- Role `codex_speed` and slot `codex_speed_override` already exist. Inspect the existing role/crew tools and update types for omission-versus-clear handling before changing them.

Keep the implementation scoped. Update relevant CLI documentation/help where needed; if either README changes, update both language versions. Do not run the dev app, modify Jason's live roles/crews/chats, or launch extra agents as tests. Use isolated test databases and existing fake-runtime fixtures.

## Review and verification

Cover omitted, explicit Inherit, Standard and Fast across create/update/add/set/start, including preservation on update, clearing a slot override back to the role, role-backed and runtime-only chats, mission-slot effective configuration, and chat Resume persistence. Verify CLI help and human/JSON readback. Reuse existing meaningful backend coverage where it already proves a path; add regression tests for new wiring and uncovered behavior. Check non-Codex behavior and existing override precedence.

Run and report exit codes for `cargo test --locked -p runner-cli -p runner-backend --profile ci --no-fail-fast`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`. CI must pass on macOS and Windows; report external blockers with evidence rather than claiming green.

Coder implements and validates, then explicitly requests reviewer inspection through Runner on the working-tree diff before committing code or opening a PR. Reviewer stays idle until that handoff. Iterate until reviewer reports `NO REMAINING MUST-FIX ISSUES`. No additional crews or subagents. Use Runner messages for handoffs and end the turn while awaiting a reply; do not poll another agent.

## Authorization and completion

After clean review, squash the brief and implementation into one commit on top of current `origin/main`, with an imperative subject naming the change and no co-author/session-link trailers. Push `feat/743-cli-codex-speed`, open a PR against `main` with `Closes #743`, the resulting CLI behavior, relevant validation evidence and any remaining manual smoke steps. Never include a Claude session URL in the PR body.

Watch CI with `gh pr checks <n> --watch`, resolve failures, send non-trivial fixes back through review, and keep the PR at one commit by amending and pushing with `--force-with-lease`. Stop at an open PR with green macOS/Windows CI. Do not merge, delete the branch or worktree, cut a nightly or release, or perform unrelated destructive operations.

The final Runner handoff includes the PR URL, review verdict, changed files, checks and exit codes, both platform CI results, and any remaining limitation or manual verification.
