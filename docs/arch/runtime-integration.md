# Adding an agent runtime

> Integration checklist, 2026-09-29. Based on the shipped runtime adapters, including Antigravity's initial integration and follow-ups. These priorities describe the requirements for a new integration, independently of its tracking issue's priority.

A runtime is supported when a person can use it in a chat and a crew can use it in a mission, with the same lifecycle and coordination guarantees as Runner's other agents. Starting its executable is only one part of that contract. This document covers an agent CLI such as `agy`; it does not describe adding another PTY implementation behind `SessionRuntime`, or adding a shell.

## Priority and release rules

| Priority | Meaning | Release rule |
| --- | --- | --- |
| **P0 — mandatory** | The runtime can do useful work, preserve conversation identity, and participate in Runner without breaking its core behavior. | All applicable P0 checks must pass before calling the runtime supported on a platform. A missing P0 is a blocker, not an ordinary follow-up. |
| **P1 — expected integration** | Runner understands more of the agent's state and configuration, reducing the need to leave the app. | Implement when the CLI exposes a usable mechanism. A first release may defer an item with a documented limitation and tracked follow-up. |
| **P2 — optional enhancement** | Additional convenience or provider-specific functionality. | Add when useful and supported by evidence; absence does not block runtime support. |

Priority does not make incorrect behavior acceptable. Hooks are P1, for example, but enabling hooks that deny tools or leave a finished session permanently Working is a correctness bug in the shipped integration. Any implemented capability must meet its acceptance criteria.

Record support separately for macOS and Windows. A CLI probe, passing Rust tests, and a native Runner smoke test are different evidence. Do not turn “builds on Windows” into “verified under ConPTY.” Existing integrations may have gaps; this checklist is not a claim that every current runtime already passes every row.

## P0 — mandatory

### P0.1 Identity, discovery, and selection

Register a stable runtime key, display name, default command, and catalog entry. The key must round-trip through persisted roles, slot overrides, direct-chat sessions, and the CLI/API. Preserve the existing handling of unknown and legacy runtime names.

Settings → Agents must detect the executable, support an explicit executable override, refresh discovery, and honor the enable switch. New chat, role creation, and crew slot overrides must agree on which runtime is selected and available. Launches must use the resolved command and existing environment pipeline, including login-shell PATH, proxy variables, role environment, and the app's bundled `runner` command.

**Accept:** launch a runtime-only chat, a role-backed chat, and a mission slot; repeat through an executable override. A missing executable or invalid override has a useful error, and disabling the agent removes it from new selections without corrupting saved rows.

### P0.2 Interactive terminal and process lifecycle

Run the real interactive CLI in Runner's PTY. Preserve cwd, terminal dimensions, Unicode input/output, ordinary keys, multiline and bracketed paste, resize, scroll behavior, and human takeover. Confirm alternate-screen and mouse modes from actual terminal output; do not infer them from another agent's renderer.

Start, interrupt, stop, restart, normal exit, and crash must use Runner's shared lifecycle. A stopped or crashed process must not remain live in the UI. Runtime-specific watchers must stop with their process, and temporary logs, hook feeds, or prompt files must follow their defined session cleanup policy. Keep the existing distinction between stopping a session and archiving its mission.

**Accept:** run a tool-using turn, interrupt it, send another turn, resize, switch tabs/panes, stop, and restart. Record a real terminal fixture and replay it. Verify wheel behavior in the native app: an alternate screen without mouse reporting can turn scrolling into history keys.

### P0.3 Persona, first turn, and startup readiness

Deliver the role persona, crew conventions, coordination instructions, and mission goal through channels the interactive CLI actually consumes. A native system-prompt mechanism is optional; a verified first-turn mechanism is sufficient. A flag that works only in print mode does not prove it works in the TUI.

Respect Runner's prompt composition: direct chats receive their role context without mission identity; workers receive their coordination context; leads receive the mission goal. Send the composed first turn once on a fresh spawn, including Restart or a resume that falls back to fresh. Do not replay it into a genuine conversation resume. A runtime with a separate system-prompt file may refresh that file on resume, as pi does.

