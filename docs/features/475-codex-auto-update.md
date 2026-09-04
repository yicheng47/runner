# Codex updates — Runner owns them, the TUI never asks

Tracking issue: [#475](https://github.com/yicheng47/runner/issues/475). Status: planned. Priority P2.

## Motivation

When a newer `@openai/codex` is published, the codex TUI opens on an **Update available!** cell — "Update now (runs `npm install -g @openai/codex@latest`)" / "Skip until next version" — and waits. Inside Runner that is a fresh chat or a mission slot sitting idle until a human clicks into the pane and answers. Accepting runs the install, codex exits, and Runner shows the **Chat paused** card, so the human comes back a second time to press **Resume**. In a mission the slot is simply stalled until someone looks. Codex ships often enough that this is a weekly interruption, and it is exactly the chore Runner exists to absorb.

The issue sketched two shapes: read the prompt off the terminal grid and answer it, or find a knob that stops the prompt and let Runner do the updating. The knob exists — codex 0.153.0 has a top-level `check_for_update_on_startup` config key, settable per invocation with `-c check_for_update_on_startup=false`, the same override path Runner already uses for `model_reasoning_effort` (`crates/runner-backend/src/router/runtime.rs:150`). This spec takes the second shape. Nothing is scraped, no keystrokes are injected, and codex never exits for an update under Runner, so the exit classifier, the paused card, and local-input suppression stay exactly as they are.

## Behavior

### The prompt never appears

- Every codex invocation Runner builds — fresh chat, `codex resume <key>`, mission slot, headless fork — carries `-c check_for_update_on_startup=false` while **Update codex automatically** is on. It is emitted next to the effort override in `model_effort_args`' `"codex"` arm (`runtime.rs:140`), so the resume subcommand prefix (`resume_plan`, `:642`) and the trailing-flag order are unchanged.
- With the setting off, the flag is not emitted and codex behaves as it does today: the prompt appears, the human answers it, and an accepted update ends the session on the paused card. Opting out means "I decide at the prompt", so Runner does not second-guess it.
- Sessions spawned before the setting is flipped keep whatever argv they started with; the flag is per spawn.

### Runner knows the versions

- Runtime discovery (`crates/runner-backend/src/runtime_status.rs`, `start_background_discovery` `:370`) learns each runtime's installed version by running `<executable> --version` through the login-shell PATH (`shell_path.rs`) and parsing the last dotted-number token — codex prints `codex-cli 0.153.0`. The version rides on `RuntimeExecutableStatus` and shows under the executable path in Settings → Agents (`crates/runner-app/src/surfaces/settings/agents.rs`), mono, `text-mid`, for every runtime that reports one.
- For codex only, discovery also asks the registry for the latest version — `npm view @openai/codex version` — in the same background task, on launch, on **Refresh**, and every six hours while the app runs. The result is kept in memory on the runtime status, never in settings. Offline or a slow registry means "latest unknown" and no row state change; discovery must not block on it (a 10 s cap, then give up until the next tick).
- **Update available** is `latest > installed` by semver, prerelease tags compared lexically after the numeric parts.

### Runner performs the update

- Settings → Agents → codex row gains **Update codex automatically** (a switch beside **Enabled**; default on; persisted as `settings.codex_auto_update: bool` with `#[serde(default = "true")]` so existing `ui-settings.json` files opt in). Under it, the version line reads one of: `0.153.0 · up to date`, `0.153.0 → 0.154.0 · updating…`, `0.153.0 → 0.154.0 · Update now` (a small button, shown when the switch is off), `0.153.0 → 0.154.0 · update failed — <first line of stderr>` with **Retry**, or `0.153.0 · managed outside npm`.
- When the switch is on and an update is available, Runner runs `npm install -g @openai/codex@latest` through the login shell in the background (`ops::runtime::update_codex`), one run at a time, never while a codex spawn is in flight (spawns hold the manager lock; the updater waits for it). Running sessions are unaffected: npm unlinks and rewrites the binary, the live process keeps its mapped inode, and only new spawns pick up the new version. On success discovery re-runs so the row shows the new number; there is no toast. On failure the row shows the error and Runner does not retry on its own until the next discovery tick reports a newer latest or the human presses **Retry**.
- **Update now** with the switch off runs the same op once.
- If the resolved codex executable is not under the npm global prefix (`npm root -g`'s parent `bin`), Runner shows `managed outside npm` and never runs the installer — a Homebrew or manually placed codex, or a Settings → Agents executable override pointing somewhere else. The `-c` flag is still passed, so those sessions never stall either; the row's `→ x.y.z` is how the human learns an update exists.
- claude-code and every other runtime are untouched: no version-latest lookup, no updater, no flag. Claude Code updates itself out of band.

## Non-goals

- Reading the update prompt off the terminal grid and answering it. The knob makes the detector, the fixture capture, and the Runner-input classification unnecessary; if a future codex drops `check_for_update_on_startup`, that is the fallback and gets its own spec.
- Restarting codex after an update accepted at the prompt (only reachable with the switch off, and then the paused card is the honest state).
- Updating codex installed by Homebrew, a pinned `npm i -g @openai/codex@<version>`, or an executable override outside npm's prefix.
- An update channel, pinning, or rollback. `@latest` is the only target.
- Auto-updating claude-code, trae, or any other runtime.

## Decisions

- **Configuration over scraping.** A `-c` override is one line in an arm Runner already owns, is stable across TUI redesigns, and produces no exit to interpret. Grid detection would have needed a recorded fixture, a matcher, synthetic keystrokes flagged as Runner input, an exit-status special case that must not clear the session key, and a paused-card exemption — five places to get wrong for the same outcome.
- **Runner updates, not "skip forever".** Suppressing the check alone would silently pin every user to whatever they installed. The updater is what earns the right to hide the prompt; the version line is what keeps it honest when the updater can't run.
- **Default on, per-user opt-out.** The issue asks for it and it matches Runner's own resume-on-launch stance: chores are absorbed unless the user says otherwise. The switch lives with the runtime it governs, not in General.
- **Never during a spawn, never restarting anything.** Replacing the binary is safe for running processes on macOS; the only race is a spawn reading a half-written file, so the updater serialises on the manager lock and does nothing else.
- **`npm view` is the source of truth for "latest".** It is what the TUI's own prompt installs from, it needs no GitHub token, and the login-shell PATH already resolves npm for the executable discovery. The GitHub releases page is a link in the row, not a data source.

## Design

Add to `design/runner.pen`, in the Settings band beside the existing Agents page and the `Spec — Agent runtime row states` sheet: `Spec — Codex update row (475) · v1` — the codex row in its six states (up to date · updating · Update now with the switch off · failed with Retry · managed outside npm · latest unknown), the **Update codex automatically** switch placement beside **Enabled**, and the version line treatment (mono, `text-mid`) applied to every runtime row.

## Implementation phases

1. **The flag.** `settings.codex_auto_update` with its serde default; the `-c check_for_update_on_startup=false` emission in `model_effort_args`' codex arm gated on it (the setting reaches `spawn_args` the way `claude_settings_args` inputs do); the switch in the codex row. Ships value on its own: no session stalls from this point.
2. **Versions.** `--version` capture and parse in runtime discovery for every runtime; the codex-only `npm view` latest with the 10 s cap and the six-hour tick; the version line in every Agents row; semver compare.
3. **The updater.** `ops::runtime::update_codex` through the login shell, serialised on the manager lock; the npm-prefix check; the row states and **Update now** / **Retry**; discovery re-run on success.
4. **Docs.** Settings → Agents copy, the runtime notes in `docs/arch/`, and the archive move at ship.

Each phase leaves `cargo test -p runner-backend` and `cargo test -p runner-app` green so it can review on its own.

## Verification

- `cargo test -p runner-backend`: `spawn_args` for codex carries `-c check_for_update_on_startup=false` when the setting is on — for a fresh spawn, a `resume <key>` plan, and a headless fork — and omits it when off; claude-code and trae never carry it; `codex-cli 0.153.0` parses to `0.153.0`; `0.153.0 < 0.154.0`, `0.153.0 < 0.153.1`, `0.154.0-beta.1 < 0.154.0`; the updater refuses an executable outside the npm prefix; an in-flight spawn delays the updater rather than racing it.
- `cargo test -p runner-app`: the settings struct reads a pre-feature `ui-settings.json` with `codex_auto_update == true`; the codex row renders each of the six states from a `RuntimeExecutableStatus` fixture; **Update now** appears only with the switch off and an update available.
- `make clippy` and `make fmt`.
- Manual smoke (Jason): `npm i -g @openai/codex@<previous>`, start a codex chat — no prompt, the session goes straight to work; Settings → Agents shows `<previous> → <latest> · updating…` then `<latest> · up to date` within a minute; start another chat and `codex --version` inside it shows the new number while the first chat is still alive; switch the setting off, downgrade again, start a chat — the prompt is back and answering it ends on the paused card as today; **Update now** from the row updates it; point the executable override at a copy outside npm's prefix — the row says `managed outside npm` and the chat still opens without a prompt.
