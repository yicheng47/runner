# 1451 — Codex TUI workspace launch option

Tracking issue: [#1451](https://github.com/yicheng47/runner/issues/1451)

## Motivation

Starting in Codex CLI 0.171.0 (`openai/codex#61352`), `features.tui_workspace` is enabled by default. It activates Codex's built-in TUI sidebar file renderer (searchable Markdown, syntax-highlighted source, and image previews from `#61888`/`#61889`/`#61890`/`#61919`, plus the model-callable `open_in_codex` MCP tool from `#63184`) and hides local file-link destinations in rendered Markdown (`hide_reader_link_destination`) without emitting `file://` OSC 8 hyperlinks.

Because the file path is hidden from the terminal grid and no `file://` OSC 8 URI is emitted, Runner's terminal link detector (`runner-terminal`'s `link_at`) cannot recognize Codex Markdown file links. Even ⌘-click falls through to Codex's mouse reporting and opens Codex's cramped TUI sidebar instead of Runner's own Files side-panel preview.

Codex does not expose a narrower switch for file previews alone: all sidebar file/image previews, `open_in_codex`, side chats, and subagent sidebar views are gated by `features.tui_workspace`. Passing `-c features.tui_workspace=false` disables it on Codex 0.168.0+ and is silently ignored on older supported Codex versions (0.164.0–0.167.x), whereas `--disable tui_workspace` exits with an unknown-feature error on older builds.

## Scope

1. **Protocol and persistence (`crates/runner-core/src/protocol/runtime.rs`).**
   - Add `pub tui_workspace: bool` to `RuntimeLaunchOptions` with `#[serde(default)]` (default `false`, i.e., **Off by default**). Existing saved `_app_state.runtime_launch_options` JSON entries without `tui_workspace` deserialize as `false` so existing users get Runner's Files preview working out of the box on upgrade without a database migration.
   - Add `RuntimeLaunchOption::TuiWorkspace` supported only for `Runtime::Codex`, wired into `RuntimeLaunchOptions::get` and `RuntimeLaunchOptions::set`.
   - Add `RuntimeCommandNote::TuiWorkspaceOff` for the Settings command preview annotation.
2. **Daemon launch args, preview, and custom-flags guard (`crates/runner-daemon/`).**
   - In interactive Codex launches (both chats and missions), emit `["-c", "features.tui_workspace=false"]` when `options.tui_workspace` is `false`, without modifying `~/.codex/config.toml`.
   - Do not add `-c features.tui_workspace=false` to headless `codex exec fork` (`crates/runner-daemon/src/runtimes/codex/fork.rs`), which does not start the TUI.
   - Do not pollute default child process `env_keys` (either thread the flag cleanly without adding an env var on default spawns, or only set an env override when `options.tui_workspace` is `true` — matching how `STATUS_HOOKS_ENV` / `REMOTE_PLUGIN_ENV` avoid adding child env keys for default launches).
   - Annotate `-c features.tui_workspace=false` in `command_lines` (`crates/runner-daemon/src/session/manager/spawn_spec.rs`) with `RuntimeCommandNote::TuiWorkspaceOff` and `option: Some(RuntimeLaunchOption::TuiWorkspace)`.
   - Include `tui_workspace` in `reserved_flags` (`crates/runner-daemon/src/runtimes/custom_flags.rs`) so Custom flags rejects conflicting `-c features.tui_workspace=...` / `--config features.tui_workspace=...` / `-c 'features={tui_workspace=...}'` overrides with the existing reserved-flag refusal.
3. **Settings UI and localization (`crates/runner-app/`).**
   - In **Settings → Agents → Codex → Launch** (`crates/runner-app/src/surfaces/settings/agents/launch.rs`), render the `RuntimeLaunchOption::TuiWorkspace` switch row using the existing Launch card switch layout (between **Remote plugin catalog** and **Custom flags**, or directly beside **Remote plugin catalog**).
   - Badge: `Chats and missions` (`ui.launch_chats_and_missions`).
   - Flag hint line: `Off: -c features.tui_workspace=false`.
   - English copy (`crates/runner-app/i18n/en.toml`):
     - Title (`ui.launch_tui_workspace`): `TUI workspace`
     - Description (`ui.launch_tui_workspace_description`): `Codex's built-in sidebar renders file previews inside the terminal and hides Markdown file paths from Runner's Files panel. Off by default so ⌘-click opens files in Runner; turn on for Codex's side panes and file viewer.` (Keep platform-neutral or phrase without macOS-only `⌘`, e.g., `…so clicking file links opens them in Runner; turn on for Codex's built-in file previews and side panes.`)
     - Preview note (`ui.launch_tui_workspace_off`): `TUI workspace off`
   - Simplified Chinese copy (`crates/runner-app/i18n/zh-hans.toml`): matching translations for `ui.launch_tui_workspace`, `ui.launch_tui_workspace_description`, and `ui.launch_tui_workspace_off`.