**Accept:** verify what the agent actually received in each launch shape, including empty direct chats, non-ASCII text, multiline bodies, quoting, and long prompts. Exercise the existing delayed first-turn path where argv cannot carry the body, especially Windows `.cmd`/`.bat` wrappers. Startup dialogs must not consume or race the prompt.

### P0.4 Mission coordination

An agent must be able to invoke the bundled `runner` CLI and use the mission bus. Preserve slot identity and `RUNNER_*` environment, access to the mission directory outside the project, and the existing router's delivery behavior. Direct chats remain off-bus; a process label alone must not grant mission identity.

**Accept:** in a small crew, have the new runtime post/read a message, receive a routed message, signal the lead, and ask the human through Runner. Check delivery during startup, while the user has a draft, and after a stopped slot resumes. Run two instances in the same cwd to expose accidental shared state or identity collisions. This work does not require registering Runner as an MCP server: agents use the CLI.

### P0.5 Permissions and folder trust

Follow the current product policy: unattended missions use **Bypass**, while direct chats strip Runner-recognized permission flags from stored role args and let the CLI's own configuration govern approvals. Map the mission policy to the runtime's actual flags or native unattended behavior. Cover conflicting flags, aliases, and `--flag=value` forms supported by that parser; do not pass another runtime's permission or sandbox flags through an override.

Handle folder trust separately from tool permissions. If a trust dialog blocks unattended work, use the established trust-seeding pattern for the exact cwd, preserving unrelated user configuration. Suppress avoidable update or consent prompts at launch where the CLI supports it; document unavoidable behavior. A runtime without a workable unattended mode cannot pass the mission requirement.

**Accept:** a mission in a previously untrusted temporary project can read, write, run a command, and use the mission bus without a permission dialog. A direct chat retains the CLI's configured approval behavior. Repeat after Resume and with old role permission arguments present. A read-only command succeeding under Bypass does not prove edits and commands are unattended.

### P0.6 Model, effort, and override correctness

Honor the model and effort settings the integration exposes, including role defaults, per-chat selections, and crew slot overrides. “Default” must leave the choice to the CLI. A static catalog is enough for P0; live discovery is P1. If the CLI has no effort control, offer no invented mapping.

Only expose verified model/effort combinations. Some CLIs silently select a default model for an invalid pair, so successful process startup is insufficient evidence. Preserve the existing precedence rules and avoid carrying runtime-specific options across a runtime override.

**Accept:** check the model the agent actually selected, with Default and explicit settings, and through a slot override. Check supported and unsupported effort combinations. Resume must retain the effective session configuration according to Runner's existing rules.

### P0.7 Conversation identity and resume

Persist the agent's conversation key separately from Runner's session ID. Either assign a key the CLI accepts or capture it from a source tied to that process. Never guess from the newest global conversation or “last session in this cwd”; simultaneous slots can share a directory.

Track identity changes during a running process when native commands such as `/new`, `/clear`, `/fork`, or `/resume` replace the active conversation. Ignore stale reports from an earlier spawn. Handle lazy conversation creation, when a blank chat has no key until its first message.

Resume must open the recorded conversation without duplicating the cold-start prompt. Probe the runtime's conversation store with its actual cwd/home rules. If history is missing, follow Runner's established fresh-start or unavailable behavior for that entry point; never claim that a fresh conversation restored history. Update the key if the CLI itself falls back to a new conversation.

**Accept:** stop and resume, quit and relaunch Runner, switch conversations in the TUI and resume again, and remove only a disposable test conversation to exercise missing history. Check two concurrent sessions in one cwd and a late report from a previous process. A runtime with no reliable continuation mechanism does not meet this first-class runtime contract; fork is a separate P2 capability.

### P0.8 Honest status and capability boundaries

