# Brief: 1341 — Anonymous active-install heartbeat

## Starting point

- Tracking issue: `yicheng47/runner#1341` (milestone `0.20`, `needs-design` removed after sign-off).
- Branch: `feat/1341-install-heartbeat`
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1341-install-heartbeat` (based on `origin/main` `bed3f353`).
- Keep the root checkout on `main` and use this worktree's own `target/` (never set a shared `CARGO_TARGET_DIR`).
- Sibling worktrees in flight: `.worktrees/feat-804-crew-role-templates` and `.worktrees/fix-1394-remote-windows-drives` — do not touch them or assume unmerged changes. Peer repo `runner-web` is implementing `POST https://runnersh.dev/api/heartbeat` in parallel on `feat/install-heartbeat`.

## Problem and outcome

GitHub release downloads cannot measure real active installs (macOS appcast counts every poll; Windows exe counts are polluted by `winget`). Implement an anonymous daily install heartbeat to `https://runnersh.dev/api/heartbeat` with a dedicated **Settings → Privacy** pane (`S3`), a First-run Setup leading checkbox (`F1`), bilingual privacy docs, and a strict zero-UX-impact guarantee on any network failure.

Authoritative spec: `docs/features/1341-install-heartbeat.md` (already on this branch — wins on payload schema, gating, and failure behavior).

Signed-off design: `runner/desktop/specs/1341-install-heartbeat.pen` (`d483cea`). Exported 2x PNG references are at `/tmp/runner-1341-design/`:
- `YmUMc.png` — Settings → Privacy (Dark)
- `b2AFwu.png` — Settings → Privacy (Runner Light)
- `Fdc0i.png` — Settings → Privacy (`DO_NOT_TRACK=1` override)
- `A6YyP.png` — First-run Setup leading checkbox (`F1`)
- `Rhp9k.png` — All four Privacy card states & zero-UX-impact contract

## Read first

- `docs/features/1341-install-heartbeat.md`
- `crates/runner-core/src/version.rs` (`CRATE_VERSION`, `RUNNER_BUILD_STAMP`, `RUNNER_RELEASE_CHANNEL`)
- `crates/runner-daemon/src/usage.rs` (proxy-aware `reqwest` client) and `crates/runner-daemon/src/ops/` / `crates/runner-core/src/protocol/`
- `crates/runner-app/src/surfaces/settings/page/mod.rs` (`SettingsPane`, `SYSTEM_PANES`), `render.rs`, `general.rs`
- `crates/runner-app/src/surfaces/start_chat/setup.rs` (First-run Setup footer & onboarding completion)
- `crates/runner-app/i18n/en.toml` and `crates/runner-app/i18n/zh-hans.toml`

## Deliverables

1. **Daemon heartbeat service & local client protocol (`runner-core` + `runner-daemon`):**
   - Store state in `_app_state` (`telemetry_enabled`, `telemetry_install_id`, `telemetry_last_sent_at`). Default enabled (`true`) on stamped builds; generate a lowercase UUID v4 on first enabled use.
   - Respect `DO_NOT_TRACK=1` (`1` / `true`) and `RUNNER_TELEMETRY=0` as an environment override locking telemetry off and surfacing the active env var name in status.
   - Build gating: unstamped or debug builds (`option_env!("RUNNER_BUILD_STAMP").is_none()` or `cfg!(debug_assertions)`) never send pings to production and report `dev_build: true`. Support an injectable/local endpoint in unit tests and an explicit `RUNNER_HEARTBEAT_TEST_URL` (debug/test only, or only when explicitly set) so QA can exercise the unreachable-endpoint blackhole check safely without ever pinging production from dev builds.
   - Compose exact payload per `docs/features/1341-install-heartbeat.md`: top-level `install_id`, `version` (`CARGO_PKG_VERSION`), `build_stamp` (`RUNNER_BUILD_STAMP`), `channel` (`RUNNER_RELEASE_CHANNEL`: `production` | `nightly` | `preview-nightly`), `os` (`macos` | `windows` | `linux`), `arch` (`aarch64` | `x86_64`), `remote_enabled` (`bool`), and `meta: { active_runtimes, used_mission_7d }` computed from local SQLite over the last 7 days (allowlisted runtime keys only; zero prompts/paths/accounts).
   - **Zero-UX-impact contract:** send on a detached background worker off the UI thread using `runner-daemon`'s proxy-aware `reqwest` client with a hard 5-second total timeout, completely separate from update checks. Drop all DNS/TLS/proxy/timeout/HTTP errors silently (`debug!` log only; zero toasts, banners, retries, or UI stalls). On `2xx`, record `telemetry_last_sent_at` and throttle for 24 hours. Defer the first ping on fresh installs until onboarding completes or is skipped so unchecking Setup prevents any initial ping.
   - Local daemon RPCs: `telemetry_status` (returns enabled, env override, dev_build, install_id, pretty-printed live payload JSON preview, endpoint URL), `telemetry_set_enabled(bool)` (toggling off immediately clears `telemetry_install_id`), and `telemetry_reset_id` (rotates UUID v4 when enabled).
