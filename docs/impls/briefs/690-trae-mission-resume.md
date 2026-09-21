# Mission brief: fix Trae mission resume (#690)

## Goal

Fix [#690](https://github.com/yicheng47/runner/issues/690): a persisted Trae mission slot must resume after Runner restarts without Trae rejecting the launch configuration, and the first user turn after resume must target the restored thread rather than fail with `thread not found`.

This mission is unit-test driven. Do not start a live Trae agent session or spend Trae quota; this machine has the CLI but no usable subscription for an end-to-end conversation. The reporter's screenshot is the live reproduction evidence.

## Confirmed current behavior

- Runner v0.11.2 maps Trae mission Bypass to `--permission-mode bypass_permissions`.
- Mission resume reapplies the app-wide permission mode before composing argv.
- Trae `thread/resume` rejects the resumed configuration because `approvals_reviewer` and `permission_mode` overrides cannot both be set. Its TUI stays alive without a valid thread, so the next input fails with `turn/start ... thread not found`.
- `resume_plan` currently groups Trae with Codex and emits the positional prefix `resume <uuid>`.
- The installed Trae CLI 0.120.52 help and `traecli doc session-management` document `--resume <uuid>`, not a `resume` subcommand. Its bundled non-interactive examples explicitly combine `--resume "$SESSION"` with `-y`, so inspect the local help/manual to select the documented bypass form without launching a session.
- Existing tests independently pin fresh Trae mission permission args and the old resume plan, but no test composes the two through `SessionManager::resume`.

## Required implementation

1. Add a regression test at the session-manager/fake-runtime level that starts or seeds a Trae mission session with a captured UUID, resumes it under the default mission Bypass posture, and asserts the complete effective argv. The test must fail on v0.11.2 for the reporter's condition.
2. Correct Trae's resume argv to the documented `--resume <uuid>` flag form. Do not change Codex's `resume <uuid>` subcommand behavior.
3. Ensure a resumed Trae mission uses a Trae-documented bypass form that does not send the conflicting `permission_mode` override. Prefer the CLI's documented `--yolo`/`-y` resume combination if the local manual confirms it. Preserve the intended unattended Bypass posture rather than silently downgrading the mission to prompting.
4. Keep fresh Trae missions working and canonicalize legacy stored `--permission-mode bypass_permissions` rows so the final argv has one permission posture. Update `strip_permission_flags`, inference, and UI description/tests only where required by the chosen canonical form.
5. Keep direct chats, Claude Code, Codex, Copilot, Pi, shell, model/effort args, first-turn suppression, mission identity/env, and app-wide permission modes unchanged outside this Trae-specific correction.
6. Do not add output-text scraping for this known argument conflict unless the argv correction cannot solve it. The current fast-exit fallback is a separate generic limitation.

## Validation

- Run the new focused Trae mission-resume regression test and the existing Trae permission/resume tests.
- Run all `runner-backend` tests.
- Run workspace Clippy with warnings denied, or `make clippy` if it is the repository's equivalent current command.
- Run formatting checks.
- Review the working-tree diff through the crew reviewer. Leave implementation changes uncommitted; do not push or open a PR.

## Definition of done

- The fake-runtime regression proves a resumed Trae mission composes documented resume argv and no conflicting `--permission-mode` override.
- Fresh and resumed Trae mission Bypass remain non-interactive.
- Relevant tests and checks pass.
- The reviewer reports no remaining must-fix findings through Runner.
- No live Trae request, commit, push, PR, or merge is performed.
