# 1285 — Custom launch flags per agent

Tracking issue: [#1285](https://github.com/yicheng47/runner/issues/1285).

## Design decision

Jason chose option A, one **Custom flags** field per agent shared by chats and missions, on 2026-11-14, and simplified its checks to three rules with one message each, showing only the first problem. Design: `runner/desktop/specs/1285-custom-launch-flags.pen` in the private `YeelinksCo/yeelinks_design` repo, revision `cc9a437`. Frames: Codex chat command `k5Euq`, Codex mission command `t0WeZ6`, anatomy and behavior `Dhmkj`, field states `PflQ5`, decisions `iVpnH`. The flag-and-value rows (B) and separate chat and mission flags (C) were dropped; git history of the design repo keeps them. Jason also considered making the Command preview editable and rejected it: the preview mixes Runner's lines with placeholders, so editing it would suggest those lines can change.

## Motivation

Settings → Agents → Launch offers a few switches and a read-only Command preview. People need other flags, such as Codex's `--search` or an extra `--add-dir`, without creating a role for each and without editing the agent's own configuration.

## Behavior

### The field

Every agent page's Launch card gets a **Custom flags** row after the switches and before Command, divided by hairlines like the other rows. Agents without switches (Cursor, TRAE) show Custom flags and Command only. The row has the title, a **Chats and missions** badge, a description naming the agent ("Added to every Codex launch, including role sessions, resumes and forks. Quote a value that contains spaces, as in a shell."), a one-line field, and a message line that appears only when there is something to say. The field uses the monospace UI font at 12 px on the page background with a 1 px border and 8 px radius, like Executable; long text scrolls inside it. Its placeholder is an untranslated example for the agent, such as `e.g. --search -c model_verbosity=high` for Codex.

Enter or leaving the field saves, as Executable does; Escape restores the saved flags. There is no Save button. Clearing the field removes the flags. The field shows the flags exactly as they were typed.

### Format

The text is split into shell words on every platform: spaces separate flags, and single quotes, double quotes or a backslash keep a value together. A pasted newline counts as a space. Nothing is expanded: `~` and `$VAR` reach the agent as typed, because Runner starts agents without a shell.

### Checks and messages

Runner checks a value when it is saved and shows the first problem only, in this order.

| Rule | Message | Saved |
| --- | --- | --- |
| A quote isn't closed, or a backslash ends the text. | **A quote isn't closed.** | No |
| A flag Runner already sets for this agent: anything that can appear in its Command preview in chats or missions, with any switch on or off, and every permission flag of the agent. For Codex's `-c`/`--config`, the key decides (`-c features.remote_plugin=…` is refused, `-c model_verbosity=…` is not). `--add-dir` may repeat. | **Runner already sets --model.** (naming the flag as typed) | No |
| A flag name the agent's `--help` doesn't list. | **codex --help doesn't list --serch.** (naming the agent's command and the flag) | Yes, with a warning |

A refused value keeps the draft in the field with a red border and red message; the saved flags stay in effect and the preview keeps showing them. A warning is amber with the normal border and never blocks: plugins and hidden flags can be missing from `--help`. A value passes when it parses, sets nothing Runner sets, and every flag name is listed; then no message shows.

Only tokens starting with `-` are checked, by the name before any `=`. Values, positional words, short forms of Runner's flags (such as `-m` for `--model`) and a role passing the same flag are not checked. A launch those break fails with the agent's own error in the session, as a bad role argument does today.

To read `--help`, Runner runs `<effective executable> --help`, nothing else, with the command runner, environment and timeout of the version probe, and keeps the flag names (tokens shaped `-x` or `--name`) for that executable path and version. On 2026-11-14 this took under 0.5 s for each of the eight installed agents (Codex, Claude Code, Antigravity CLI, pi, Copilot, Cursor, Grok, OpenCode). If the help cannot be read, there is simply no warning. The warning for saved flags is computed again when the Launch card loads, so an agent update can add or clear it.

### Where the flags apply

Custom flags apply to every session of the agent from its next start: plain and role chats, missions, resumes and forks, on every launch path that composes agent arguments (new chat, resume, fork, mission member). A running session keeps its flags. They do not apply to Runner's own helper commands, such as the version and help probes, model discovery, usage reads or headless fork steps.

The order is:

```
<command> [resume subcommand] <custom flags> <role arguments, with Bypass> [resume flags] [mission folder] <Runner's flags> <first message>
```

Custom flags come before the role's arguments, so a CLI that takes the last value of a flag uses the role's. Permission handling is unchanged: chats still strip stored role permission flags and Bypass still adds its flags; custom flags never carry permission flags because the field refuses them.

### Preview and Copy

The Command preview shows the saved custom flags as one line right after the executable, re-quoted from the parsed words, with an accent note **Custom flags** like a switch's lines. The line appears in both the Chat and Mission tabs and is absent when the field is empty. Copy includes it. The Launch caption becomes "Missions always start in Bypass. A role's own arguments come after custom flags. Changes take effect when a session next starts."

## Out of scope

Overriding or removing flags Runner generates; separate chat and mission flags; per-flag rows; an editable preview; checking values, short forms or role duplicates; expanding `~` or variables; changing the role editor's Args field, which still splits on spaces.

## Implementation