2. **Desktop UI (`runner-app`):**
   - Add `SettingsPane::Privacy` (Lucide `shield.svg`, adding the SVG asset if not yet bundled) first in `SYSTEM_PANES` (`Privacy`, `Updates`, `About`), searchable in Settings search.
   - Implement **Settings → Privacy** (`S3`) matching `/tmp/runner-1341-design/YmUMc.png`, `b2AFwu.png`, `Fdc0i.png`, and `Rhp9k.png`: **Telemetry** card (toggle, disclosure, `Privacy guide ↗` link, `Honors DO_NOT_TRACK=1` / amber override / dev-build notice, divider, live JSON payload preview box, divider, `Install ID` row with **Reset ID** button) and **What stays on your machine** card (2 rows).
   - Implement the First-run Setup leading 16×16 checked checkbox strip (`F1`, `/tmp/runner-1341-design/A6YyP.png`) above the footer, synced with `telemetry_enabled` before Setup finishes.
   - Full EN and `zh-hans` i18n entries; preserve dark and Runner Light contrast.
3. **Bilingual guide pages:**
   - Add `docs/guide/guides/privacy.md` and `docs/guide/zh-CN/guides/privacy.md`, indexed in both `docs/guide/README.md` and `docs/guide/zh-CN/README.md`, documenting exact fields (including edge `country` without IP logging), D1 retention, `DO_NOT_TRACK=1`, Settings → Privacy, and Reset ID. Link `Privacy guide` / `What's sent` to `https://runnersh.dev/docs/guides/privacy/` (and `/zh-CN/docs/guides/privacy/` for `zh-hans`).
4. **Non-goals:** No changes to companion/remote protocol, no third-party analytics SDK, no per-click tracking, no `runner-web` edits in this repo.

## Validation and handoffs

- `coder` runs `cargo test -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `python3 script/check-guide-translations.py`, and a 20-run nextest stress loop (`cargo nextest run --stress-count 20 ...`) for any new background/async tests, then hands a frozen unpushed local commit SHA to `reviewer`.
- `reviewer` reviews the full diff and names required live checks under `AGENTS.md` Test Scope.
- `qa` takes `script/qa-lock`, runs the macOS dev-app checks in `docs/features/1341-install-heartbeat.md` (Settings → Privacy dark/light + toggle + Reset ID + search + `DO_NOT_TRACK=1` + dev-build notice, First-run Setup checkbox sync, and zero-UX-impact blackhole test), and records evidence in `docs/tests/runs/1341-install-heartbeat-mac.md`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). Authorized to commit on `feat/1341-install-heartbeat`, fold this brief into implementation commits before pushing, push the branch, open a PR against `main` (`Fixes #1341`, appropriate risk label, `reviewed` when clean, `needs-live-test` only for any unverified platform), and drive CI green. Do not merge.
