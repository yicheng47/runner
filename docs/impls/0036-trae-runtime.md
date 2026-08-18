# TRAE CLI as a runtime

## Status

Planned. Tracking issue [#361](https://github.com/yicheng47/runner/issues/361). Capabilities were live-probed against `traecli 0.200.19 (internal edition)` on 2026-07-27: help output for the interactive and `exec` surfaces, a real `exec` run in a scratch git repo, the on-disk rollout envelope, and an add/remove round-trip through `traecli mcp`. Follows [0034](archive/0034-qoder-runtime.md), which added qoder and established that a runtime is mostly a catalog edit.

## Problem

`RUNTIME_DEFINITIONS` (`router/runtime.rs:37`) has three entries. TRAE CLI is a **Codex fork**, so the adapter is codex's, cloned. The evidence is direct rather than inferred: the rollout envelope records `"originator":"codex_exec"`, and the CLI ships codex's `-c key=value` TOML override flag, the `model_reasoning_effort` config key, the `[mcp_servers.<name>]` config shape, `-a/--ask-for-approval`, `-s/--sandbox`, a `resume` subcommand, `apply`, `--oss`, and the `sessions/YYYY/MM/DD/rollout-<ts>-<uuidv7>.jsonl` layout — rooted at `~/.trae/cli/sessions` rather than `~/.codex/sessions`.

What makes this more than a catalog edit is that trae is the **second capture-needing runtime**. Qoder took the claude-code path (caller-supplied `--session-id`), which is why 0034 decision 1 deliberately avoided `codex_capture.rs` and non-goal 3 deferred generalizing it on the grounds that "no third self-assigning runtime is on the table." Trae is not self-assigning, so that module has to be parameterized now.

Five specific traps, all confirmed in the current tree or by probe:

1. **`codex_capture` is hardcoded to codex twice over.** The sessions root is built inline as `$HOME/.codex/sessions` (`codex_capture.rs:91`), and capture is gated on `runner.runtime == "codex"` at three spawn sites (`spawn.rs:489`, `:876`, `:1336`). Trae's root carries an extra segment (`.trae/cli/sessions`), so a naive copy of the codex arm produces a runtime that never captures a key and therefore never resumes.
2. **`--session-id` looks like self-assignment but is not.** The flag exists on both the interactive and `exec` surfaces, documented as "Legacy compatibility flag for selecting or naming the session." Probed directly: `traecli exec --session-id d0971d26-…` produced rollout `019fa1b9-a133-7841-…` and our supplied id appears in **no** rollout file. Mirroring qoder or claude-code here would persist an `agent_session_key` that can never resume, and the failure is silent until the user restarts a chat.
3. **Frontend session-key polling is codex-gated.** `MissionWorkspace.tsx:454`, `:482` and `RunnerChat.tsx:530` all test `runtime === "codex"` before polling for the captured key. Backend capture can work perfectly and the key readout still never populates.
4. **`runtime_purges_on_resume` (`output.rs:828`) is a denylist** — `!matches!(runtime, Some("claude-code") | Some("qoder"))`. For once the inverted default is *correct*: trae repaints its whole frame on resume exactly like codex, so it must be left out of that list. Adding it "for symmetry" with the other three-runtime match arms would garble resumed panes.
5. **Directory trust gate.** `traecli exec` in an untrusted or non-git directory fails with `Not inside a trusted directory and --skip-git-repo-check was not specified`. Codex has the same posture, so this is not new, but Runner spawns into arbitrary project cwds and the first spawn in a fresh directory may block on trust.

## Key Decisions

1. **Clone codex's adapter, not claude-code's.** `resume_plan("trae", Some(uuid))` → `args: ["resume", uuid]` with `prepend: true` (subcommand prefix, same as codex); fresh spawn → empty args and `assigned_key: None`, with the key arriving post-spawn through capture. Trae also accepts a `--resume <uuid>` flag form, but the subcommand form is what codex's branch already encodes and what the caller's prepend plumbing expects.
2. **Parameterize `codex_capture`'s sessions root; do not rename the module.** Add the resolved root to `CaptureRequest` (or a `runtime` field plus a `sessions_root_for(runtime)` helper) so `run()` stops building the path inline, and widen the three spawn gates to `matches!(runner.runtime.as_str(), "codex" | "trae")`. Renaming `codex_capture` / `CodexCaptureContext` / `SessionHandle.codex_capture` / `capture_codex_session_key` to something runtime-neutral is a large diff with zero behavior change — leave the names and add a module doc line stating it serves codex-lineage runtimes. Everything else in the module already generalizes: the rollout envelope, the `payload.id` field, the pid-owned-rollout scan, the cwd + start-time fallback, and the `claimed_rollouts` de-dup are all identical for trae.
3. **Permission modes mirror codex exactly.** `permission_mode_args("trae", Auto)` → `["--ask-for-approval", "on-request", "--sandbox", "workspace-write"]`; `Bypass` → `["--ask-for-approval", "never", "--sandbox", "workspace-write"]`; `AcceptEdits` → empty (no equivalent, reads as Default). `strip_permission_flags("trae")` → `[("--ask-for-approval", true), ("--sandbox", true)]`. Trae *also* declares a qoder-style `--permission-mode default|bypass_permissions|auto`, deliberately unused: two mechanisms for one setting invites drift, and codex's pair is what the semantics table already encodes. Only `--sandbox read-only` was live-probed; the remaining values are help-declared with value sets identical to codex's.
4. **Model and effort come free; model suggestions do not.** Trae joins codex's arm in `model_effort_args` — `-m/--model` plus `-c model_reasoning_effort=<lowercased>`, and the user's own `~/.trae/traecli.toml` already carries a `model_reasoning_effort` key, confirming the config path. Effort options mirror codex's TOML enum. `MODEL_SUGGESTIONS_BY_RUNTIME` gets **no** trae key (degrades to `[]`): its catalog is internal and differs from codex's `gpt-5-codex` family, and `traecli models` is the source of truth if suggestions are wanted later.
5. **MCP registration reuses codex's TOML writer.** `~/.trae/traecli.toml` stores servers as `[mcp_servers.<name>]` with `command` / `args` — verified by adding and removing a probe entry. `codex_status_at` / `codex_write_at` work unchanged behind a new path function. This matters more than for codex: the file is mode `600` and also holds auth state and per-hook trust hashes, so the `toml_edit` document-preserving write (which those functions already use) is load-bearing, and only `mcp_servers.runner` may be touched. `traecli mcp add/remove` exists but is not used — direct config editing is how Runner already treats codex.
6. **Catalog and internal docs yes, public README no.** Trae is an internal-edition binary authed against Trae with plugins sourced from `code.byted.org`; it is not publicly installable. Advertising it in the README tagline and feature copy would be a promise readers can't act on. The runtime simply activates when `traecli` is on `PATH`. `docs/arch/arch.md` and the MCP tool doc comment do get it, since those are engineering references.
7. **Hook-based session status is out of scope.** Worth recording because it is a genuine find: the probe run emitted `hook: UserPromptSubmit` and `hook: Stop`, and `traecli.toml` carries per-hook trust state for `user_prompt_submit`, `pre_tool_use`, `post_tool_use`, `post_tool_use_failure`, and `stop` — claude-code's event vocabulary, declared in a plugin's `hooks.json`, with `--dangerously-bypass-hook-trust` available for automation. That would make trae a first-class runtime for spec 52. The open question is whether a Runner-owned hook can be injected per-spawn via `-c hooks…=` overrides without writing to the user's config, which spec 52 requires. Not investigated here.
8. **No `--skip-git-repo-check`, no launch gate.** Keep the same posture as codex: don't pass trust-bypass flags speculatively. If spawns into non-git project cwds fail, that becomes a follow-up with a real reproduction. Likewise no `enter_claude_launch_gate` equivalent — that exists for claude-code's OAuth refresh race, and adding a serialization gate for a hypothetical would slow every multi-slot mission.

## Goals

- Trae is selectable everywhere the other three runtimes are: direct-chat runtime picker, runner form, and crew slot runtime override.
- Settings → Agents shows a TRAE CLI row with its detected `traecli` path, an override field, and the same states as the other runtimes.
- Settings → MCP can register or unregister Runner in `~/.trae/traecli.toml` without disturbing auth or hook-trust state.
- A trae direct chat spawns, takes its first turn via positional argv, captures its rollout id into `agent_session_key`, stops, and resumes into the same conversation.
- A trae mission slot receives its worker preamble and responds to inbox nudges.
- A missing or deleted rollout falls back to a fresh spawn instead of an error loop.

## Non-Goals

- Hook-based session status (decision 7) — belongs to spec 52.
- Renaming `codex_capture` and its types to runtime-neutral names (decision 2).
- Model suggestions for trae (decision 4).
- Trae's `--permission-mode` preset flag (decision 3), `--worktree` isolation, `--add-dir`, `--search`, and `--no-alt-screen`.
- README and public-facing runtime enumerations (decision 6).
- Generating `src/components/ui/runtimes.ts` from the Rust catalog — still a hand-port, as 0034 left it.

## Implementation Phases

### Phase 1 — Rust catalog and adapter (`src-tauri/src/router/runtime.rs`)

- **Append** to `RUNTIME_DEFINITIONS` (`:37`): `{ name: "trae", display_name: "TRAE CLI", command: "traecli" }`. Append, never prepend — `RUNTIME_OPTIONS[0]` is the Create Runner default and a `runtime_status.rs` test indexes `.runtimes[0]`.
- `resume_plan` (`:612`): add a `"trae"` arm mirroring codex — `Some(k) if is_uuid(k)` → `args: ["resume", k]`, `prepend: true`, `assigned_key: Some(k)`, `resuming: true`; otherwise the fresh-spawn shape with `assigned_key: None` (decision 1). Note trae's ids are UUIDv7, which `is_uuid` accepts.
- `first_turn_argv` (`:522`): add `"trae"` to the `"claude-code" | "codex" | "qoder"` arm. Mandatory — without it a trae lead spawns with no persona and no mission goal.
- `model_effort_args` (`:107`): add `"trae"` to codex's arm (`--model`, `-c model_reasoning_effort=<lowercased>`).
- `permission_mode_args` (`:198`) and `mode_match_pairs` (`:372`): trae arms mirroring codex per decision 3.
- `strip_permission_flags` (`:263`): `"trae"` → `&[("--ask-for-approval", true), ("--sandbox", true)]`.
- `src-tauri/src/commands/mcp.rs`: add Trae to the client enum (`:66`), the snippet and status structs (`:12`, `:22`), a `trae_path()` returning `~/.trae/traecli.toml`, and wire status/write to the existing `codex_status_at` / `codex_write_at` (decision 5) — the same delegation pattern `qoder_status_at` uses for claude-code.
- Tests: extend the multi-runtime loops (`:1548` and the permission matrix near `:1090`) to include trae; add resume-plan coverage for both the fresh and prior-key arms; assert the strip set round-trips.

### Phase 2 — Capture parameterization, spawn and output policy

- `src-tauri/src/session/codex_capture.rs`: replace the inline `$HOME/.codex/sessions` (`:91`) with the root carried on `CaptureRequest`; add `sessions_root_for(runtime)` mapping `"codex"` → `.codex/sessions` and `"trae"` → `.trae/cli/sessions`; keep the `is_dir()` bail-out. Update the module header (`:6-7`) to name both runtimes.
- `src-tauri/src/session/manager/spawn.rs:489`, `:876`, `:1336`: widen `runner.runtime == "codex"` to `matches!(runner.runtime.as_str(), "codex" | "trae")` and pass the resolved root into `CodexCaptureContext`.
- `src-tauri/src/session/manager/spawn.rs:189-200` `codex_capture_prompt_marker`: same widening, so trae gets the prompt-marker disambiguation path that protects sibling chats in one cwd.
- `src-tauri/src/session/manager/spawn.rs:550`, `:936`, and the third site near `:1421`: add `"trae"` to the first-turn-not-delivered warning gate so the diagnostic stays honest.
- `src-tauri/src/session/manager/output.rs:812-815` `runtime_clears_on_resize`: add `Some("trae")`.
- `src-tauri/src/session/manager/output.rs:843` `runtime_purges_on_resume`: **no change** — leaving trae out of the exception list is what gives it codex's purge-on-resume (trap 4). Add a test asserting it, so a later "symmetry" edit fails loudly.
- `src-tauri/src/session/manager/output.rs:813` full-repaint list and the `Some("claude-code") | Some("codex") | Some("qoder")` arm near `:874`: add trae.
- Tests in `session/manager/tests.rs`: mirror codex's purge-on-resume coverage for trae, add a trae case to the `resolve_runtime_override` matrix, and cover `sessions_root_for` for both runtimes.

### Phase 3 — Frontend

- `src/components/ui/runtimes.ts`: **append** to `RUNTIME_OPTIONS` (`:18`) — `{ value: "trae", label: "trae", defaultCommand: "traecli", description: "TRAE CLI" }`. Add `"trae"` to `RUNTIMES_CLEARING_ON_RESIZE` (`:39`, in lockstep with Phase 2) and `RUNTIMES_WITH_PERMISSION_MODE` (`:55`). Add `PERMISSION_MODES_BY_RUNTIME` (`:178`), `MODE_MATCH_PAIRS_BY_RUNTIME` (`:245`), `PERMISSION_STRIP_KEYS_BY_RUNTIME`, and `EFFORT_OPTIONS_BY_RUNTIME` (`:85`) entries mirroring codex. Leave `MODEL_SUGGESTIONS_BY_RUNTIME` without a trae key (decision 4).
- `src/pages/MissionWorkspace.tsx:454`, `:482` and `src/pages/RunnerChat.tsx:530`: widen the `runtime === "codex"` session-key polling guards to include trae (trap 3).
- `src/components/settings/McpPane.tsx`: add the TRAE CLI registration toggle and manual-config copy action alongside the existing three.
- `AgentsPane.tsx` needs no change — it renders from `status.runtimes` and `RUNTIME_OPTIONS`, with no hardcoded row count (0034's skeleton fix already landed).

### Phase 4 — Docs and verification

- `docs/arch/arch.md`: update the runtime enumerations and record trae's purge-on-resume / clear-on-resize policy alongside the others.
- `src-tauri/src/mcp/tools/session.rs:16`: the `runtime` doc comment lists valid names — add trae.
- `src-tauri/src/commands/slot.rs`: the invalid-runtime error lists valid names; extend the assertion at `:843` to cover trae.
- Leave `README.md` untouched (decision 6).
- Full check matrix: `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

## Verification

Automated:

- [ ] `resume_plan("trae", None)` yields no args and `assigned_key: None`; with a prior UUID it emits `["resume", uuid]` with `prepend: true`.
- [ ] `first_turn_argv("trae", body)` returns the positional body; suppressed on resume.
- [ ] Permission round-trip: Auto → `--ask-for-approval on-request --sandbox workspace-write`; switch to Default → both flags stripped.
- [ ] `model_effort_args("trae", Some("…"), Some("High"))` emits `--model` plus `-c model_reasoning_effort=high` (lowercased).
- [ ] `sessions_root_for` returns `.codex/sessions` for codex and `.trae/cli/sessions` for trae.
- [ ] `runtime_purges_on_resume("trae")` is **true**; `runtime_clears_on_resize("trae")` is true; the TS mirror agrees.
- [ ] Unknown-runtime slot-override rejection lists trae among valid names.
- [ ] Trae MCP registration creates `~/.trae/traecli.toml` when absent, preserves unrelated keys — specifically `[hooks.state]` entries and auth values — and removes only `mcp_servers.runner` when disabled.

Manual (the human smoke-tests):

- [ ] Direct chat: spawn, converse, stop, resume — conversation restored via `traecli resume <uuid>`, no blank grid.
- [ ] `agent_session_key` populates within the capture window; the key readout appears in the chat meta line (confirms trap 3 is fixed).
- [ ] Two trae chats started seconds apart in the **same** cwd each capture their own rollout id — no fused keys (exercises the prompt-marker and `claimed_rollouts` paths).
- [ ] First-turn prompt auto-submits in interactive mode.
- [ ] Mission slot: worker preamble lands; the inbox nudge submit chord (80ms Enter) lands in its input box.
- [ ] Resize a trae pane mid-conversation — frame repaints cleanly, no shredded box-drawing.
- [ ] Delete the rollout `.jsonl` and resume — falls back to a fresh spawn without an error loop.
- [ ] Spawn a trae chat in a cwd that is not a git repo — record whether the trust gate blocks it (decision 8; becomes a follow-up if it does).
- [ ] Settings → Agents shows a TRAE CLI row with the detected `traecli` path.
- [ ] Settings → MCP toggles trae registration; a new trae session sees Runner tools.
