# 37 — Agent runtime executable settings

> Tracking issue: [#279](https://github.com/yicheng47/runner/issues/279)
> Priority: P1.

## Motivation

Runner launches Claude Code and Codex by their catalog command names (`claude` and `codex`) and expects the child PTY to resolve them through a `$PATH` captured from the user's login shell. This works for standard installations, but some packaged-app users receive "command not found" even though the CLI works in their terminal. The current startup probe runs `$SHELL -ilc` once, assumes common shell flag and startup-file behavior, and discards the result after a two-second timeout. Slow zsh/Oh My Zsh configurations and CLIs installed through nvm, fnm, mise, asdf, pnpm, Bun, or Volta can therefore lose the only path entry that exposes the executable.

Runtime-only direct chats have no configurable command, and the create/edit runner UI binds the command field to the fixed catalog default. Users need automatic discovery that explains what Runner found, plus an explicit per-runtime executable override when their shell environment cannot be reproduced reliably.

## Scope

### In scope

- **Agent runtimes settings pane.** Add an Agent runtimes pane under Settings → Integrations. Show one row/card for each first-class runtime returned by the backend registry, initially Claude Code and Codex.
- **Detected executable.** Resolve each runtime's catalog command against Runner's composed user `$PATH` and show the resulting absolute executable path, or a clear Not found / Detection failed state.
- **Executable override.** Let the user enter or pick an absolute executable path per runtime. An empty override means automatic discovery. Validate that a non-empty path exists, is a regular file, and is executable before saving.
- **Resolution precedence.** Use the explicit runtime override first, then the automatically detected executable, then the catalog command only when it can be resolved through the effective child `$PATH`. Do not silently report a configured runtime as available when none of those paths resolves.
- **Spawn coverage.** Apply the effective runtime executable to runtime-only direct chats and runtime-backed runner/mission spawns whose stored command is still the catalog default. Preserve legacy runners with a genuinely custom non-default command.
- **Session metadata.** Persist the effective command used for a runtime-only session so resume uses the same executable unless the file no longer exists. Runner-backed session behavior should remain compatible with existing `runner.command` rows.
- **Refresh.** Allow users to rerun discovery after installing or upgrading an agent CLI without restarting Runner.
- **Shell-aware discovery.** Keep reading the user's login-shell environment, but make the shell choice and probe outcome explicit. Use the configured login shell when available, support the startup semantics of the Unix shells Runner supports, and avoid claiming that one invocation shape loads the same startup files for every shell.
- **Slow shell initialization.** Replace the current all-or-nothing two-second startup probe with a non-blocking or otherwise startup-safe discovery flow that accommodates realistic zsh/Oh My Zsh initialization. Preserve the last known good result on timeout and expose the timeout as a diagnosable state instead of silently dropping to launchd's stripped environment.
- **Diagnostics.** Log the selected shell, discovery duration, success/failure reason, and resolved runtime executable paths without logging unrelated environment values. Surface enough status in Settings for a user to distinguish Not installed from Shell probe timed out.
- **Backend persistence.** Store overrides in backend-owned app settings so all windows and all Rust spawn paths share the same value; do not make localStorage the source of truth for executable selection.
- **Design first.** Add the settings pane and its detected/override/error/refresh states to `design/runners-design.pen` before implementation.

### Out of scope

- Installing, upgrading, or authenticating Claude Code or Codex.
- Accepting aliases or shell functions as runtime executables; Runner spawns a real process and requires an executable file.
- General-purpose editing of the child process `$PATH`.
- Adding new agent runtimes; OpenCode remains tracked separately by spec 30.
- Reworking model, effort, permission-mode, prompt, or resume adapters.
- Windows support beyond retaining a clear unsupported state for the current Unix-only PTY runtime.

## Implementation Phases

### Phase 1 — UX design and settings contract

- Design Settings → Integrations → Agent runtimes in `design/runners-design.pen`, including detected, overridden, not-found, probing, timeout, validation-error, and refresh states.
- Define a backend runtime-settings shape keyed by stable runtime name with an optional executable override.
- Define the effective-command precedence and legacy `runner.command` compatibility rules in tests before changing spawn behavior.

### Phase 2 — discovery and persistence

- Refactor login-shell discovery so it reports the shell used, elapsed time, captured `$PATH` status, and structured failure reason.
- Resolve catalog commands to absolute executable paths in Rust using the same composed `$PATH` supplied to child PTYs; do not rely on aliases or functions returned by shell builtins.
- Make slow shell probing startup-safe, retain the last known good discovery result, and add an explicit refresh command.
- Add backend persistence and Tauri commands to list runtime status, set/clear an override, and refresh discovery.
- Add unit coverage for zsh/Oh My Zsh-style slow startup, Bash startup semantics, missing or invalid `SHELL`, timeout, executable permissions, stale overrides, and PATH precedence.

### Phase 3 — settings UI

- Add the Agent runtimes pane under Integrations and populate it from the backend runtime registry/status command.
- Show the detected absolute path and discovery state for Claude Code and Codex.
- Add browse, edit, clear-to-auto, and refresh actions with inline validation and useful failure copy.
- Keep the UI synchronized across windows after runtime settings or discovery status changes.

### Phase 4 — spawn integration

- Centralize effective runtime command resolution in the backend and use it for runtime-only direct chats.
- Use the effective runtime executable for runner/mission spawns when the runner still carries the catalog default command, while preserving custom legacy commands.
- Ensure new runner rows no longer freeze a stale auto-detected absolute path into `runner.command`; store the stable catalog default and resolve it at spawn time.
- Return a pre-spawn error naming the runtime and linking the user to Agent runtimes settings when no executable resolves.
- Verify session metadata and resume behavior use a valid effective executable without overwriting intentional custom runner commands.

### Phase 5 — migration, diagnostics, and documentation

- Preserve existing runner rows and runtime-only sessions without destructive migration.
- Add targeted startup and spawn diagnostics that make shell timeouts, missing executables, and active overrides distinguishable.
- Update architecture documentation for login-shell environment capture, runtime registry resolution, and command precedence.

## Verification

- [ ] Settings → Integrations → Agent runtimes shows Claude Code and Codex from the backend registry.
- [ ] A standard executable on the captured login-shell `$PATH` is displayed as an absolute detected path.
- [ ] A slow zsh/Oh My Zsh startup does not silently discard a previously valid `$PATH` after two seconds.
- [ ] Bash, zsh, and other explicitly supported shells use documented, tested startup semantics.
- [ ] Missing, invalid, or unsupported login shells produce a visible detection failure rather than a misleading Not installed state.
- [ ] A user can refresh discovery after installing Codex without restarting Runner.
- [ ] A valid absolute override is persisted and used by runtime-only direct chats.
- [ ] The override is used by runner and mission spawns whose stored command is the runtime's catalog default.
- [ ] A runner with a custom non-default command continues using that command.
- [ ] Clearing an override returns the runtime to automatic discovery.
- [ ] Nonexistent, non-file, and non-executable overrides are rejected with inline errors.
- [ ] Aliases and shell functions are not accepted as executable paths.
- [ ] A missing runtime fails before PTY spawn with actionable copy pointing to Agent runtimes settings.
- [ ] Discovery logs include shell, duration, outcome, and resolved executable without dumping the user's full environment.
- [ ] Settings and effective command behavior remain consistent across multiple app windows.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
- [ ] Relevant Rust tests pass, followed by `cargo test --workspace` when implementation is complete.
