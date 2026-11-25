# #1479 — Gate Antigravity CLI on 1.5.8+ and support update checks and `agy update`

## Starting point

- Issue: [#1479](https://github.com/yicheng47/runner/issues/1479)
- Branch: `feat/1479-antigravity-version-gate-updates`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1479-antigravity-version-gate-updates` (base `origin/main` `678da3ec`, own `target/`, never share `CARGO_TARGET_DIR`)
- Isolation: keep root checkout on `main`; never touch sibling worktrees (`.worktrees/fix-1413-notification-click-remote-window` with active `neon trio` mission `#1413`, and `.worktrees/fix-818-terminal-round-trips`).
- Fold this brief into an implementation commit before final push.

## Problem and outcome

Antigravity CLI (`agy`) is the only OSC 7501 runtime without a minimum-version lock (`min_version: None`) or in-Runner update support (`update_args: &[]`, `npm_package: None`). Other OSC 7501 runtimes gate old builds (Codex `0.164.0`, Claude Code `2.1.295`, pi `1.1.0`, Grok `1.0.89`, OpenCode v2 `2.1.0`), whereas Antigravity needs `1.5.8+` for OSC 7501 and Unix usage probes yet launches old installs like `1.4.7` silently. Because `#1425` disabled `agy`'s background auto-updater on Windows (`AGY_CLI_DISABLE_AUTO_UPDATE=true`, `#1422`), Windows users running `agy` through Runner get neither self-updates nor an Update button and stay stuck on outdated builds.

Outcome: gate Antigravity CLI on `1.5.8+` at launch/recovery, wire `agy update`, and check for newer `agy` versions via the official Antigravity release manifest without enabling `npm install -g`.

## Read first

- Issue `#1479`
- `crates/runner-core/src/protocol/runtime_metadata.rs` (`antigravity::catalog`, `withhold_below_version`, `runtime_updatable_for_version`)
- `crates/runner-daemon/src/runtime_status/versions.rs` (`request_latest_with`, `fetch_latest`, `parse_latest`, `updatable`, `versions_catalog_golden`)
- `crates/runner-daemon/src/runtimes/mod.rs` and `crates/runner-daemon/src/runtimes/antigravity/mod.rs`
- `crates/runner-daemon/src/ops/runtime.rs` (`runtime_install_spawn_spec`, `runtime_update_spawn_spec`, `catalog_golden`) and `crates/runner-daemon/src/session/manager/spawn_spec.rs` (`update_spawn_spec`, `min_version` gate)
- `crates/runner-app/src/surfaces/settings/agents/{presentation,render}.rs` and `crates/runner-app/src/surfaces/start_chat/launcher.rs`
- `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md`, `docs/arch/arch.md`

## Deliverables

1. **Minimum version gate & `agy update` (`runner-core`, `runner-daemon`).** Set `min_version: Some("1.5.8")` and `update_args: &["update"]` in `antigravity::catalog()`, keeping `npm_package: None`. Verify `RuntimeVersionGate` blocks chat/mission/resume/fork launches when `agy < 1.5.8` (still allowing unknown versions), surfaces the launch-pad warning and `Update needed` badge, enables Settings → Agents **Update** and session recovery, and runs `<effective agy> update` via `update_spawn_spec` without `AGY_CLI_DISABLE_AUTO_UPDATE=true`. Keep `start_install` rejecting Antigravity (`npm_package: None`).
2. **Non-npm latest-version discovery (`runner-daemon`).** Generalize `runtime_status/versions.rs` so latest checks support either an npm `(package, tag)` or an HTTPS JSON manifest URL. For `Runtime::Antigravity`, query `https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/<platform>.json` (same endpoint as `install.sh` / `install.ps1`), mapping the target to `darwin_arm64`, `darwin_amd64`, `windows_amd64`, `windows_arm64`, `linux_amd64`, `linux_arm64` (and `linux_<arch>_musl` on musl Linux). Parse `"version"` through `parse_latest`, reusing `crate::usage::http_client(env)`, the 6-hour cache, forced Refresh, generation superseding, and silent failure degradation. Update `versions::updatable` so `Runtime::Antigravity` is `true` while `Grok`, `OpenCode`, `Trae`, and `Cursor` stay unchanged.
3. **Tests & goldens.** Update `catalog.json` and `catalog-versions.json` goldens and add tests for: Antigravity manifest platform URL selection, JSON parsing and failure degradation, `versions::updatable(Runtime::Antigravity)`, `request_latest_with` fetching Antigravity alongside npm runtimes, `1.5.8` launch/resume gating, Settings **Update** & launcher optional/required states, and `start_install(Antigravity)` still rejecting npm install. Stress-loop any new async/background test with `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'` and state the count.
4. **Docs.** Update `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md`, and `docs/arch/arch.md` for Antigravity's `1.5.8+` minimum version, Settings/launcher updates via `agy update`, manifest-backed latest checks, and Windows updates.

Non-goals: `npm install -g` for Antigravity; non-npm latest checks for Grok or OpenCode v2; adding an Update button to `LAUNCHER_VERSION_NOTICE` (`#1469`); changing Windows `AGY_CLI_DISABLE_AUTO_UPDATE=true` on sessions or probes.

## Validation and handoffs

- **@coder:** Run `cargo test -p runner-core`, `cargo test -p runner-daemon`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, and `git diff --check`. Hand `@reviewer` an unpushed commit SHA with summary, files, and checks.
- **@reviewer:** Verify `npm_package` stays `None`, manifest platform mapping covers macOS/Windows/Linux, `update_spawn_spec` omits `AGY_CLI_DISABLE_AUTO_UPDATE`, cache/superseding/failure semantics match npm checks, goldens and bilingual docs are complete, and tests cannot race. Name selected live checks and reviewed SHA in the clean review.
- **@qa:** Acquire `script/qa-lock acquire --timeout 540` before launching the macOS dev app (wait cleanly if `#1413` holds it; never touch pre-existing dev user sessions). Run live checks using temporary `/tmp` executable overrides (never mutating real `agy`) and restore Auto before finishing:
  1. **Below-minimum gate (`1.4.7`):** Override Antigravity to a `/tmp` fake `agy` printing `1.4.7` (upgrading itself to `1.5.17` on `update`). Verify Settings → Agents shows `1.4.7` + **Update**, launcher shows the `1.5.8` warning and `Update needed`, starting Antigravity hits the update gate/recovery, and **Update** runs `agy update` and re-probes to `1.5.17`.
  2. **Optional update (`1.5.8` + Refresh):** Override to a `/tmp` fake `agy` printing `1.5.8`, click **Refresh** in Settings → Agents, and verify live manifest fetch shows `available_version` (e.g. `1.5.8 → 1.5.17`), **Update** button, Settings dot, and `LAUNCHER_OPTIONAL_UPDATE`.
  3. **Reset to Auto:** Reset override to Auto, refresh, and confirm real `agy` shows its actual version with no false update or gate.

  Record pass/fail, candidate SHA, screenshots, and honest platform scope (macOS live-tested; Windows manifest + `agy.EXE update` covered by unit/CI tests only) in `docs/tests/runs/1479-antigravity-version-gate-updates-mac.md`. Archive only QA sessions, clean `/tmp`, release `script/qa-lock release`, and report verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`; 3 slots). After `@reviewer` passes and `@qa` reports its verdict, `@coder` may fold this brief into implementation commit(s), push `feat/1479-antigravity-version-gate-updates`, open a PR against `main` with `Fixes #1479`, apply `risk-medium` (keep `core-review` if workflow-applied; keep `needs-live-test` while Windows live testing remains unrun and note the gap in the PR), confirm `@reviewer` adds `reviewed` on the head SHA, and drive CI green. Do not merge.
