# #1251 — OpenCode v2 label, v1 detection as not installed, and v2 compatibility gate

Tracking issue: [#1251](https://github.com/yicheng47/runner/issues/1251).

Jason approved the **`agy trio`** crew on 2026-11-15. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1251-opencode-v2` (canonical `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1251-opencode-v2`) on branch `fix/1251-opencode-v2`, based on `origin/main` at `c2bf622f`. Keep the root checkout on `main`.

## Problem

Runner's OpenCode integration targets **OpenCode v2** (`2.0.0+`), launching `opencode --standalone` for private-server isolation, querying models with `serve --stdio`, and reading `session_v2` history. OpenCode v1 (`0.15.x`, `1.19.x`) uses the same `opencode` command without `--standalone`. When OpenCode v1 is on `PATH` or set as an override, Runner lists OpenCode as installed and spawns `opencode --standalone`, which prints CLI help and exits in ~0.4s (`crashed`). OpenCode has no Runner-managed background update command (`update_args: &[]`).

## Deliverables (settled with Jason, 2026-11-15)

1. **Label the runtime `OpenCode v2`:**
   - Set `Runtime::OpenCode`'s display name to `"OpenCode v2"` in `crates/runner-core/src/runtime.rs` and `opencode::catalog()` in `crates/runner-core/src/protocol/runtime_metadata.rs`, with `install_url: "https://opencode.ai/v2/docs"`.
   - Keep `McpClientId::OpenCode` wire/serialized identifiers (`"opencode"` / `"OpenCode"`) intact; update catalog/UI expectations intentionally.

2. **Treat detected OpenCode v1 (`< 2.0.0`) as Not installed:**
   - Set `min_version: Some("2.0.0")` on `opencode::catalog()`.
   - In `crates/runner-daemon/src/runtime_status/versions.rs` and `crates/runner-daemon/src/runtime_status.rs`, ensure candidate `opencode` executables (detected on `PATH` or overridden) still have `--version` probed even when an incompatible version is withheld from `effective_command`. When evaluating an unprobed executable synchronously (override validation or launch before background probes complete), probe `--version` with `PROBE_TIMEOUT`.
   - When an auto-detected `opencode` on `PATH` (no override) reports `< 2.0.0`:
     - Set `RuntimeRowState::NotFound`, `effective_command: None`, `effective_source: None`, and `below_minimum: true`, while keeping `detected_path` and `installed_version` populated on `RuntimeExecutableStatus`.
     - This marks `available: false` in `runtime_catalog` (excluding it from the Launcher's installed agents) and places **OpenCode v2** under **Not installed** in **Settings → Agents**.
     - In **Settings → Agents** (`crates/runner-app/src/surfaces/settings/agents/presentation.rs`, `render.rs`, `crates/runner-app/i18n/en.toml`, `crates/runner-app/i18n/zh-hans.toml`), when a `NotFound` runtime has `below_minimum` (`detected_path` + `installed_version`), show in its row detail, caption, and Auto path placeholder that OpenCode `<installed>` was found at `<path>` and Runner only supports **OpenCode v2 (`2.0.0` or newer)**.

3. **Reject OpenCode v1 overrides, model discovery, and launches cleanly:**
   - **Override validation:** In `validate_override` and `status_list` (`crates/runner-daemon/src/runtime_status.rs`), reject an explicit override whose probed `--version` is below `min_version` (`2.0.0`) as `RuntimeRowState::InvalidOverride` (`effective_command: None`, `effective_source: None`) with a message that `<installed>` was found and Runner only supports **OpenCode v2 (`2.0.0` or newer)**.
   - **Launch & model discovery gate:** In `resolve_role_executable` (`crates/runner-daemon/src/session/manager/spawn_spec.rs`), probe `--version` synchronously when `min_version.is_some()` and `installed` is not cached yet; refuse `< 2.0.0` before spawning `--standalone`. Skip background model discovery (`crates/runner-daemon/src/runtime_status/models.rs`) when below `min_version`.
   - **Non-updatable recovery & classifier fallback:** Add `OpenCode::classify_startup_error` (`crates/runner-daemon/src/runtimes/opencode/mod.rs`) for non-zero early exits on `--standalone`. In `crates/runner-app/src/surfaces/agent_recovery.rs` and `crates/runner-app/src/surfaces/start_chat/launcher.rs`, when a gated/recovery runtime has `update_args.is_empty()`, offer **Open Settings** (`Settings → Agents`) instead of **Update** and never call `start_target_update`.

4. **Docs:** Update `docs/guide/reference/supported-agents.md`, `docs/guide/guides/troubleshooting.md`, and their `zh-CN` mirrors.

## Validation

- **Automated:** `cargo test -p runner-core -p runner-daemon -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `--features updater`, `cargo fmt --all -- --check`, `git diff --check`, `python3 script/check-guide-translations.py`.
- **Live QA (`qa` slot):** Use `script/qa-lock acquire --timeout 540` and record in `docs/tests/runs/1251-opencode-v2-mac.md`:
  1. Scratch `opencode` reporting `1.19.5` shows **OpenCode v2** under **Not installed** / invalid override with the v2 requirement explanation, and stays out of the Launcher's installed agents.
  2. Resuming a stopped OpenCode chat against `1.19.5` is refused without crashing into CLI help and offers **Open Settings** instead of a broken update task.
  3. Compatible `2.0.29` executable appears under **Installed** and starts with `--standalone`.

## Authorization

Jason authorized **`agy trio`** to implement, review, live-test in the macOS dev app, commit (folding this brief in), push `fix/1251-opencode-v2`, open one PR against `main`, and drive CI green. Do not merge.
