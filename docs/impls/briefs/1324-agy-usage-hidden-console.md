# #1324 — Keep the Antigravity usage probe hidden on Windows

Jason asked on 2026-11-14 for the agy trio to fix [#1324](https://github.com/yicheng47/runner/issues/1324). Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1324-agy-usage-hidden-console` on `fix/1324-agy-usage-hidden-console`, based on `origin/main` at `74140168`.

## Problem

On Windows, runnerd's background Antigravity usage check intermittently flashes a Windows Terminal window for 2–3 seconds. The reporter's trace shows `runnerd.exe` → `agy -p /usage --output-format json` → a Nodist `node.exe` shim → `node.exe` running a configured stdio Puppeteer MCP server, with `OpenConsole.exe -Embedding` and `WindowsTerminal.exe -Embedding` starting at the same moment. Disabling Antigravity stops it. The fix must keep Antigravity enabled and make scheduled and manual quota checks fully invisible, including every descendant.

## Likely cause

`prepare_isolated_fork` in `crates/runner-daemon/src/runtimes/antigravity/usage.rs` spawns all three probe steps (`--version`, the bare-`agy` auth check and `-p /usage`) with `DETACHED_PROCESS`. A detached process has no console, so when agy starts a console-subsystem child with default flags (its MCP server via the Nodist shim), Windows allocates a new console for that child, and Windows Terminal's default-terminal delegation shows it as a window until the child exits. Every other runnerd probe uses `prepare_headless_fork` (`CREATE_NO_WINDOW`), whose hidden console every descendant inherits, which is why only Antigravity flashes. Confirm this before building on it.

`DETACHED_PROCESS` was chosen in #1286 for a reason that still binds: with no console, `CONIN$` cannot open, so agy's print mode cannot treat itself as interactive and start browser OAuth. Read the module doc and the Windows helper test (`CONIN$ must not open under DETACHED_PROCESS`) first. Simply swapping to `CREATE_NO_WINDOW` gives agy a hidden but openable console and weakens that guarantee, so a fix must keep both properties: no visible window anywhere in the probe tree, and no path to interactive or browser login.

## Leads to evaluate

These are leads, not decisions. Pick the simplest one that provably holds both properties for `agy` 1.5.8 and later, and say why in the handoff.

- Give the probe tree a hidden console (`CREATE_NO_WINDOW`) and force non-interactive behaviour some other way. agy 1.5.9's binary contains `AGY_CLI_NONINTERACTIVE_HEADLESS` (and `AGY_CLI_INTERACTIVE_HEADLESS`); find out what it does, from which version, and whether it stops print mode from treating an openable console as interactive input.
- Keep `DETACHED_PROCESS` and stop the probe from starting MCP servers or other tools, for example a quota-only path or `--settings` for this session only. This alone does not cover descendants other than MCP servers, and `--settings` may not exist in 1.5.8.
- Anything else you find, such as creating the hidden console Runner-side, provided it keeps the guarantees above and is not heavier than the problem.

Never trigger a real browser OAuth or sign a real account out while investigating. Use an isolated `HOME`/fixture, as `SET-USAGE-05` does, and detect a would-be browser launch rather than letting one happen.

## Deliverables

- The Windows probe spawns no visible console or Windows Terminal window for any descendant, for scheduled and manual refreshes, while usage values and the signed-out, unsupported-version and timeout outcomes are unchanged.
- The zero-login guarantee from #1286 holds on every platform.
- A Windows unit test that would have caught this: a probe-spawned helper starts a console grandchild with default flags, and the test asserts that the grandchild has no visible console window, alongside whatever replaces or keeps the existing `CONIN$` assertion. It runs in Windows CI.
- The module doc comment in `usage.rs` matches the new design. No guide page describes these flags, so no guide change is expected; say so if you find one.
- Interactive Antigravity chats and other runtimes' probes are unchanged. Keep the change in the Antigravity usage code unless the shared helpers genuinely need it; touching `session/process/` makes the PR `core-review`.

## Validation

`coder` runs the scoped `runner-daemon` tests and workspace Clippy locally, then drives macOS and Windows CI green; Windows CI is the only place the Windows test executes. `reviewer` decides from the diff whether the macOS checks below are needed.

`qa` runs on this macOS host. Selected live checks:

- **MAC-1324-01**, only if the diff changes non-`cfg(windows)` behaviour (shared args, env vars or helpers): real Antigravity usage still loads in Settings → Usage and the footer after a manual refresh, because a shared change could break the probe on macOS.
- **MAC-1324-02**, under the same condition: `SET-USAGE-05` with an isolated signed-out fixture, because the zero-login guarantee is what the change risks.
- **WIN-1324-01**: on Windows with Windows Terminal as the default terminal and agy ≥ 1.5.8 signed in with a stdio MCP server launched through a shim, scheduled and manual usage refreshes show no window and still return values. This host cannot run it: record it as Blocked, left to Jason, with exact reproduction steps.
- **WIN-1324-02**: `SET-USAGE-05` on Windows. Also Blocked here and left to Jason.

Record the run in `docs/tests/runs/1324-agy-usage-hidden-console-both.md`. With the Windows checks outstanding, the PR carries `needs-live-test` and lists them in its description.

## Authorization

Jason approved the agy trio for this mission. The crew may commit, push this branch and open one PR against `main` with `risk-medium` and `needs-live-test`.

## Design decision: `direct-quota-api` (Jason, 2026-11-15)

After investigating Windows console behavior with `DETACHED_PROCESS` and `CREATE_NO_WINDOW`, the approved direction for [#1324](https://github.com/yicheng47/runner/issues/1324) is **`direct-quota-api`**:

- Keep macOS and Linux (`#[cfg(not(windows))]`) completely unchanged with the existing three-step `agy` subprocess probe (`--version` >= `1.5.8`, `AGY_CLI_CDE_AUTH_ACTION=check`, and `-p /usage --output-format json`).
- Replace the Windows probe (`#[cfg(windows)]`) with direct HTTPS quota fetching so Runner never spawns `agy` or any child process for Windows quota checks: read `agy`'s saved sign-in strictly read-only from Windows Credential Manager (`CredReadW` on generic target `"gemini:antigravity"`) or `.gemini` / `.gemini/antigravity-cli` fallback token files, refresh expired tokens strictly in memory against `oauth2.googleapis.com/token` without writing back or logging credentials, and query Cloud Code `v1internal:loadCodeAssist` + `v1internal:retrieveUserQuotaSummary`.
- Missing or rejected credentials map to `UnavailableReason::SignIn`; network errors, timeouts, `429`, `5xx`, and unparseable responses map to `UnavailableReason::AntigravityNoAnswer`.
- Replace the Windows `CONIN$` / `DETACHED_PROCESS` test with zero-spawn and isolated Credential Manager tests; label the PR `risk-high` and `needs-live-test` (since Windows live execution cannot run on the macOS host).
