# #1248 — Inline test extraction validation

Changed production behavior: none. No live checks are required because this change only relocates test modules; the app and real agent sessions were not launched.

## Current baseline and inventory

The current baseline is `origin/main` at `c2fbafd9b41979d5ff18e9f752672584baa4a955`, including merged PRs #1274 and #1275. After rebasing, the complete inventory remains 22 production files and 28 distinct inline modules, now totaling 38,294 lines including module boundaries. Every eligible file is accounted for below. Paths are relative to each crate’s `src/`; each module now lives in the conventional child file with its own name.

| Crate | Production file | Extracted modules | Original lines | macOS tests |
| --- | --- | --- | ---: | ---: |
| `runner-app` | `keymap.rs` | `tests` | 1,388 | 25 |
| `runner-app` | `surfaces/app_shell/mod.rs` | `tests` | 1,546 | 20 |
| `runner-app` | `surfaces/panes/mod.rs` | `tests` | 3,716 | 58 |
| `runner-app` | `surfaces/settings/agents/mod.rs` | `tests` | 1,167 | 31 |
| `runner-app` | `surfaces/settings/page/mod.rs` | `tests`, `localization_tests` | 2,105 | 31 |
| `runner-app` | `surfaces/settings/remote.rs` | `tests` | 1,619 | 15 |
| `runner-app` | `surfaces/start_chat/launcher/mission.rs` | `tests`, `remote_tests` | 1,469 | 24 |
| `runner-app` | `surfaces/start_chat/launcher.rs` | `tests`, `remote_tests` | 2,596 | 40 |
| `runner-app` | `surfaces/update_ui.rs` | `tests` | 1,053 | 33 |
| `runner-cli` | `command.rs` | `tests` | 2,093 | 45 |
| `runner-daemon` | `ops/mission/report.rs` | `tests` | 1,209 | 23 |
| `runner-daemon` | `ops/mission.rs` | `tests`, `project_tests` | 2,567 | 56 |
| `runner-daemon` | `ops/node.rs` | `tests` | 1,468 | 29 |
| `runner-daemon` | `ops/role.rs` | `tests` | 1,715 | 45 |
| `runner-daemon` | `ops/session.rs` | `tests`, `client_tests`, `launch_definition_output_tests` | 1,853 | 46 |
| `runner-daemon` | `ops/skills.rs` | `tests` | 1,021 | 24 |
| `runner-daemon` | `runtimes/claude_code/claude_status.rs` | `tests` | 1,898 | 38 |
| `runner-daemon` | `runtimes/codex/codex_status.rs` | `tests` | 1,105 | 24 |
| `runner-daemon` | `runtimes/copilot/copilot_status.rs` | `tests` | 1,060 | 21 |
| `runner-daemon` | `session/pty_runtime.rs` | `tests` | 2,868 | 50 |
| `runner-terminal` | `input_state.rs` | `tests` | 1,733 | 47 |
| `runner-terminal` | `terminal.rs` | `tests` | 1,045 | 27 |

Conflict resolution retained the current main’s complete production files and tests before reapplying the mechanical extraction. This preserves the new keymap action and defaults, changed keymap/settings assertions, both new inline tests, the app-shell listener, and the terminal key-classification changes. The extraction adds no production changes of its own.

Open PR #1277 (`fix/1271-usage-preview`) was checked during the rebase and reported to the operator because its app-shell and settings-page production files overlap this inventory. Its branch/worktree was not touched. If it lands before readiness, the repository catch-up rule applies.

## Test identities and source equivalence

- macOS arm64, Rust 1.97.1, default workspace features: `cargo test --workspace -- --list` and `cargo test --workspace -- --list --ignored` passed against the new main baseline and after extraction. All 4,060 `(target, fully qualified test name)` identities and all 7 ignored statuses match exactly. The moved modules account for 752 host-discoverable tests, including the unchanged ignored manual tracker measurement.
- The sorted manifest contains `target<TAB>test name<TAB>ignored` with a trailing newline per entry; its SHA-256 is `ab325f72d6e8e27926423ebc491e8d3776e841dc0701642ff1d040555b975e81`.
- A syntax-tree audit found 770 test functions across the moved modules. The 18 functions unavailable on this host retain their attributes and bodies: four keymap tests gated on Windows/Linux, one Windows shell-command test, one Linux non-UTF-8 skill-path test, and 12 Windows PTY tests (including both subprocess probes).
- All 22 production files are byte-identical to the new main baseline after replacing the selected inline module bodies with external module declarations. Every extracted body matches an independently Rustfmt-formatted baseline body at the token level, preserving multiline string contents, helpers, assertions, test attributes, and ignored reasons. The only normalized token changes are the two source-relative `include_str!` paths.
- The CLI golden remains `crates/runner-cli/tests/goldens/cli.txt`; its include becomes `../../tests/goldens/cli.txt`. The PTY JSON remains in `session/fixtures/`; its include becomes `../fixtures/codex-0.155.1-idle.json`. No fixture content changed.
- The Windows `--exact session::pty_runtime::tests::batch_argument_probe` and `--exact session::pty_runtime::tests::hook_argv_batch_probe` strings and module paths are unchanged. `CARGO_MANIFEST_DIR` fixture lookups stay unchanged; no moved code uses `file!`, `line!`, `column!`, or `module_path!`.
- Module attributes remain in place, including `#[cfg(all(test, debug_assertions))]` on `localization_tests`. No modules were merged, no visibility was widened, and test-only helpers outside the selected modules remain in place.

## Current validation

- `cargo fmt --all --check` and source-equivalence audit: passed.
- `git diff --check` and `git diff --cached --check`: passed.
- `make verify`: passed completely with exit 0 against the new baseline: workspace check, workspace tests (4,053 passed, seven intentionally ignored), workspace Clippy, updater-feature Clippy, and formatting. Full output and exit code are preserved in the current rebase artifact directory.
- A fresh independent review and all macOS, Windows, Linux x86_64, and Linux arm64 CI gates are required for the rebased head. The previous head’s review and CI do not establish readiness for this candidate.

## Earlier candidate (historical)

Before main advanced, candidate `64389ce5f9ca2f454331c632ce3ed76daf5ffe3c` used baseline `b841b51a63fae25426fd9a36b4da23b927213efa`. It preserved 4,048 host identities and seven ignored statuses, with 768 moved source tests (750 host-discoverable). Those counts describe the earlier baseline and are not fixed expectations for the rebased candidate.

That candidate’s workspace check, 4,041 executing tests, workspace Clippy, and updater-feature Clippy passed. Its initial `make verify` exited 2 at final formatting because the first format pass left leading blank lines; removing only those blank lines made the formatting check and repeated source audit pass. All required CI subsequently passed. Linux arm64’s initial CI attempt failed an unchanged remote-client recovery-click test (zero `runtimeTaskStart` requests instead of one); the complete job passed on a same-commit retry with no code or test changes. The original failure and retry remain recorded in PR #1276 and local logs.

Current rebase logs, original main source snapshots, syntax audit, test manifest, comparison results, and exit codes are under `/tmp/1248-tests-rebase-artifacts/`. Earlier-candidate evidence remains under `/tmp/1248-tests-artifacts/`.
