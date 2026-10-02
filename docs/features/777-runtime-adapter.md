# 777 — Runtime adapter trait: one module per agent runtime

> Tracking issue: [#777](https://github.com/yicheng47/runner/issues/777)
> Priority: P1, 0.12. Platforms: macOS and Windows; a refactor with no user-visible change.
> Status: draft, 2026-10-01; decisions settled the same day. Lands as three PRs (see Decisions).

## Motivation

Adding an agent runtime touches most of the codebase. Antigravity changed 57 Rust files in [#644](https://github.com/yicheng47/runner/issues/644) and 27 more in the [#747](https://github.com/yicheng47/runner/issues/747) follow-ups. Today there are 803 references to the six agent variants (`Runtime::Codex`, `ClaudeCode`, `Antigravity`, `Pi`, `Copilot`, `Trae`) across 36 source files: 288 in `router/runtime.rs` and 96 in `session/manager/spawn.rs`. Most are free functions that take `Option<Runtime>` and `match` on it, so one runtime's behavior is spread over dozens of functions and every new runtime adds an arm to each.

Some places hard-code the set of runtimes outright, and a new runtime has to be threaded through each of them by hand:

- `HookStatusWatcher` in `session/pty_runtime.rs`, an enum with one variant per hook-capable runtime, started by probing five env-var pairs in turn.
- `UsageSnapshot` in `usage.rs`, with `claude`, `codex` and `antigravity` fields and matching `_error` fields.
- `McpClientId` in `ops/mcp.rs`, a second runtime enum with four exhaustive matches.
- `DISCOVERY_RUNTIMES` in `runtime_status/models.rs`, with a `match` that ends in `unreachable!()`.
- Two runtime tables that repeat display name, command and fork support: `RUNTIME_DEFINITIONS` in `router/runtime.rs` and the catalog in `ops/runtime.rs`.
- Copies outside the backend: the CLI's `runtime_command` in `runner-cli/src/command.rs`, `RUNNER_SKILL_ROOTS` in `runner-core`, and fixed lists such as `[ClaudeCode, Codex, Antigravity]` in `app_shell.rs`.

[#723](https://github.com/yicheng47/runner/issues/723) (Grok Build, Cursor Agent) and [#764](https://github.com/yicheng47/runner/issues/764) (pi MCP management) both add to these surfaces, so the adapter lands first. The [runtime integration checklist](../arch/runtime-integration.md) stays the contract for what a runtime must do; this spec changes only where that code lives.

## Proposal

Each agent runtime becomes one module that implements a `RuntimeAdapter` trait. `runtimes::adapter()` is the registry. Outside the adapters, code asks the adapter or reads the catalog instead of matching on a runtime.

```rust
// crates/runner-backend/src/runtimes/mod.rs: shape, not final signatures
pub trait RuntimeAdapter: Send + Sync {
    // required: every agent answers these
    fn catalog(&self) -> RuntimeCatalogEntry;          // merges RUNTIME_DEFINITIONS + ops/runtime.rs
    fn first_turn_argv(&self, body: &str) -> Vec<String>;
    fn resume_plan(&self, prior_key: Option<&str>) -> ResumePlan;
    fn permissions(&self) -> &'static Permissions;     // offered modes, flags, strip set, syntax

    // launch: default no-op
    fn model_effort_args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String>;
    fn prompt_channels(&self) -> PromptChannels;       // first turn only, or pi's system-prompt file
    fn launch_args(&self, ctx: &LaunchContext) -> Vec<String>;
    fn launch_env(&self) -> &'static [(&'static str, &'static str)];
    fn mission_dir_args(&self, dir: &Path) -> Vec<String>;
    fn launch_gate(&self) -> Option<Duration>;         // Claude's OAuth spacing

    // conversation identity
    fn fork_plan(&self, source_key: &str, label: &str) -> Option<ForkPlan>;
    fn conversation_exists(&self, key: &str, ctx: &ProbeContext) -> Option<bool>;
    fn missing_conversation(&self) -> MissingConversation; // fail, start fresh, or reuse key
    fn key_capture(&self) -> KeyCapture;               // shared mechanism: none, rollout scan, log tail, rekey drop

    // per-spawn hooks
    fn seed_trust(&self, cwd: &Path, env: &RoleEnv) -> io::Result<()>;
    fn status_hooks(&self) -> Option<&'static dyn StatusHooks>; // env, args, watcher, platforms

    // catalog surfaces
    fn native_defaults(&self, home: &Path) -> RuntimeDefaults;
    fn model_discovery(&self) -> Option<&'static dyn ModelDiscovery>;
    fn usage(&self) -> Option<&'static dyn UsageSource>;
    fn skills(&self) -> SkillSupport;                  // roots, toggle backend or read-only
    fn mcp(&self) -> Option<McpConfig>;                // path, format, wire name
}

// A free function: `Runtime` lives in runner-core (decision 1), and Rust
// allows inherent impls only in the defining crate.
pub fn adapter(runtime: Runtime) -> &'static dyn RuntimeAdapter {
    match runtime {
        Runtime::Codex => &codex::Codex,
        Runtime::ClaudeCode => &claude_code::ClaudeCode,
        // … one arm per runtime
        Runtime::Shell => &NoAgent,
    }
}
```

Every method except the four required ones has a default that means "not supported". A plain CLI therefore implements its catalog entry, first-turn argv, resume plan and permission table, and nothing else. A module that grows also holds that runtime's support files: `runtimes/antigravity/` takes `agy_capture.rs`, `agy_status.rs`, `agy_trust.rs` and `runtime_status/models/antigravity.rs`, and the other runtimes' files move the same way.

### Where each concern moves

| Concern | Today | What varies by runtime | Adapter member |
| --- | --- | --- | --- |
| Identity | `model.rs` `Runtime`, `RUNTIME_DEFINITIONS`, `ops/runtime.rs` catalog, CLI `runtime_command`, core `RUNNER_SKILL_ROOTS` | key, display name, command, fork, skill roots, update args, npm package, description, install URL, enabled by default, static models and efforts | core identity row and `catalog()` |
| Model and effort | `model_effort_args` | flag names, lowercasing, agy's model and effort pair table | `model_effort_args` |
| First turn and persona | `first_turn_argv`, `system_prompt_args`, `prompt::split_session_prompt`, persona resend on fresh fallback in `spawn.rs` | positional, `-i` or `--`; pi's `--append-system-prompt` file; pi and agy resend the persona | `first_turn_argv`, `prompt_channels` |
| Permissions | `permission_mode_args`, `strip_permission_flags`, `infer_permission_mode`, `mode_match_pairs`, `mission_permission_mode_args`, app `roles/logic.rs` `permission_modes` | flags per mode, strip set, GNU or Go flag syntax, Codex's mission Bypass with `danger-full-access`, Copilot's variadic `--allow-tool`, Claude's legacy `--dangerously-skip-permissions` | `permissions()`; the app reads offered modes from the catalog |
| Launch extras | `trailing_runtime_args`, `agent_env`, `mission_bus_sandbox_args`, pi's mission `--approve`, `enter_claude_launch_gate` | Codex update check and service tier, Copilot `--no-auto-update`, agy `--log-file`, env that silences surveys and version checks, `--add-dir` for the mission dir | `launch_args`, `launch_env`, `mission_dir_args`, `launch_gate` |
| Resume and fork | `resume_plan`, `fork_plan`, the `*_conversation_exists` probes, the conversation-missing arm in `spawn.rs` | assigned or captured key, argv prepended or appended, where the probe looks, what happens when history is gone | `resume_plan`, `fork_plan`, `conversation_exists`, `missing_conversation` |
| Folder trust | `seed_runtime_project_trust` | Codex, Copilot (honouring `COPILOT_HOME`) and agy each write their own file | `seed_trust` |
| Key capture | `codex_capture` (Codex and TRAE), `agy_capture`, the `claude_rekey` drop (Claude and pi) | rollout scan with a prompt marker, log tail, hook drop | `key_capture`, naming one of the shared mechanisms, which the manager runs |
| Status hooks | `hook_feed::hooks_supported`, the five `*_status` modules, env set in `spawn.rs`, args in `router/runtime.rs`, `HookStatusWatcher` | platforms, env vars, the CLI args, plugin or settings that install the hook, the watcher, the interrupt signal | `status_hooks()`, returning a `Box<dyn HookWatcher>` |
| Native defaults | `runtime_defaults.rs` | config path, format and keys | `native_defaults` |
| Model discovery | `runtime_status/models.rs` | query command and config home | `model_discovery()` |
| Versions | `runtime_status/versions.rs` | Claude's release channel picks the npm dist-tag | `catalog()` and a `npm_dist_tag` default |
| Usage | `usage.rs`, `UsageSnapshot`, fixed lists in `app_shell.rs` | fetchers for Claude, Codex and agy | `usage()`; the snapshot becomes a map keyed by `Runtime` |
| Skills | `skills.rs`, `agent_skill.rs`, `app_store/skill_defaults.rs` | toggles via Codex config, Copilot `disabledSkills` or Claude overrides; read-only roots elsewhere | `skills()` |
| MCP | `ops/mcp.rs` `McpClientId` | config path and format, and the `claude_code` wire name | `mcp()` |
| Codex speed | `ops/role.rs`, `ops/slot.rs`, `ops/session.rs`, `spawn.rs` | Codex only | a speed capability in `catalog()`, args in `launch_args` |
| App identity | `chat_icon.rs`, captions in `settings/skills.rs`, usage labels in `app_shell.rs`, `effort_needs_launch_model` in `start_chat.rs` | icon, tint, copy, and one capability | the app's `runtime_ui` table; the capability moves to the catalog |

### Rules

- **The registry is a `match`, not self-registration.** Rust cannot self-register without link-time crates such as `inventory` or `linkme`. One exhaustive arm in `runtimes::adapter()` is explicit, and the compiler forces it.
- **No `match` on an agent variant outside `runtimes/`.** The exceptions are the identity table in `runner-core`, the app's `runtime_ui` table, and tests. Checks of `Runtime::Shell` stay where they are: terminal versus agent is a product distinction, not per-runtime behavior, and they become `runtime.is_shell()`.
- **Shell and unknown keys use `NoAgent`.** It takes every default: empty argv, a fresh resume plan, no hooks. Callers that hold `Option<Runtime>` today call `runtimes::for_key(key)`, which returns `&NoAgent` for an unknown or legacy key, so call sites carry no `Option` and unknown rows behave exactly as they do now.
- **Shared mechanisms become helpers that adapters call.** Codex and TRAE share the rollout capture; Claude and pi share the rekey drop; Codex, Copilot and agy share `--add-dir`; Go-style flag parsing is shared by any adapter that needs it. No adapter calls another adapter.
- **UI-only data stays in the app; behavior and capabilities live in the backend.** `runner-app` keeps one `runtime_ui(Runtime) -> RuntimeUi { icon, tint, skills caption, usage labels }` table, because it holds GPUI colors and copy that [#565](https://github.com/yicheng47/runner/issues/565) will translate. Everything else the app shows (offered permission modes, whether effort needs a model, usage support, fork support) comes from the `RuntimeCatalogEntry` it already receives.
- **No behavior change.** Argv, env, persisted rows, wire names (including `McpClientId`'s `claude_code`), log lines and error strings stay byte-identical. Column and field names such as `codex_speed` keep their names.

### After the change

Adding a runtime means a `runtimes/<name>/` module, one `Runtime` variant with its identity row, one adapter arm, one `runtime_ui` row, an icon asset, and its tests. The product work does not shrink and stays in the checklist: the README pair, the Pencil mark, a terminal fixture, and the smoke record.

## Non-goals

- Plugins, runtimes loaded at run time, or runtimes defined in a config file. Antigravity and Claude need code for log tailing, trust files and hooks, so a declarative manifest would cover only the simplest CLIs.
- New runtime features, and fixes to existing per-runtime gaps. A bug found during the move gets its own issue and stays in its current form, unless it blocks the move.
- Renaming `SessionRuntime`, `PtyRuntime` or `RuntimeSession`. The new trait is named `RuntimeAdapter` precisely to avoid a clash with those PTY-layer names.

## Decisions

Settled with Jason on 2026-10-01.

1. **`Runtime` moves into `runner-core`.** Core already has `serde`; the `schemars` derive goes behind a core feature that `runner-backend` enables, and `runner_backend::model::Runtime` stays as a re-export so import paths don't change. Key, display name, default command and managed-skill root sit next to the enum, which removes the CLI's `runtime_command`. `RUNNER_SKILL_ROOTS` stays a const, because its order is observable in `runner skill` output, and a core test ties it to the identity rows' managed roots.
2. **Three PRs, one mission each, in order.** PR 1 is phases 0 and 1: the argv layer, which the golden tests prove unchanged. PR 2 is phase 2: spawn hooks and status watchers, the riskiest part, because key capture, trust and hook watchers only show in a live session. PR 3 is phases 3 and 4: making `UsageSnapshot` a map and replacing `McpClientId` break `app_shell.rs` and Settings → MCP in the same change, so the catalog and the app move together without temporary shims. Each mission starts after the previous PR merges.
3. **Authorized keyboard fix in PR 3, 2026-10-03.** During manual smoke testing, Jason reported that Enter on the New Chat model chooser submits the chat and explicitly asked to fold the fix into PR #792. When model suggestions are available, Enter opens the closed chooser or selects its highlighted model without submitting; Cmd+Enter (Ctrl+Enter on Windows) remains the form-wide submission shortcut. This is the sole authorized behavior-change exception to the catalog refactor. The catalog expectations remain frozen, and existing Enter behavior in other text fields is preserved.

## Implementation phases

The phases land as three PRs: phases 0 and 1, then phase 2, then phases 3 and 4. Each PR leaves behavior unchanged and keeps the golden tests passing without edits. Temporary `Option<Runtime>` shims may live between PRs but are gone by the end of PR 3.

0. **Characterization tests.** Golden tests that build the composed argv and env for each of the six runtimes across these launch shapes: direct chat fresh and resumed; mission worker and lead; fork where supported; each offered permission mode in a direct chat and in a mission; model and effort set and unset; hooks on and off. The app data dir, session ids and generated UUIDs are normalized. The tests are written and passing against current `main` before any code moves.
1. **Trait, registry and identity.** Add `runtimes/` with the trait, `NoAgent` and the six adapters. Move the identity data, the catalog merge, and every `router/runtime.rs` argv, permission, resume and fork function into adapters. `router/runtime.rs` keeps only shared types (`ResumePlan`, `ForkPlan`, `PermissionMode`, `MissionPermissionMode`) and helpers.
2. **Spawn hooks.** Launch env and args, trust seeding, the launch gate, key capture and status hooks. `HookStatusWatcher` becomes `Box<dyn HookWatcher>`, and `SpawnSpec` carries what `PtyRuntime` needs to ask the adapter for a watcher instead of probing env-var pairs. The per-runtime `session/*` files move into their adapter folders. The mechanisms shared by two runtimes (`codex_capture.rs` for Codex and TRAE, `claude_rekey.rs` for Claude and pi) stay in `session/`, with `hook_feed.rs` and `status.rs`. `key_capture` returns an enum that names the mechanism rather than a trait object, because the four mechanisms take different spawn contexts and two are shared.
3. **Catalog surfaces.** Native defaults, model discovery, versions, usage (`UsageSnapshot` becomes a map), skills and MCP (`McpClientId` becomes a thin wrapper over `Runtime` with the adapter's wire name).
4. **App and docs.** Add the `runtime_ui` table; drive permission modes, usage lists and the effort rule from the catalog. Rewrite "Where the implementation lives" in [`runtime-integration.md`](../arch/runtime-integration.md) around `runtimes/<name>/`, and point `docs/arch/arch.md`'s references to the adapter in `router/runtime.rs` at `runtimes/`.

## Verification

- `make verify` passes for every PR, and CI is green on macOS and Windows. Imports and helpers used only by `cfg(unix)` tests are `cfg(unix)`-gated.
- The golden expectation files do not change after phase 0.
- At the end, `rg 'Runtime::(Codex|ClaudeCode|Antigravity|Pi|Copilot|Trae)\b' crates -g '*.rs'` matches only `runtimes/`, the `runner-core` identity table, the app's `runtime_ui`, and tests. Review checks this with a grep rather than a test that scans source files.
- **Stub runtime check:** at the end of phase 4, a throwaway branch adds an `Example` runtime that appears in Settings → Agents and launches as a chat. Its diff touches only the files listed under [After the change](#after-the-change). The PR records that file list, and the branch is not merged.
- **Jason's smoke test** on each PR, on the installed runtimes (Codex, Claude Code, Antigravity, pi). Every PR: one chat and one mission slot each, a resume, and a model and effort override. PR 2 adds status changes through a turn and an interrupt, and a conversation switch (`/new` or `/clear`) followed by a resume. PR 3 adds the usage popover and Settings → Agents, Skills and MCP. Nothing should look or behave differently.

## Relevant code

- `crates/runner-backend/src/model.rs`: the `Runtime` enum.
- `crates/runner-backend/src/router/runtime.rs`: `RUNTIME_DEFINITIONS` and the argv, permission, resume and fork functions.
- `crates/runner-backend/src/router/prompt.rs`: `split_session_prompt`.
- `crates/runner-backend/src/session/manager/spawn.rs`: env, trust, capture, status env, conversation-missing handling, the launch gate.
- `crates/runner-backend/src/session/pty_runtime.rs`: `HookStatusWatcher`.
- `crates/runner-backend/src/session/`: `{agy,claude,codex,copilot,pi}_*.rs`, `hook_feed.rs`, `claude_rekey.rs`.
- `crates/runner-backend/src/ops/runtime.rs`: the catalog. `ops/mcp.rs`: `McpClientId`. `ops/role.rs`, `ops/slot.rs`, `ops/session.rs`: Codex speed.
- `crates/runner-backend/src/runtime_defaults.rs`, `runtime_status/{models,versions}.rs`, `usage.rs`, `skills.rs`, `agent_skill.rs`.
- `crates/runner-core/src/lib.rs`: `RUNNER_SKILL_ROOTS`. `crates/runner-cli/src/command.rs`: `runtime_command`.
- `crates/runner-app/src/`: `chat_icon.rs`, `surfaces/app_shell.rs`, `surfaces/roles/logic.rs`, `surfaces/settings/skills.rs`, `surfaces/start_chat.rs`, `app_store/skill_defaults.rs`.
