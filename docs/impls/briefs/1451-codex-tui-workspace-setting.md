# #1451 — Codex TUI workspace launch option

## Starting point

- Issue: [#1451](https://github.com/yicheng47/runner/issues/1451)
- Branch: `feat/1451-codex-tui-workspace-setting`
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1451-codex-tui-workspace-setting` (base `origin/main` `4a958228`, local `target/`, never share `CARGO_TARGET_DIR`)
- Isolation: keep the root checkout on `main` and never touch sibling in-flight worktrees (currently `.worktrees/feat-1342-unified-markdown-renderer`).
- Initial commit on this branch holds the spec (`docs/features/1451-codex-tui-workspace-setting.md`), `docs/features/README.md` index entry, and this brief so the worktree starts clean. Fold this brief into the implementation commit(s) before pushing so it does not land as a standalone brief commit.

## Problem and outcome

Codex CLI 0.171.0+ enables `features.tui_workspace` by default, turning on its built-in TUI sidebar file renderer and hiding local Markdown file-link destinations without emitting `file://` OSC 8 links. That prevents Runner's terminal link detection from recognizing Codex file links and causes clicks to open Codex's TUI sidebar instead of Runner's Files panel.

Outcome: add a Codex-only **TUI workspace** switch (**Off by default**) in **Settings → Agents → Codex → Launch**, mirroring **Remote plugin catalog**. When off, Runner passes `-c features.tui_workspace=false` on interactive Codex launches (chats and missions, leaving `~/.codex/config.toml` and headless `codex exec fork` untouched), previews and annotates that line in Settings, and reserves `features.tui_workspace` in Custom flags.

The spec [`docs/features/1451-codex-tui-workspace-setting.md`](../../features/1451-codex-tui-workspace-setting.md) is authoritative for behavior, copy, and checks.

## Read first

- `docs/features/1451-codex-tui-workspace-setting.md`
- `crates/runner-core/src/protocol/runtime.rs` (`RuntimeLaunchOptions`, `RuntimeLaunchOption`, `RuntimeCommandNote`) and `crates/runner-core/src/protocol/tests.rs`
- `crates/runner-daemon/src/runtimes/mod.rs`, `crates/runner-daemon/src/runtimes/codex/mod.rs` (`Codex::launch_args`), `crates/runner-daemon/src/session/manager/spawn_spec.rs` (`compose_preview_args`, `command_lines`, launch option wiring)
- `crates/runner-daemon/src/runtimes/custom_flags.rs` (`reserved_flags` and its `features.remote_plugin` tests)
- `crates/runner-daemon/src/session/manager/tests/expectations/spawns.json` (plus `forks.json`, `launch_gate.rs`, `resume.rs`, `permissions.rs`)
- `crates/runner-app/src/surfaces/settings/agents/launch.rs`, `crates/runner-app/src/surfaces/settings/agents/tests.rs`, `crates/runner-app/i18n/{en,zh-hans}.toml`
- `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md`, `docs/arch/runtime-integration.md`

## Deliverables

1. **Core protocol (`runner-core`).** Add `tui_workspace: bool` (default `false`, `#[serde(default)]`) to `RuntimeLaunchOptions`, `RuntimeLaunchOption::TuiWorkspace` (Codex-only), and `RuntimeCommandNote::TuiWorkspaceOff`. Verify legacy JSON without `tui_workspace` deserializes as `false`.
2. **Daemon launch, preview & custom-flags guard (`runner-daemon`).** Emit `-c features.tui_workspace=false` in interactive Codex launches (chats and missions) when `options.tui_workspace` is `false`; keep headless `codex exec fork` unchanged; do not leak a new env key into default Codex child `spec.env` (for example, only pass an env override when `options.tui_workspace` is `true`, or pass the flag directly on `LaunchContext`). Annotate `-c features.tui_workspace=false` in `command_lines` with `RuntimeCommandNote::TuiWorkspaceOff` and `option: Some(RuntimeLaunchOption::TuiWorkspace)`. Include `tui_workspace` in `custom_flags::reserved_flags` so `-c features.tui_workspace=...` and `--config features.tui_workspace=...` are refused. Update Codex spawn goldens (`spawns.json` and any affected assertions in `launch_gate.rs`, `resume.rs`, `permissions.rs`, `spawn_spec.rs`, `custom_flags.rs`, `ops/runtime_tests.rs`).
3. **Settings UI & localization (`runner-app`).** Add the `TuiWorkspace` toggle row to Codex's Launch card (`AGENT_LAUNCH_SWITCH_codex_TuiWorkspace`), with `Chats and missions` badge, `Off: -c features.tui_workspace=false` monospace hint, bilingual strings in `en.toml` and `zh-hans.toml` (keep description platform-neutral — no macOS-only `⌘` glyph), and `preview_note` mapping. Update and add tests in `surfaces/settings/agents/tests.rs`.
4. **Docs.** Update `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md`, and `docs/arch/runtime-integration.md` (English and Chinese sections).

Non-goals: modifying `~/.codex/config.toml`; using `--disable tui_workspace`; adding the toggle to non-Codex runtimes; changing terminal link parsing in `runner-terminal` (#1452); adding CLI commands.

## Validation and handoffs

- **@coder:** Run `cargo test -p runner-core`, `cargo test -p runner-daemon`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, and `git diff --check`. If any new test waits on background work, run `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'` and state the count. Hand `@reviewer` a local unpushed commit SHA with summary, changed files, and checks run (no push or PR yet).
- **@reviewer:** Review the handed-off commit SHA against the spec, checking default-off serde compatibility, clean child `env_keys`, custom-flags reservation, golden updates, i18n parity, and docs. Name the selected live checks and candidate commit SHA in the clean review.
- **@qa:** Acquire `script/qa-lock acquire --timeout 540` before launching the macOS development app (remember sibling mission #1342 may also use the lock; wait cleanly if held, and preserve any pre-existing dev user sessions). Run `CTW-01`, `CTW-02`, and `CTW-03` from the spec, record pass/fail per check with screenshots in `docs/tests/runs/1451-codex-tui-workspace-setting-mac.md`, archive only QA-created sessions, clean up `/tmp`, and release `script/qa-lock release`. Report the verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (3 slots: `coder`, `reviewer`, `qa`). Once `@reviewer` reports a clean review and `@qa` reports its verdict, `@coder` may shape the reviewed commits (folding this brief in), push `feat/1451-codex-tui-workspace-setting`, open a GitHub PR against `main` with `Fixes #1451`, apply `risk-low` (preserving `core-review` if workflow-applied; keep `needs-live-test` only if any required live check has not passed, otherwise omit it once macOS live QA passes and note platform scope honestly), confirm `@reviewer` applies `reviewed` on the PR head SHA, and drive CI green across all platforms. Do not merge.