Wire process lifecycle and the existing terminal-activity baseline even if no semantic status adapter is available. Estimated activity is an acceptable minimum. Do not infer a successful response, approval request, or human question from output silence. Unsupported controls such as Fork must stay disabled or unavailable through both UI and backend.

**Accept:** the UI distinguishes a live process from a stopped/crashed one, and it does not advertise unsupported operations. With hooks absent or disabled, the runtime still works using the baseline. Any limitation in status precision is recorded explicitly.

### P0.9 Platform evidence and regression coverage

Declare the supported platforms and record the tested CLI version and date. For each claimed platform, cover discovery, paths containing spaces and non-ASCII characters, shell/shim quoting, first-turn delivery, terminal input, process exit, and resume in the native app. Windows needs a ConPTY run, not only a macOS test of Windows argument construction.

**Accept:** focused adapter/session tests, terminal fixture replay, and dated native smoke results exist. Update runtime support documentation, including `README.md` and `README.zh-CN.md` together when their support lists change. Keep unverified platforms or features labeled as such.

## P1 — expected integration

| Capability | Required behavior when implemented | Acceptance evidence / allowed limitation |
| --- | --- | --- |
| **Semantic session status** | Map native hooks/events to Working, Idle, surfaced approval/question waits, and turn outcomes only where evidence exists. Handle completion, errors, Esc/Ctrl+C interruption, later turns, and subagent events. Use per-session feeds and spawn generations; preserve user hooks and their permission behavior. | Test ordinary completion, a tool turn, failure, interrupt without a final hook, delayed events, stale generations, and bridge failure. A healthy hook-driven turn must not become Idle because output went quiet. If an event is unavailable, document the missing signal; retain the baseline on unsupported platforms. |
| **Approval/question delivery holds** | When the adapter can identify a prompt actually shown to the user, hold crew delivery until that interaction resolves. A pre-tool event or permission check alone is not evidence of a visible wait. | Exercise approve, deny, answer, dismiss, and interrupt. Verify that queued delivery resumes and does not answer a native dialog accidentally. If no surfaced-prompt event exists, do not synthesize Needs you. |
| **Live model catalog and native defaults** | Discover models and per-model efforts from a supported CLI/config source; read native defaults where their representation is understood. Use the effective executable and environment, bounded background probes, and the existing cache/static fallback. | Check refresh, offline/unauthenticated failure, empty or malformed output, and an override changed during a probe. Preserve saved model values; never guess how a display label maps to a CLI ID. |
| **Skills and Runner discovery skill** | Declare the agent's real personal skill roots for Settings → Skills. If it supports skills, install/reconcile the managed `runner` or `runner-dev` skill in a root it reads, honoring Runner's global switch and ownership markers. | Verify catalog/edit behavior, install, refresh, removal, and foreign-folder/symlink preservation in temporary homes. A runtime without native skill toggles gets a read/edit catalog, not a fake enable switch. Mission coordination must already work without this convenience. |
| **MCP settings integration** | Read and edit the agent's native global server entries through Settings → MCP where supported. Preserve unrelated entries, comments, formatting, and unmodelled keys. Add only transports whose native shape is verified. | Check missing/empty files, malformed config, copy/toggle/edit, and preservation of unrelated content. Unsupported transports are refused clearly. This is management of user MCP servers, not automatic registration of Runner itself. |
| **Provider identity and version** | Add a consistent provider mark across agent selectors, panes, sidebar, archived sessions, and settings; design it in Pencil before UI implementation. Show a version from the effective executable when a reliable probe exists. | Check both themes and existing identity tests. Version probes run off the UI thread with a timeout. Detection and launch must still work if a version probe fails. |

P1 can be partial when the CLI is partial. For example, Working/Idle hooks without an approval event are useful, provided the integration states that limitation and does not claim a full status vocabulary.

## P2 — optional enhancements

