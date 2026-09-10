# 539 — pi runtime

> Tracking issue: [#539](https://github.com/yicheng47/runner/issues/539)
> Priority: P2.

## Motivation

pi ([earendil-works/pi](https://github.com/earendil-works/pi), formerly badlogic/pi-mono) is a terminal-first coding agent at roughly 103k stars and the agent the Chinese dev community is building around right now: V2EX and linux.do threads ask for a GUI, and at least five desktop shells have appeared since July. vastsa/PI-Desktop alone reached 1.7k stars six weeks after its first release, mostly by being the answer in those threads. Runner is already a desktop cockpit for CLI agents, so supporting pi puts it in that conversation for the cost of one runtime adapter entry, and every "pi tools" list and awesome-list becomes a place Runner can be linked from.

pi also fits Runner's adapter better than either first-class runtime. Probed against pi 0.85.1 on 2026-09-10:

- The interactive TUI honours `--append-system-prompt <text>` (claude-code's equivalent is print-mode only and codex has none), so `runner.system_prompt` can ride the native hook the adapter was designed for instead of the first-turn fold.
- `--session-id <id>` uses an exact project session id and creates it if missing, so the key is caller-assigned like claude-code and there is no post-spawn rollout capture. `--fork <id>` is native.
- `--model <provider/id>` and `--thinking <level>` map onto Runner's model/effort fields directly.
- It is bring-your-own-model (Anthropic, OpenAI, `openai-codex` OAuth, `github-copilot/*`, local), which gives users on a ChatGPT or Copilot subscription and no Claude a way into Runner.

## Scope

### In scope

- **Runtime catalog entry.** `pi` in `RUNTIME_DEFINITIONS` with command `pi`, display name `pi`, `native_fork: true`, `skills_dirs: [".pi/agent/skills", ".agents/skills"]` (pi discovers user skills under both, plus `.pi/skills` and `.agents/skills` under the project cwd).
- **Model and effort.** `model` emits `--model <value>` (pi accepts `provider/id`, a bare pattern, or `pattern:<thinking>`); `effort` emits `--thinking <level>` with pi's enum `off | minimal | low | medium | high | xhigh | max`. Runtime defaults come from `~/.pi/agent/settings.json`: `defaultModel` prefixed by `defaultProvider` when both are set, no effort default (pi records thinking level per session, not in settings).
- **System prompt.** `system_prompt_args("pi", prompt)` returns `["--append-system-prompt", prompt]`. This is the first runtime where the hook is non-empty, so the direct-chat first turn (`compose_direct_first_turn`) must be empty for pi rather than repeating the persona as a user turn; mission launch prompt and worker preamble still ride the positional first turn, with the persona section dropped from the pi body because it already arrived on the system prompt.
- **First turn.** The composed body lands as the trailing positional message, same as claude-code, codex, and trae.
- **Session key and resume.** Fresh spawns pre-assign a UUID and pass `--session-id <uuid>`; resume passes the same flag (pi resumes when the id exists, creates otherwise, so the resume-fresh fallback is free). The conversation-exists probe looks for `~/.pi/agent/sessions/<cwd-slug>/<ts>_<uuid>.jsonl`, whose first line is `{"type":"session","id":"<uuid>","cwd":"<cwd>"}`. Native fork (spec 60) uses `--fork <id>` and assigns a fresh `--session-id` for the child.
- **No permission mode.** pi has no approval gating (tools run; control is the `--tools` / `--exclude-tools` allowlists). Hide the permission-mode control for pi rows, keep `strip_permission_flags` a no-op, and make the mission permission mode (#527) a no-op for pi slots.
- **Mission bus.** pi has no sandbox, so `runner msg` / `runner signal` work from its bash tool with no `--add-dir` equivalent. `mission_bus_sandbox_args` stays empty.
- **Settings → Agents row.** Detect `pi` on PATH with the existing executable override, show `pi --version`, and wire the #533 **Update** button to `pi update self`. Enabled by default when detected, both platforms.
- **Skills pane (#73).** pi's catalog comes from the two home-relative dirs above; global on/off is out of scope until a pi-side override mechanism is probed (`settings.skills` exists in pi's config but is unverified).
- **Terminal fixture.** Capture a pi TUI transcript for `crates/runner-app/tests/fixtures`. pi's default `--tui-mode regular` is inline, not alt-screen, so host scrollback behaves like codex rather than claude-code 2.1+.
- **Docs.** README supported-agents table, `docs/arch` wherever first-class runtimes or per-runtime prompt/resume behaviour are enumerated.

### Out of scope

- pi extension and package management (`pi install`, `pi config`), prompt templates, themes.
- Curated provider/model catalog UI; the model field stays free text with pi's `provider/id` shape documented in the placeholder.
- ACP. pi's `--mode rpc` is a different integration surface and Runner's contract is the PTY.
- MCP registration from Settings → Agents. pi has no built-in MCP client; `mcp_defaults` skips pi.
- Bundling or installing pi.

## Implementation Phases

### Phase 1 — adapter

- Add the `pi` definition to `crates/runner-backend/src/router/runtime.rs` and cover `runtime_definitions`, `runtime_display_name`, `supports_native_fork`, `model_effort_args`, `system_prompt_args`, `first_turn_argv`, `permission_mode_args` (empty), `infer_permission_mode` (default), and `resume_plan` for fresh, resume, and fork.
- Add `"pi"` to `runtime_defaults` reading `~/.pi/agent/settings.json`, with tests for provider+model, model only, and missing file.
- Add the pi conversation-exists probe beside `claude_code_conversation_exists`, keyed on pi's cwd-slug directory layout.
- Extend `compose_direct_first_turn` and the worker/launch composition so pi bodies omit the persona section.
- Add `"pi"` to the spawn-path runtime matches (`first-turn warning`, `agent_session_key` handling); no launch gate, no codex-style capture thread.

### Phase 2 — UI

- Start Chat runtime picker and runner create/edit form list `pi`, command prefilled `pi`, effort dropdown showing pi's seven levels, permission-mode control hidden.
- Settings → Agents row per #533: detect, version, update, executable override.
- Skills pane catalog for pi from its two home-relative dirs.

### Phase 3 — mission smoke

- Crew with a pi slot: PTY paints, launch prompt arrives once, `runner msg read/post` and `runner signal ask_human` succeed from pi's bash tool.
- Close and relaunch Runner: the pi session resumes by id with its history intact.
- Fork a pi chat (spec 60) and confirm the child has its own session file.
- Windows: same smoke on JASONPC; pi is a Node CLI so it should behave like codex under ConPTY.

### Phase 4 — docs

- README supported-agents table gains a pi row.
- `docs/arch` runtime enumerations and the adapter comment block in `router/runtime.rs` stop saying no runtime honours a system-prompt flag.

## Verification

- [ ] `runtime_list` includes `pi` with command `pi` and `native_fork: true`.
- [ ] A pi direct chat with a persona spawns with `--append-system-prompt` and no persona user turn.
- [ ] `model = openai/gpt-5.5`, `effort = high` produce `--model openai/gpt-5.5 --thinking high`.
- [ ] Fresh spawn passes `--session-id <uuid>` and the row's `agent_session_key` is set before the process starts.
- [ ] Relaunching Runner resumes the pi conversation; deleting the session file falls back to a fresh session without error.
- [ ] pi rows show no permission-mode control; mission permission mode leaves pi argv untouched.
- [ ] A mission with a pi slot completes a `runner msg` round trip.
- [ ] Settings → Agents shows the pi version and `pi update self` runs in the update pane.
- [ ] The pi terminal fixture renders without stray escapes in the runner-app tests.