4. **Documentation.**
   - Update `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md` to document the **TUI workspace** launch toggle for Codex (off by default, adds `-c features.tui_workspace=false` without touching Codex's config file).
   - Update `docs/arch/runtime-integration.md` (Per-runtime launch options, English and Chinese paragraphs) with the new default and behavior.

### Non-goals

- Do not modify `~/.codex/config.toml` or pass `--disable tui_workspace`.
- Do not add a `TuiWorkspace` toggle to any runtime other than Codex.
- Do not change terminal file-link parsing in `runner-terminal` (tracked separately in [#1452](https://github.com/yicheng47/runner/issues/1452)).
- Do not add new CLI subcommands.

## Implementation Phases

1. **Core protocol & persistence.** Add `tui_workspace` (default `false`), `RuntimeLaunchOption::TuiWorkspace`, `RuntimeCommandNote::TuiWorkspaceOff`, and protocol serialization/backward-compatibility tests in `crates/runner-core`.
2. **Daemon launch, preview & validation.** Emit `-c features.tui_workspace=false` on interactive Codex launches when off, wire command preview annotations and `custom_flags` reserved-flag checks, and update golden spawn expectations (`spawns.json`, `forks.json` if affected, `launch_gate.rs`, `resume.rs`, `permissions.rs`, `spawn_spec.rs`, `custom_flags.rs`).
3. **Settings UI, i18n & docs.** Add the switch row, preview note, bilingual strings, Settings UI tests in `crates/runner-app/src/surfaces/settings/agents/tests.rs`, and user/architecture docs.

## Verification

### Automated

- `cargo test -p runner-core` (protocol roundtrip, legacy JSON without `tui_workspace` defaults to `false`, `supported()` returns `true` only for `Runtime::Codex`).
- `cargo test -p runner-daemon` (interactive fresh/resume/mission Codex spawns include `-c features.tui_workspace=false` when off and omit it when on; headless `codex exec fork` unchanged; command preview includes the line and `TuiWorkspaceOff` note when off and removes it when on; custom flags reject `-c features.tui_workspace=true` and `--config features.tui_workspace=false`; no extra env keys on default spawns).
- `cargo test -p runner-app` (Codex Launch card renders `AGENT_LAUNCH_SWITCH_codex_TuiWorkspace`, other runtimes do not, toggle updates preview, English and Simplified Chinese strings and preview note match).
- `make clippy`, `make fmt` / `cargo fmt --all -- --check`, `git diff --check`.
- Any new test waiting on background work runs under `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'`.

### Live checks (macOS development app)

Selected because this change adds a visible Settings → Agents → Codex control, changes Codex's live command preview and spawned process arguments, and changes live Codex file-link behavior:

1. **CTW-01 — Settings switch, preview & persistence:** Open **Settings → Agents → Codex → Launch**. Confirm **TUI workspace** starts **Off**, both **Chat** and **Mission** command previews show `-c features.tui_workspace=false` with the accent **TUI workspace off** note, toggling it **On** removes the line from both previews, reopening Settings preserves the state, and toggling back **Off** restores the line.
2. **CTW-02 — Live Codex argv & file-link behavior:** With **TUI workspace** **Off**, start a fresh bounded Codex chat in `/tmp` (inexpensive model, low effort, tiny prompt) and verify its live process argv contains `-c features.tui_workspace=false` (and does not modify `~/.codex/config.toml`). Confirm a Markdown file link printed by Codex displays its path rather than opening Codex's built-in TUI sidebar viewer. Archive the test chat and clean up `/tmp`.
3. **CTW-03 — Custom flags reserved-key guard:** In **Settings → Agents → Codex → Launch → Custom flags**, enter `-c features.tui_workspace=true` and verify the red reserved-flag refusal appears, nothing is saved, and the command preview remains unchanged (regression anchor: `SET-LAUNCH-03`).

Record live QA results with screenshots in `docs/tests/runs/1451-codex-tui-workspace-setting-mac.md`.