1. **Storage and protocol.** Add the typed text to the per-runtime launch options stored under the existing `runtime_launch_options` app-state key, with a serde default so old values load empty; no migration. `RuntimeLaunchOptions` is `Copy` today, so either drop `Copy` and borrow at call sites or keep the text beside it; choose the smaller diff. Add a request that saves the text for a runtime and returns the refusal message without saving, or the updated status with an optional warning. Make the warning for saved flags available when the Launch card loads, for example alongside the command preview it already requests. The daemon owns parsing and every check; the app only displays the message.
2. **Parsing and checks.** Parse with a shell-words parser (`shlex` is already in `Cargo.lock`; add it as a direct dependency) and re-quote for the preview with the same crate. Derive the reserved set from what Runner actually composes, not a hand list: the preview argv for chat and mission with each switch on and off, the adapter's resume and fork arguments and its permission strip flags, plus Codex's reserved `-c` keys. Add a unit test per runtime that fails if a flag Runner composes is not reserved. Read and cache `--help` beside the version probe in `runtime_status`.
3. **Launch.** In `compose_args` (`session/manager/spawn_spec.rs`), insert the parsed custom flags after a prepended resume subcommand and before the role's arguments. `apply_runtime_args` already serves direct spawn, resume, fork and mission spawn, so every session path picks them up; read the flags where it reads the launch options. Use the same insertion in `command_preview`, with a new `RuntimeCommandNote` variant for the **Custom flags** line, so the preview, Copy and the real argv come from one composition.
4. **UI.** Add the row in `surfaces/settings/agents/launch.rs`, reusing the Executable field's commit-on-Enter/blur and Escape handling and `FieldValidation`, with the warning shown in the warning tone. Add English and Chinese strings, including the updated caption.

Starting points: `crates/runner-core/src/protocol/runtime.rs`, `crates/runner-core/src/protocol/api.rs`, `crates/runner-daemon/src/db/app_state.rs`, `crates/runner-daemon/src/ops/runtime.rs`, `crates/runner-daemon/src/runtime_status/versions.rs`, `crates/runner-daemon/src/runtimes/mod.rs`, `crates/runner-daemon/src/session/manager/spawn_spec.rs`, `crates/runner-app/src/surfaces/settings/agents/{launch,mod}.rs`, `crates/runner-app/i18n/{en,zh-hans}.toml`.

Guide pages: `docs/guide/reference/supported-agents.md` and `docs/guide/zh-CN/reference/supported-agents.md`, in Launch options: the field, format, the three checks, the order relative to role arguments, and where the flags apply.

The change touches launch composition in `session/manager/`, so expect the automatic `core-review` label. Expected risk is medium. It is not `frontend-only`: the protocol, storage and daemon launch change.

## Verification

### Automated

- Parsing: plain words, single and double quotes, backslash escapes, a value with spaces, empty text, a pasted newline, an unclosed quote and a trailing backslash.
- Reserved flags for every runtime: each flag Runner composes in chat and mission with each switch combination is refused; permission flags are refused with Bypass on and off; Codex `-c` reserved keys are refused and other keys accepted; `--add-dir` is accepted.
- The help parser on captured `--help` output from each runtime, an unlisted flag producing the warning, `--name=value` checked by name, and an unreadable help producing no warning.
- Composition: the custom flags' position for each runtime in new chat, role chat, resume (including Codex's prepended subcommand), fork and mission member; the preview lines and Copy text matching the composed argv; old stored launch options loading with no flags.
- Save request: a refusal leaves the stored value unchanged; a warning still saves.

Run the `runner-core`, `runner-daemon` and `runner-app` tests, workspace Clippy, the updater-feature Clippy pass on macOS, formatting and the normal required CI.

### Live checks

This changes what a person sees and operates in Settings and the argv of launched sessions, so it needs live checks and a claude trio. They run in the development app on a reviewed candidate, with an inexpensive model, low effort and tiny prompts in fresh folders under `/tmp`. Record the agent's saved custom flags first and restore them afterwards. Prove argv with `ps -o args= -p <pid>` (macOS) or the process command line (Windows), compared with the copied preview text.

| ID | Check and reason | Platform |
| --- | --- | --- |
| CLF-01 | Codex: enter `--search -c model_verbosity=low --add-dir "/tmp/runner qa/extra dir"`, press Enter, leave the agent page and return, then restart the app. The field shows the text as typed; Chat and Mission previews show one Custom flags line after `codex`; Copy includes it. Covers saving, reopening and the preview. | mac |
| CLF-02 | Edit the field and press Escape, then edit and click elsewhere. Escape restores the saved flags; leaving the field saves. Covers the commit behavior. | mac |
| CLF-03 | Enter an unclosed quote, then `--model gpt-x`, then `-c features.remote_plugin=true`. Each shows its red message, nothing is saved (reopen shows the CLF-01 flags) and the preview is unchanged. Covers the refusals. | mac |
| CLF-04 | Enter `--serch`. The amber warning names `codex --help` and `--serch`, the value is saved and the preview shows it. Restore the CLF-01 flags. Covers the warning. | mac |
| CLF-05 | Start a Codex chat, then stop and resume it, then fork it. Each session process's argv has the custom flags, with `"/tmp/runner qa/extra dir"` as one argument, after `codex` (after `resume <id>` for the resumed chat and the fork, which opens as a resume of its new conversation) and before the Bypass and Runner flags. The fork's headless `codex exec fork` step does not get them. Covers the launch paths and quoting. | mac |
| CLF-06 | Start a one-slot mission with a Codex role. The member's argv has the custom flags before the mission's permission flags, and both `--add-dir` arguments are present. Covers missions. | mac |
| CLF-07 | Claude Code: save `--add-dir "/tmp/runner qa/extra dir"` and start a chat. Argv matches the preview and the session starts. Covers a second runtime with a different flag parser. | mac |
| CLF-08 | Windows, with Codex installed as an npm `.cmd` shim: repeat CLF-01's flags and start a chat. Argv keeps the spaced value as one argument and the session starts. Covers the batch-script quoting path, which has not carried values with spaces before. | win |

Unchanged switches, account selection, IME, authentication and session lifecycle are outside scope. Record the run in `docs/tests/runs/1285-custom-launch-flags-<platform>.md`.