| Capability | Required behavior when implemented | Acceptance evidence / allowed limitation |
| --- | --- | --- |
| **Native fork** | Create an independent conversation and direct-chat row from the source through a verified native mechanism. Keep the source row, process, key, and conversation untouched; clean up failed fork attempts. | Prove context continuity and independent subsequent turns. An in-TUI `/fork` does not establish a callable native fork for Runner's button. Keep `native_fork: false` until the complete path works. |
| **Usage and quota** | Expose real account/provider windows, percent used, and reset times through the existing usage surfaces. Fetch in the background without triggering an agent turn; keep unavailable data distinct from zero use. | Check units, remaining-versus-used conversion, multiple model groups, auth/network failure, and popover layout. Do not invent equivalent windows across providers. |
| **Update management** | Offer a version comparison and Update action only when Runner knows the runtime's distribution channel and updater. Run updates through the existing unlisted modal process. | Verify override paths, failed updates, and refreshed versions. A runtime that updates itself may legitimately have no Update button. Preventing startup update prompts remains part of P0.5. |
| **Native titles and richer input** | Use native titles for display with provider decorations removed. Add verified image/attachment input or other native terminal conveniences where useful. | Missing titles retain Runner's naming. Check runtime-specific input behavior in the native app. A display title must not become status or routing evidence without a separately verified adapter. |
| **Provider-specific controls** | Expose features such as service tiers, extra thinking controls, or plugin management only when the product needs them and their semantics are verified. | Gate controls to the supporting runtime. For example, Codex speed settings are not generic agent options. Basic text input and model selection remain P0. |

Installing an agent, signing in, buying a subscription, and reproducing every vendor configuration screen are outside this checklist. Probe and document those prerequisites; runtime support does not require Runner to own them.

## Antigravity: lessons for the next runtime

These examples describe the implementation and recorded evidence through 2026-09-29, using the [initial Antigravity spec](../features/archive/644-antigravity-runtime.md) and the [follow-up validation record](../tests/747-antigravity-followups.md). They are examples of the contract, not a fresh certification of the installed CLI.

| Area | What Antigravity taught us | Tier |
| --- | --- | --- |
| First turn and trust | `-i` can send the first turn while a folder-trust dialog is still open. Seed the exact cwd before spawn; a successful response alone does not prove unattended startup works. | P0 |
| Conversation identity | Capturing only `Created conversation` misses later switches. The current watcher follows `Streaming conversation <id>` for the lifetime of the process, including `/new`, `/fork`, and `/resume`, with a guard against stale spawns. | P0 |
| Model and effort | Invalid pairs can silently select the default model. Group only proven aliases into effort choices; retain other discovered full IDs without attaching an unverified effort flag. Static correctness is P0; `agy models` discovery is P1. | P0 / P1 |
| Hook semantics | A `PreToolUse` reply of `{}` denied tools in the probe. Runner omits that hook. The available events report work, idle, and failure, but not surfaced approval/question waits. | P1, with mandatory correctness |
| Interrupt recovery | A tool-using turn could end on Esc without `Stop`, leaving Working stuck. The adapter now accounts for successfully sent interrupt input and ignores delayed completion events until a new invocation. | P1, with mandatory correctness |
| Skills and quota | Follow-ups added the managed skill under `.gemini/antigravity-cli/skills` and structured `/usage` data. These improve integration but are not substitutes for a working mission and resume path. | P1 / P2 |
| Fork and updates | In-TUI `/fork` does not provide Runner's native fork path. The catalog keeps fork disabled and offers no Update button for agy's self-managed update flow. | P2 |
| Platform evidence | The record includes Jason's macOS UI smoke report; JASONPC/ConPTY remains unverified there. macOS hook support does not imply Windows hook support. | P0 evidence / P1 parity |

## Where the implementation lives

Each agent owns its integration in [`crates/runner-backend/src/runtimes/<name>/`](../../crates/runner-backend/src/runtimes/). The [`RuntimeAdapter`](../../crates/runner-backend/src/runtimes/mod.rs) defaults unsupported capabilities to no operation; `NoAgent` supplies those defaults for Shell and unknown keys. Shared modules handle scheduling, process transport, config splicing and application state, and dispatch through the adapter.

