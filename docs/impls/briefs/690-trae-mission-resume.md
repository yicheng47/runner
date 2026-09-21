# Mission brief: fix Trae mission resume (#690)

## Goal

Fix [#690](https://github.com/yicheng47/runner/issues/690): a persisted Trae mission slot must resume after Runner restarts without Trae rejecting the launch configuration, and the first user turn after resume must target the restored thread rather than fail with `thread not found`.

This mission is unit-test driven. Do not start a live Trae agent session or spend Trae quota; this machine has the CLI but no usable subscription for an end-to-end conversation. The reporter's screenshot is the live reproduction evidence.

## Confirmed current behavior

- Runner v0.11.2 maps Trae mission Bypass to `--permission-mode bypass_permissions`.
- Mission resume reapplies the app-wide permission mode before composing argv.
- Trae `thread/resume` rejects the resumed configuration because `approvals_reviewer` and `permission_mode` overrides cannot both be set. Its TUI stays alive without a valid thread, so the next input fails with `turn/start ... thread not found`.
- The reporter uses the Trae Next Codex-derived runtime. Its existing positional `resume <uuid>` prefix reached `thread/resume` with the captured UUID, so that resume form is not the reported failure and must remain unchanged.
- This machine's `traecli` 0.120.52 is the separate Go-based [TraeCode CLI 2.0](https://docs.trae.cn/cli_get-started-with-trae-cli). Its bundled `--resume` and `--yolo` documentation does not establish argv compatibility with the reporter's runtime; its optional-value `--resume` flag also requires `--resume=<uuid>` to bind a UUID under pflag rather than the documented separated form.
- Existing tests independently pin fresh Trae mission permission args and the old resume plan, but no test composes the two through `SessionManager::resume`.

## Required implementation

1. Add a regression test at the session-manager/fake-runtime level that starts or seeds a Trae mission session with a captured UUID, resumes it under the default mission Bypass posture, and asserts the complete effective argv. The test must fail on v0.11.2 for the reporter's condition.
2. Preserve Trae's existing `resume <uuid>` prefix exactly. Do not apply the local Go CLI's `--resume` flag contract to the Trae Next adapter, and do not change Codex's resume behavior.
3. When resuming an existing Trae mission thread, strip Trae `--permission-mode` args after resolving the resume plan so `thread/resume` can use the approvals reviewer already carried by the restored thread without the conflicting override.
4. Keep fresh Trae mission Bypass unchanged as `--permission-mode bypass_permissions`; do not canonicalize it to the Go CLI's `--yolo` form or widen its sandbox posture.
5. Keep direct chats, Claude Code, Codex, Copilot, Pi, shell, model/effort args, first-turn suppression, mission identity/env, and app-wide permission modes unchanged outside this Trae-specific correction.
6. Do not add output-text scraping for this known argument conflict unless the argv correction cannot solve it. The current fast-exit fallback is a separate generic limitation.

## Validation

- Run the new focused Trae mission-resume regression test and the existing Trae permission/resume tests.
- Run all `runner-backend` tests.
- Run workspace Clippy with warnings denied, or `make clippy` if it is the repository's equivalent current command.
- Run formatting checks.
- Review the working-tree diff through the crew reviewer before committing. After a clean review, follow the human-authorized PR, CI, and merge workflow.

## Definition of done

- The fake-runtime regression proves a resumed Trae mission keeps `resume <uuid>`, preserves unrelated args, suppresses the first turn, and sends no conflicting `--permission-mode` override.
- Fresh Trae mission Bypass remains `--permission-mode bypass_permissions`; resumed threads rely on their restored approvals reviewer.
- Relevant tests and checks pass.
- The reviewer reports no remaining must-fix findings through Runner.
- No live Trae request is performed. The PR records the target correction and the absence of a live end-to-end smoke test; CI is green before merge.
