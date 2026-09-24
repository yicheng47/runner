# 592 — OpenCode runtime

> Tracking issue: [#592](https://github.com/yicheng47/runner/issues/592)
> Priority: P2, milestone 0.12. Platforms: macOS first; Windows after a JASONPC smoke.
> Probed 2026-09-23 against OpenCode 1.18.30 (`~/.opencode/bin/opencode`, curl install; npm `opencode-ai` is at 1.18.32). Every claim below comes from `--help`, the read-only SQLite schema of the real database, OpenCode's source at tag `v1.18.30` (anomalyco/opencode), or a live TUI recorded in a PTY inside a throwaway environment: `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `OPENCODE_DB` and `OPENCODE_CONFIG_DIR` all pointed into a scratch directory, with no sign-in and one provider, a local OpenAI-compatible stub on `127.0.0.1` that answers "Hello from the fake model." No real model was called and nothing under Jason's `~/.config/opencode` or `~/.local/share/opencode` was read beyond the schema or written. This spec supersedes the issue body where they differ.

## Motivation

OpenCode is one of the most-used open-source terminal coding agents and brings its own model (Anthropic, OpenAI, GitHub Copilot, Z.ai, local providers), so supporting it puts Runner in front of people who never run Claude Code or Codex. It is a TUI on a PTY and fits the adapter the way pi ([539](./archive/539-pi-runtime.md)), Copilot ([540](./archive/540-copilot-cli-runtime.md)) and Antigravity ([644](./644-antigravity-runtime.md)) do. The inventory in 540 is the checklist; this spec lists only OpenCode's values. The branch is stacked on #644, so every list, match and table appends OpenCode after Antigravity.

## Probe evidence

### CLI surface

- **TUI flags** (`opencode --help`): `[project]`, `-m/--model provider/model`, `-c/--continue`, `-s/--session <id>`, `--fork` (with `--continue` or `--session`), `--prompt <text>`, `--agent <name>`, `--auto` ("auto-approve permissions that are not explicitly denied (dangerous!)"), `--mini`, and the server set (`--port`, `--hostname`, `--mdns`, `--cors`). The source adds two hidden aliases for `--auto`: `--yolo` and `--dangerously-skip-permissions` (`tui.ts`: `auto: args.auto || args.yolo || args["dangerously-skip-permissions"]`). No system-prompt flag and no effort flag; `--variant` exists on `opencode run` only.
- **Parsing is yargs.** Booleans take `--flag`, `--flag=true|false` and `--no-flag`.
- **Subcommands:** `acp`, `mcp` (`add`, `list`, `auth`, `logout`, `debug`), `attach`, `run`, `debug` (`config`, `paths`, `skill`, `agent`, …), `providers` (alias `auth`), `agent`, `upgrade [target]` (`--method curl|npm|pnpm|bun|brew|choco|scoop`), `uninstall`, `serve`, `web`, `models [provider]`, `stats`, `export`, `import`, `github`, `pr`, `session` (`list`, `delete`), `plugin`, `db` (`path`, `[query]`).
- **Version:** `opencode --version` prints a bare `1.18.30`.
- **Self-update.** One second after the TUI starts it calls `checkUpgrade`: unless `autoupdate` is `false` in the global config or `OPENCODE_DISABLE_AUTOUPDATE` is set, a newer *patch* release is installed in the background with the detected method, and a minor or major release only raises an "update available" toast (`cli/upgrade.ts`).

### Session store and key

- **SQLite at `~/.local/share/opencode/opencode.db`.** `Database.path()`: `OPENCODE_DB` when set (absolute, `:memory:`, or relative to the data dir), else `<data>/opencode.db` for the `latest`, `beta` and `prod` channels. `<data>` is `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode`, on every platform (`xdg-basedir`). `opencode db path` printed the override in the probe. The database is in WAL mode.
- **`session` table:** `id`, `project_id`, `workspace_id`, `parent_id`, `slug`, `directory`, `path`, `title`, `version`, …, `agent`, `model`, `time_created`, `time_updated`, `time_archived` (ms since the epoch). Messages are in `message` (`data` JSON without the ids) and their parts in `part` (`id`, `message_id`, `session_id`, `time_created`, `data`); a user text part's `data` is `{"type":"text","text":"…"}`.
- **Ids are OpenCode's.** `ses_` plus 26 characters (12 hex, 14 base62), e.g. `ses_f31048251ffepsv6qvMgfycvy1`; session ids are descending, so a newer session sorts first.
- **The row appears at the first message, not at spawn.** A TUI launched with no prompt had no `session` row after 10 s. With `--prompt "say hi"` the row appeared 2.5 s after spawn, then the user message and its part.
- **`--prompt` auto-submits only on the home route**, once sync and the model store are ready (`routes/home.tsx`). With `--session` the TUI navigates to that session and the prompt is not sent: a resume with `--prompt` made no model call.
- **`directory`** is the realpath of the cwd, normalized (`Filesystem.resolve(process.cwd())`); `/tmp/x` is stored as `/private/tmp/x` on macOS. `parent_id` is set only for subagent child sessions.
- **Nothing per-process reaches the row.** The TUI's `session.create` sends only `directory`, `workspace`, `agent` and `model`; `metadata` and `permission` stay null. The log is one shared file, `<data>/log/opencode.log`, appended by every OpenCode process and tagged with a random `run=` id, not a pid; it does carry `message=created id=ses_…`. There is no `--log-file`.
- **Two sessions in one cwd.** Two TUIs started 50 ms apart in the same folder, each with a `--prompt`, created their rows 191 ms apart. Nothing in the rows says which process made which.
- **The first turn is stored verbatim.** A `--prompt` ending in `<!-- runner-opencode-session-key-capture:<id> -->` landed in `part.data` byte for byte (JSON escapes neither `<`, `>`, `-` nor `:`), and the marker was visible in the TUI transcript, as Codex's marker is.
- **Resume:** `--session <id>` reopens the session with its history, from its own folder or any other; the store is global. An unknown id prints `Error: Session not found: <id>` and exits with status 1 within a second (`validateSession`).
- **Fork:** `--session <id> --fork` forks once sync completes, about 2.9 s after spawn, with no message and no model call (`app.tsx` → `session.fork`). The new row has `parent_id` NULL, the fork's `directory`, the title `<source title> (fork #N)`, and a copy of every message and part: each cloned part's `data` is byte-identical to the source's, under new ids.
- **Read-only access.** `sqlite3 -readonly` reads the live database while OpenCode holds it. A read-only connection to a WAL database whose `-shm` and `-wal` files are absent fails with `unable to open database file` (error 14); opening it with `immutable=1` works, and is exact there, because SQLite deletes the side files only after the last connection has checkpointed everything into the main file.

### A per-spawn plugin

- **Plugins load from `OPENCODE_CONFIG_CONTENT`.** A config source's `plugin` list may name a file (`file://…` or an absolute path); a path spec is imported directly, with no npm install (`config/plugin.ts`, `plugin/install.ts`). Plugin lists from every source are concatenated, so the user's own plugins still load. A plugin is a module exporting `async (ctx) => ({ event: async ({ event }) => … })`, run in the server worker, which inherits the TUI's environment.
- **Each process sees only its own sessions.** A probe plugin loaded through `OPENCODE_CONFIG_CONTENT` appended every `session.*` event to a file named by an env var. Two TUIs started 50 ms apart in one folder, each with a `--prompt`, each saw exactly one `session.created`: its own session, matching the first-turn text in the database. The rest of the events were `session.updated` for that same id.
- **Every way a session starts is covered.** A blank TUI whose first message was typed into the PTY reported its session on submit. `--session <id> --fork` reported the fork's id, not the source's. A plain `--session <id>` resume reported no `session.*` event at all. Subagent sessions carry `parentID` (`session.ts` `createNext`), and `/new` in the TUI creates a session like any other.
- **A missing plugin file is harmless.** With `plugin` naming a file that does not exist, the TUI started, sent its `--prompt`, and printed nothing about it.
- Setting `OPENCODE_CONFIG_CONTENT` also skips OpenCode's seeding of an empty global config file on a first run; nothing else keys on it.

### Permissions

- **Built-in defaults** (`agent/agent.ts`): `"*": "allow"`, `doom_loop: ask`, `external_directory: ask` except OpenCode's own and skill dirs, `read` of `*.env` files `ask`. So an unconfigured OpenCode asks for almost nothing; the user's `permission` config is what makes it ask. The Plan agent adds `edit: deny`.
- **Rules are last-match-wins** over `defaults + agent rules + user rules` (`Permission.evaluate` uses `findLast`); `fromConfig` emits rules in object-key order.
- **`--auto`** is TUI-side: while the toggle is on, the TUI answers every `permission.asked` with `once` (`context/sync.tsx`). A `deny` rule never asks, so it still denies. The TUI can toggle it at runtime.
- **`OPENCODE_CONFIG_CONTENT` does merge.** It is deep-merged after the global, project and `OPENCODE_CONFIG_DIR` configs (`config.ts`); `OPENCODE_PERMISSION` is a documented sibling that deep-merges into `permission` last. `opencode debug config` showed `{"edit":"ask","bash":"ask"}` becoming `{"edit":"allow","bash":"ask"}` under either variable, and a string `"permission": "ask"` normalized to `{"*":"ask"}` before the merge.
- **But the merge keeps the user's key order.** With the user's `{"edit":"ask","*":"ask"}`, the injected `edit: allow` stayed in first position and `opencode debug agent build` resolved `edit` to the later `"*": "ask"`: edits still asked. `debug agent plan` showed the injected `edit: allow` as the Plan agent's last `edit` rule, so it also lifts Plan mode's edit deny.

### Model

- `--model provider/model`. An id without a slash raises a toast "Invalid model format"; an unknown id raises "Model x/y is not valid" and the TUI falls back (`context/local.tsx`).
- **The default.** The current model is the first valid of: the per-agent pick (which `--model` sets at startup, and which is not persisted), the agent's `model` in config, then `--model` again, the global `model` (`provider/model`), the recently-used list in `<state>/model.json`, and the first provider's default. So without `--model`, `model` in `opencode.json` is the default for anyone without an agent-level model, and its value is exactly what `--model` takes.
- The reasoning variant is remembered per model in `<state>/model.json` and set in the TUI; the TUI has no flag for it.

### Config, MCP, skills

- **Global config dir** `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`. Loaded in order `config.json`, `opencode.json`, `opencode.jsonc` (later keys win), then `OPENCODE_CONFIG`, the project's `opencode.json[c]`, `.opencode/` dirs and `OPENCODE_CONFIG_DIR`, then `OPENCODE_CONFIG_CONTENT`, then managed config. Files are JSONC: `//` and `/* */` comments and trailing commas. When no env override is set, OpenCode seeds an empty global config file, and it writes `$schema` into any config file that lacks it.
- **`opencode mcp add <name>`** writes the global config with `jsonc-parser`'s `modify`, which kept a `//` comment, a `/* */` comment and a trailing comma in the probe. It picks `opencode.json` if it exists, else `opencode.jsonc`, else creates `opencode.json`. Shapes written under `mcp`:
  - local: `{"type":"local","command":["fs-mcp","/work"]}`, plus `"environment":{"TOKEN":"x"}` with `--env`;
  - remote: `{"type":"remote","url":"https://example.com/mcp"}`, plus `"headers":{…}` with `--header`.
  The schema also has `enabled`, `timeout` and remote `oauth`; `mcp add` writes none of them.
- **Skills** (`skill/index.ts`, confirmed with `opencode debug skill`): `~/.claude/skills` (unless `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS`), `~/.agents/skills`, and `{skill,skills}/` under every config dir, which is `~/.config/opencode` and `~/.opencode`; project `.claude/skills` and `.agents/skills` walking up to the worktree, and `.opencode/skills`. The documented global roots are `~/.config/opencode/skills`, `~/.claude/skills` and `~/.agents/skills`. A duplicate name logs a warning and the later root wins. So OpenCode shares roots with Claude Code and with Codex, Copilot and pi.

### Terminal

- DECSET `1049` alternate screen, `2004` bracketed paste, `2026` and `2027`, `2031` colour-scheme notifications, hidden cursor, and **mouse reporting `1000` + `1002` + `1003` + SGR `1006`**. It sets modifyOtherKeys (`CSI > 4;1 m`), sets the cursor colour (`OSC 12`), and queries DA1, cursor position, `OSC 10/11/4` colours, XTVERSION, XTGETTCAP, kitty keyboard (`CSI ? u`), kitty graphics, `OSC 99` notifications, `OSC 1337` capabilities, `OSC 66` and `CSI 14 t`.
- **Title:** `OSC 0` `OpenCode` on the home screen and for a session with a default title, then `OC | <session title>` (40 characters, else 37 and `…`) once the title is generated, `OC | <plugin id>` on a plugin route, and an empty title on exit. `OPENCODE_DISABLE_TERMINAL_TITLE` or a TUI toggle turns it off.
- Because mouse reporting is on, Runner sends wheel ticks to OpenCode as SGR mouse events, and OpenCode scrolls its own transcript.

## What a new runtime touches

Sites follow the [540 inventory](./archive/540-copilot-cli-runtime.md#what-a-new-runtime-touches) and 644's table; these are OpenCode's values. *Exhaustive* marks the `match` arms the compiler forces.

### Spawn path (`crates/runner-backend`)

| Site | OpenCode |
| --- | --- |
| `model.rs` `Runtime` | `OpenCode`, wire name `opencode`, after `Antigravity` in `ALL`; round-trip test. *Exhaustive.* |
| `router/runtime.rs` `RUNTIME_DEFINITIONS` | Display `OpenCode`, command `opencode`, `native_fork: true`, `skills_dirs: [".config/opencode/skills", ".claude/skills", ".agents/skills"]`, `update_args: ["upgrade"]`, `npm_package: Some("opencode-ai")`. Appended after Antigravity. |
| `model_effort_args` | `--model <id>` verbatim; effort is never passed (decision 4). |
| `first_turn_argv` | `["--prompt", body]`. |
| `system_prompt_args` | Empty; the persona folds into the first turn. |
| `permission_mode_args` and the strip, infer and mission functions | Default → nothing; Bypass → `--auto`; AcceptEdits and Auto hidden (decision 3). The strip set is `--auto`, `--yolo` and `--dangerously-skip-permissions`, bare or `=value`, plus `--no-auto`. *Exhaustive on `(runtime, mode)`.* |
| `mission_bus_sandbox_args` | Nothing: OpenCode has no sandbox, and `runner msg` needs no path argument. |
| `trailing_runtime_args` | Model args and the first turn only. |
| `resume_plan` | Fresh: no key flag, `assigned_key: None`. Resume: `--session <key>` trailing, `assigned_key` set, `resuming: true`, when the key has OpenCode's shape (`ses_` + ASCII alphanumerics). |
| `fork_plan` | `ForkPlan::Direct` with `--session <source> --fork`, `assigned_key: None`, `resuming: true`; the fork reports its own key (decision 1). The UUID guard at the top of `fork_plan` becomes a per-runtime key check. |
| Conversation probe | `opencode_conversation_exists(key, env)`: `SELECT 1 FROM session WHERE id = ?1` in the resolved database, on a fresh read-only connection per call (decision 2). |
| `session/opencode.rs` (new) | The plugin source and its install into `<app data>/opencode/runner-session-key.js`, the spawn environment (decision 1), and the database path and existence check (decision 2). |
| `session/claude_rekey.rs` | The shared drop watcher accepts an OpenCode `ses_…` id for OpenCode rows; other rows keep the UUID check, and everything else about it is unchanged. |
| `session/manager/mod.rs` `start_runtime_watchers` | Installs the plugin file at startup, beside the Copilot plugin and pi extension. |
| `session/manager/spawn.rs` | The plugin environment on every OpenCode spawn, the conversation-missing arm, the persona resend on a fresh fallback, `OPENCODE_DISABLE_AUTOUPDATE=1` in `agent_env` (decision 7), the first-turn warning gate and the Windows batch fallback list. *Exhaustive in two places.* |
| `session/codex_capture.rs` `sessions_root_for` | `OpenCode => None`. *Exhaustive.* |
| `session/title.rs` `decoration` | Add `oc`, so `OC | <title>` keeps the title; `opencode` is already listed, so the bare product name never becomes a tab name. |
| `runtime_defaults.rs` | `model` from `~/.config/opencode/{config.json,opencode.json,opencode.jsonc}`, later files winning; no effort. The JSONC reader learns `/* */` comments and trailing commas (decision 6). *Exhaustive.* |
| `ops/runtime.rs` catalog | Description, `install_url` `https://opencode.ai/docs/`, `default_enabled: cfg!(target_os = "macos")`, models `[default]` only (the field is free text, `provider/model`), efforts `[default]` only. |
| `ops/role.rs`, `ops/slot.rs`, `runtime_status.rs`, `runtime_status/versions.rs` | Runtime lists, the valid-runtime error string and its tests gain `opencode`; the version parser test gains `1.18.30`; `updatable(OpenCode)` is true. |
| `runtime_status/models.rs` `DISCOVERY_RUNTIMES` | Unchanged in v1; `opencode models` is #590's. |
| `session/hook_feed.rs`, `pty_runtime.rs` | Nothing: no hook status (decision 8). `hooks_supported` stays false for OpenCode, and its test lists it as unsupported. |

### Settings, identity, status (`runner-app`, `ops/mcp.rs`, `skills.rs`)

| Site | OpenCode |
| --- | --- |
| `ops/mcp.rs` `McpClientId` | `OpenCode`, key `opencode`, label `OpenCode`, file `~/.config/opencode/opencode.json` (the resolved path follows `mcp add`: `opencode.json`, else an existing `opencode.jsonc`), entry key `mcp.<name>`, entry shapes as `mcp add` writes them (decision 5). *Exhaustive in four matches.* |
| `ops/mcp.rs` JSON splicer | `json_members` and the read path accept JSONC; strict JSON gives the same spans and output as today (decision 5). |
| `app_store/mcp_removal.rs` | A compile arm only: Runner never registered itself with OpenCode. *Exhaustive.* |
| `app_store/skill_defaults.rs` `root_runtimes` | `.agents/skills` gains `OpenCode`, so a machine with only OpenCode detected still gets the Runner skill in a root OpenCode reads. |
| `surfaces/settings/agents.rs` | Catalog-driven. `opencode --version` is a bare semver; `opencode upgrade` is the Update command; the npm check reads `opencode-ai`. |
| `skills.rs`, `surfaces/settings/skills.rs` | Three roots, no global toggle (the TRAE and Antigravity shape); the caption says the Claude and `.agents` roots are shared and that a Claude Code toggle does not hide a skill from OpenCode. |
| `surfaces/settings/mcp.rs` | The format label reads `JSONC` for OpenCode; copy hints and presentation tests name the new client. |
| `assets.rs`, `chat_icon.rs` and the icon tests | A placeholder, `opencode.svg` → the `message-square` glyph, as #644 shipped (decision 9). *Exhaustive in `chat_icon.rs`.* |
| `surfaces/roles/logic.rs` | `[Default, Bypass]` with OpenCode's wording. |
| `surfaces/panes.rs`, `surfaces/sidebar`, `ops/session.rs` `forkable` | OpenCode is forkable once the row has a key. |
| `runner-cli` `runtime_command` | `opencode` → `opencode`. |

### Docs

`README.md` and `README.zh-CN.md` together (the agents column, the Windows footnote, the closing sentence), the runtime enumerations in `docs/arch/arch.md` (§1.1 diagram, §3 runtime list and `runtime TEXT` comment, §4 MCP and Skills, §5 TUIs, first turn, key capture, fork, updates, Runner skill roots) and `docs/arch/windows.md`, the `docs/features/README.md` index line, and `docs/tests/592-opencode-smoke.md`.

## Scope

### In scope

- The catalog entry and every site above.
- First turn on `--prompt`, persona folded into it, and resent on a fresh fallback.
- Session key reported by a Runner-owned OpenCode plugin loaded per spawn, for fresh, blank, forked and fallback sessions alike.
- Resume with `--session`, native fork with `--session <id> --fork`, and the conversation-exists `SELECT`.
- Permission modes Default and Bypass (`--auto`); mission Bypass ([#527](./archive/527-mission-permission-mode.md)) maps to `--auto`; chats assert nothing.
- `--model provider/model` and the default read from `opencode.json`.
- Settings → Agents row with version and `opencode upgrade`; Settings → MCP catalog client with a JSONC-preserving write; Skills pane roots; the Runner skill through `~/.agents/skills`.
- Placeholder mark, READMEs, `docs/arch`, the smoke checklist.

### Out of scope

- ACP, `serve`, `attach`, `web`, `--mini`, `--agent`, `--continue`, managing the user's plugins, and a provider or model picker; users can put flags in runner args.
- Hook status: the plugin reports session keys only, and status events are a follow-up, so OpenCode sessions use the terminal-activity baseline.
- AcceptEdits and Auto (decision 3).
- Model discovery through `opencode models` ([#590](https://github.com/yicheng47/runner/issues/590)) and effort.
- Installing or authenticating the CLI; the `.pen` mark and README screenshots.

## Decisions

1. **The key comes from a Runner-owned OpenCode plugin, through the shared drop watcher.** Runner installs `<app data>/opencode/runner-session-key.js` at startup, as it installs Copilot's plugin and pi's extension. Every OpenCode spawn, fresh or resumed, direct, mission or fork, loads it by setting `OPENCODE_CONFIG_CONTENT` to `{"plugin":["<absolute path>"]}` and `RUNNER_OPENCODE_REKEY_PATH` to `claude_rekey::drop_path(app data, runner session id)`. On every `session.created` without a `parentID`, the plugin writes `{"session_id":"ses_…"}` to a temporary file beside that path and renames it into place, as pi's extension does. The existing `ClaudeSessionKeyWatcher` picks the report up and calls `rekey_agent_session_key`, guarded by the row's start time and running status. Its key check accepts an OpenCode id (`ses_` followed by ASCII alphanumerics) only for a row whose runtime is OpenCode (the row's `agent_runtime`, else its role's); every other row keeps the UUID check. Ownership is per process: the plugin runs inside the OpenCode process Runner spawned, the path comes from that process's environment, and the probe showed each of two same-folder TUIs reporting only its own session. Every way a session starts reports it: a `--prompt` first turn, a blank chat's first typed message, a fork, a fresh start after a missing conversation, and `/new` in the TUI, which moves the key to the new session so a relaunch resumes where the user was. A plain resume reports nothing, so its key stands.
   - *Merging with the user's own value.* The value OpenCode would otherwise inherit is the role's env if it sets `OPENCODE_CONFIG_CONTENT`, else Runner's own process environment (portable-pty starts every child from it; `agent_env` holds only the proxy vars and the role's env). If that value is set, Runner parses it as JSONC and appends its plugin path to that object's `plugin` list instead of replacing it. If the value is not a JSON object, Runner leaves it untouched and injects nothing, so that session gets no key. If the plugin file is missing, Runner injects nothing either.
   - *Rejected:* matching the newest session in the cwd, or the cwd plus `time_created` — two sessions in one mission worktree landed 191 ms apart with nothing to say whose was whose, and an OpenCode run in a terminal in the same folder would be adopted. OpenCode's log — one shared file, tagged with a random run id rather than a pid. A marker appended to the first turn and found in `part.data` (the Codex technique; it works) — a blank chat has no first turn, the marker shows in the transcript, and a fork carries no message, so forks would need a content match that cannot tell two forks of one source apart.
   - This is the plugin channel the issue reserved for hook status, used for the key only; status stays out of v1 (decision 8).
2. **The database is read only to check that a conversation exists.** Before resuming, Runner runs `SELECT 1 FROM session WHERE id = ?1` on a fresh read-only connection per check. The database path is resolved like `Database.path()`: `OPENCODE_DB` from the role's env, else from Runner's process env (absolute, or relative to the data dir), else `<data>/opencode.db`, with `<data>` from `XDG_DATA_HOME` the same way, else `~/.local/share/opencode`. The connection opens normally when the `-wal` or `-shm` side file exists, so a live OpenCode's uncheckpointed writes are read. `immutable=1` is used only when both are absent, where the main file is complete. A missing database file or a missing row means "missing": Runner then spawns fresh with the persona as the first turn, and the plugin reports the new key. Any other open or query error means "exists", so a locked or unreadable database never throws away a resumable conversation; OpenCode's own "Session not found" is the backstop.
3. **Permission modes: Default and Bypass only.** Bypass is `--auto`, the documented flag; OpenCode applies it by answering every surfaced request, and explicit `deny` rules still deny. AcceptEdits stays hidden, though `OPENCODE_CONFIG_CONTENT` and `OPENCODE_PERMISSION` do merge, for four reasons from the probe: it is a no-op on OpenCode's `"*": "allow"` defaults; it is silently ineffective when the user's `permission` puts `"*"` after `edit`; it lifts the Plan agent's edit deny; and a mode carried in the environment has no argv spelling for `infer_permission_mode` to read back from the role's args. Auto has no equivalent. When Runner asserts a mode it strips all three spellings of the flag, as it does Copilot's aliases.
4. **Model without effort.** `--model <provider/model>` is forwarded verbatim; an invalid value makes OpenCode fall back with a toast, which is its own guard. No effort is sent, because the TUI takes no variant flag. `runtime_defaults` reports the global `model` as the default: it is what OpenCode starts with, absent `--model`, unless the user set an agent-level model, and it is already in the form `--model` takes. The catalog lists only Default; the model field is free text.
5. **Settings → MCP treats OpenCode's file as JSONC and splices only the entry it owns.** OpenCode is a catalog client, since #648 removed Runner's own registration. Runner reads and writes the file `opencode mcp add` would: `opencode.json` if it exists, else an existing `opencode.jsonc`, else a new `opencode.json`. The entry shapes are `mcp add`'s: stdio becomes `{"type":"local","command":[command, …args]}` plus `environment` when non-empty; HTTP becomes `{"type":"remote","url":…}` plus `headers` when non-empty. Reading maps back the same way. On a copy that updates an entry, keys Runner does not own (`enabled`, `timeout`, `oauth`) survive. The splicer stays one implementation, built on two offset-preserving copies of the whole document:
   - *scan*: `//` and `/* */` comments blanked to spaces (newlines kept), and every trailing comma in the document, at any depth, blanked too. A comma counts as trailing when the next non-blank byte is `}` or `]`. `json_members` and serde's `IgnoredAny` run on this copy, so nested comments and trailing commas never reach serde;
   - *commas*: comments blanked, commas kept. For each member, `json_members` records its comma from this copy, so a trailing comma after the last member is that member's comma. An insert then adds no second comma, and removing that member takes the comma with it; removing the only member of `{"a":1,}` leaves `{}`.
   Splices use those offsets on the original text, so every byte outside the replaced span survives: comments, key order, spacing, other servers and every other key. The one exception is the entry being rewritten: its value is replaced whole, so comments inside it are dropped, exactly as `opencode mcp add` (jsonc-parser `modify` on `["mcp", name]`) drops them. The detail editor's text is that entry alone, and it is parsed as JSONC too. Every output is re-parsed as JSONC before it is written; if it does not parse, the write fails and the file is left as it was. On strict JSON, which has no comments or trailing commas, both copies equal the original, so spans and output are byte-identical to today's, and Claude Code, Copilot and Antigravity writes do not change. Those clients' files are still parsed strictly, before and after a splice, so a file with comments that they would reject is neither listed nor written. Known limits: `$XDG_CONFIG_HOME` is not read (Runner resolves under `~/.config`, as it does the other clients' files), and a server defined only in `opencode.jsonc` while `opencode.json` also exists is not listed, as with `mcp add`.
6. **One JSONC reader.** `runtime_defaults::jsonc_document` today strips `//` comments only. It becomes "parse the *scan* copy of decision 5", the same blanking the MCP splicer uses, so OpenCode's config parses and Copilot's files, which only use `//`, parse as before.
7. **Sessions do not self-update.** Runner sets `OPENCODE_DISABLE_AUTOUPDATE=1` for every OpenCode spawn, the #475 rule the other runtimes follow (Copilot's `--no-auto-update`, pi's `PI_SKIP_VERSION_CHECK`): a background patch upgrade launched inside a mission slot would replace the binary under every other running session. The Update button in Settings → Agents runs `opencode upgrade` instead, which picks the install method itself, and the npm check reads `opencode-ai`. OpenCode run outside Runner keeps the user's own `autoupdate` setting.
8. **No hook status in v1.** OpenCode's plugin API also emits `session.status`, `permission.asked` and `question.asked`, which would give Working, Idle and Needs you. Reporting them from the same plugin is the follow-up; v1's plugin handles `session.created` and nothing else. OpenCode's title never carries a spinner, so the byte-activity baseline is the only status source.
9. **Placeholder mark.** `opencode.svg` maps to the existing `message-square` glyph, as `antigravity.svg` does, with every icon test extended. The real mark is designed on the canvas later.
10. **Enabled by default on macOS; Windows after a smoke.** OpenCode ships a native Windows binary (npm, scoop, choco; `opencode upgrade --method` lists them), and the npm install puts an `opencode.cmd` shim first, which Runner's batch fallback handles by pasting the first turn. The plugin and the drop watcher have no platform branch. None of this is verified on JASONPC.

## Open items

- Whether OpenCode's `--prompt` still submits when a provider setup dialog is open at launch, or waits until setup finishes. The key arrives whenever the session is created either way; the smoke records which.
- Whether a user's `opencode.json` `plugin` list that fails to load affects Runner's plugin. The probe only showed that a missing file is skipped silently.
- Picking an existing session with `/sessions` inside the TUI creates nothing, so the key stays on the session the pane started with. The follow-up plugin could report route changes if that matters.
- Windows: the database path (`%USERPROFILE%\.local\share\opencode\opencode.db`), the `.cmd` shim, ConPTY rendering, the plugin under OpenCode's Windows build, and whether `--auto` behaves the same.

## Implementation phases

### Phase 1 — adapter (`runner-backend`)

Enum, definition, argv, permission flags, key checks in the resume and fork plans, `session/opencode.rs` with the plugin, its environment and the database check, the drop watcher's key check, `agent_env`, the catalog, `runtime_defaults` with the wider JSONC reader, the title decoration, the CLI's `runtime_command`, and the test loops.

### Phase 2 — settings and identity (`runner-app`, `ops/mcp.rs`)

The MCP client with the JSONC splicer, the Runner skill root, the Skills caption, the Agents row, the placeholder mark and icon tests, the permission copy, docs, both READMEs and the smoke checklist.

### Phase 3 — smoke (Jason: macOS, then JASONPC)

`docs/tests/592-opencode-smoke.md`: the fixture `crates/runner-terminal/fixtures/opencode-first-turn.ndjson` from a live first turn and its render test, the wheel check, live key capture for a persona chat, a blank chat and a fork, resume after relaunch, a missing session, the two permission modes, a crew with two OpenCode slots in one worktree, Settings, and JASONPC.

### Phase 4 — follow-ups, each its own issue

Hook status from the same plugin; model discovery through `opencode models` (#590); the mark; Windows default-on after its smoke.

## Verification

Automated (no signed-in OpenCode needed):

- [ ] `Runtime::parse("opencode")` round-trips; `runtime_list` includes `opencode` with command `opencode`, `native_fork: true`, the three skill roots, `upgrade` and `opencode-ai`; `updatable(OpenCode)`.
- [ ] A fresh spawn carries `--model <id>` when set, never an effort flag, and `--prompt <body>`; a resume carries `--session <key>` and no `--prompt`; a fork carries `--session <source> --fork` and no key; every other runtime's argv is unchanged.
- [ ] Every OpenCode spawn's environment carries `OPENCODE_DISABLE_AUTOUPDATE=1`, `RUNNER_OPENCODE_REKEY_PATH` naming its own row's drop file, and `OPENCODE_CONFIG_CONTENT` listing the installed plugin; a user value that is a JSON or JSONC object keeps its keys and its own plugins, with Runner's appended, whether it comes from the role's env only, Runner's process env only, or both (the role's wins); a non-object value is left alone with no injection; no injection when the plugin file is missing; no other runtime gets these variables.
- [ ] The plugin, run under Node against a fake event stream, reports a top-level `session.created` id atomically to the path, ignores a `parentID` session and other event types, and does nothing without the env var.
- [ ] The drop watcher rekeys a running OpenCode row to an OpenCode id, rejects an OpenCode id for a Claude Code or pi row, and still rejects other shapes.
- [ ] Default rows have no flag; Bypass rows and mission Bypass carry `--auto`; the strip removes `--auto`, `--auto=false`, `--yolo`, `--dangerously-skip-permissions=true` and `--no-auto`; the role form offers Default and Bypass only.
- [ ] The conversation probe finds an existing id, reports a missing id and a missing database as missing, honours `OPENCODE_DB`, reads a WAL database without side files through `immutable=1`, and reads a live WAL database's uncheckpointed rows; a resume whose session is missing spawns fresh with the persona.
- [ ] Settings → MCP reads `mcp` from JSONC with comments and nested trailing commas; a copy writes `mcp add`'s shapes and keeps every byte outside the rewritten entry, including comments, other keys and `enabled`/`timeout`; inserting after a trailing comma, removing the last member, and removing the only member all leave valid JSONC; strict-JSON clients' outputs are byte-identical to before; HTTP copies work both ways.
- [ ] `runtime_defaults` reads `model` from JSONC and prefers `opencode.jsonc` over `opencode.json` over `config.json`.
- [ ] `OC | Fix the bug` becomes the tab title `Fix the bug`; `OpenCode` becomes nothing.
- [ ] Every runtime-enumerating test lists `opencode`; the icon tests map it to the placeholder.

Live (Jason's smoke): the first turn arrives once and the key lands on the row within seconds; a blank chat gets its key on the first message; relaunch resumes; a deleted session falls back fresh with a new key; a fork keeps history and gets its own key; two OpenCode slots in one worktree each get their own key; Bypass runs a shell command unasked; the fixture renders and the wheel scrolls OpenCode's transcript.