| Concern | Runtime-owned implementation | Shared mechanism |
| --- | --- | --- |
| Identity and catalog | Each `runtimes/<name>/mod.rs` defines its catalog and capabilities; `runner-core/src/runtime.rs` keeps the variant, wire identity and managed skill root | `runtimes/mod.rs` selects the adapter; `ops/runtime.rs` exposes catalog entries |
| Native defaults, discovery and versions | `native_defaults`, `model_discovery`, `npm_dist_tag`; model commands and parsers live in `runtimes/<name>/models.rs` | `runtime_defaults.rs`, `runtime_status.rs`, `runtime_status/models.rs` and `runtime_status/versions.rs` handle reads, processes and caches |
| Prompt, permission, model, resume, fork and Speed argv | The adapter's permissions, prompt channels, launch args and resume/fork plans | `router/prompt.rs` composes content; `session/manager/spawn.rs` coordinates launches |
| Environment, trust, key capture and status | `launch_env`, `launch_gate`, `seed_trust`, `key_capture`, `status_hooks`, and the runtime's trust, capture and status modules | `session/launch.rs`, `session/hook_feed.rs`, `session/status.rs` and `session/pty_runtime.rs` provide transport and lifecycle machinery |
| Skills and MCP | `skills()` owns roots, flags, state and toggle writes; `mcp()` owns wire identity, config format and translation | `skills.rs` scans and parses; `ops/skills.rs` validates; `ops/mcp.rs` splices named entries; `agent_skill.rs` manages Runner-owned files |
| Usage | Each supported runtime's `usage.rs` owns commands, credentials and response parsing | `usage.rs` schedules refreshes and stores values/errors in a runtime map |
| App presentation and behavior | `runner-app/src/runtime_ui.rs` owns icons, tints, skill copy and usage labels; behavior reads catalog capabilities | `chat_icon.rs`, Settings, Start Chat, roles, crews and the usage surfaces render the catalog |
| Regression evidence | Runtime parser and adapter tests stay in their owning modules | `session/manager/tests/`, app tests, terminal fixtures and `docs/tests/` |

## Adding a runtime

1. Add a `runtimes/<name>/` module implementing `RuntimeAdapter`, using unsupported defaults for capabilities the CLI lacks. Keep runtime-specific discovery, usage, skills, MCP, hooks and argv there.
2. Add the `Runtime` variant and identity row in `runner-core/src/runtime.rs`, then register one adapter arm in `runtimes/mod.rs`.
3. Add one `runtime_ui` row and its icon asset. App behavior follows the catalog's capability fields.
4. Add focused adapter tests, parser fixtures and characterization expectations. Run workspace tests and platform checks; no shared dispatch, spawn or UI branch should need another runtime arm.

## Checklist for a runtime spec

Start with a capability inventory before implementation. Probe the actual CLI and record its version; do not copy an older runtime spec's flags or assumptions. The older [Copilot inventory](../features/archive/540-copilot-cli-runtime.md) is useful history, but some of its permission and MCP-registration behavior has since changed.

For each P0 subsection and each P1/P2 capability above, record:

| Field | What to write |
| --- | --- |
| Priority and capability | For example, P0.7 conversation identity and resume. |
| Native mechanism | Command/flag, event, file format, or “not exposed by the CLI,” with dated probe evidence. |
| macOS / Windows | Separate results: verified, implemented but unverified, pending implementation, or unsupported. |
| Acceptance evidence | Link tests, fixtures, and the native smoke record; say which layer each result covers. |
| Limitation / follow-up | State what users cannot do and link the follow-up when deferring P1/P2. An unmet P0 stays a blocker. |

Before release, run the relevant backend tests for adapter behavior, terminal replay tests for captured output/input, and app tests plus workspace Clippy for UI changes, following [`AGENTS.md`](../../AGENTS.md). Complete the platform smoke checks above and keep the validation record honest about what was automated, observed directly, or reported by the user.
