# 804 — Crew and role template import and export

> Tracking issue: [#804](https://github.com/yicheng47/runner/issues/804) (folds [#1334](https://github.com/yicheng47/runner/issues/1334) in-app Export/Import; supersedes [#1002](https://github.com/yicheng47/runner/issues/1002) offline export). Related: [#1333](https://github.com/yicheng47/runner/issues/1333) (curated crew samples page on `runnersh.dev`). Platforms: macOS, Windows and Linux.
> Status: signed off by Jason on 2026-11-27 (local `.toml` Stage 1, drag-and-drop upload modal, fresh role recreation, and interactive missing-runtime reassignment).
> Design: `runner/desktop/specs/804-crew-role-templates.pen` in `yeelinks_design`, revision `cd7c9f6`. Every screen is a full Runner window across three boards: `KUm7t` (entry points: Crews list `k7Ns4`, Roles list `nT1t8`, Crews empty state `FmDW8`, Crew page `IPZyP`, Role page `jpcxC`), `vIBqb` (complete 3-step import flow: Upload/Drop `T1xukv` → Import Preview `T91Gr` → Prompt Reader `J2DG3F`), and `g8ycMu` (edge states & Light mode: drag-over `yZWKa`, bulk + per-slot missing-runtime picker `ATvjj`, standalone role import `zLtkO`, recoverable validation error `yPEtJ`, and Runner Light preview `BgdON` & reader `Wlswz`).

## Motivation

Crews, roles and slots live only in Runner's local SQLite database, so people cannot back up a setup, share a working crew with a teammate offline, or start from a curated crew without recreating every role and prompt by hand. First-run setup (#1171) already offers one starter crew built from the detected agents; templates cover everything beyond that pair.

A versioned, self-contained template file solves local backup and offline sharing, and gives `runnersh.dev` (#1333) — and later a catalog on `runner_server` — one canonical format for downloadable crew and role templates. The prompt folders under `examples/` become real template files.

## Decisions

Settled with Jason on 2026-11-24:

1. **TOML v1 format.** A template is one self-contained `.toml` file (`version = 1`, `kind = "crew" | "role"`), named `<slug>.crew.toml` or `<slug>.role.toml`. Multiline Markdown prompts stay readable and diffable.
2. **Stage split.** Stage 1 (this spec) owns the template format, offline local-file CLI and desktop UI, an in-app link to downloadable sample crews on `runnersh.dev`, and converting `examples/` to `.crew.toml`. Stage 2 (#1333) publishes downloadable sample `.toml` templates on `runnersh.dev`. `runner://` deep links can layer on later without changing the format or preview.
3. **Never overwrite existing roles; leave out machine-local state.** Export omits ids, timestamps, `working_dir`, `env`, `command`, raw `args` and the retired `lead`. Import never overwrites an existing role and never starts a mission.

Settled on 2026-11-25 and revised with Jason on 2026-11-27:

4. **One centered modal for the whole import flow** (`vIBqb`): clicking **Import…** opens Step 1 (drag-and-drop `.crew.toml` / `.role.toml` zone + **Choose file…** + link to `runnersh.dev/crews`), transitions in place to Step 2 (Import Preview), and swaps the modal body for Step 3 (Prompt Reader, `J2DG3F`).
5. **Local files only in Stage 1; defer `runner://` links and URL fetching (2026-11-27).** People download `.crew.toml` or `.role.toml` files (from `runnersh.dev` or a teammate) and drop or pick them offline. Runner links to `https://runnersh.dev/crews` from the Crews page, empty state, and Step 1 upload modal. Dropping `runner://` and HTTP fetching removes OS URL-scheme registration, Windows single-instance pipe changes, and all network code in Stage 1, giving macOS, Windows and Linux full parity.
6. **Always recreate imported roles; no conflict/reuse detection (2026-11-27).** Every imported role is created fresh — Runner never compares against or reuses existing local roles. Because role handles are globally unique in SQLite (`roles.handle UNIQUE`), a handle that already exists locally simply auto-suffixes to the next free `<handle>-2`, `-3`, … (editable on standalone role import). This removes conflict toggles, role-equality matching and changed-plan races completely.
7. **Check runtime availability and let the user pick an installed runtime (2026-11-27).** The only environment check during import is whether each suggested runtime is installed on this machine. When a template suggests a runtime the user does not have installed, the preview highlights it and offers a picker of their installed runtimes (plus a bulk switch when 2+ roles share the same missing runtime, and a **Keep \<runtime\> (not installed)** escape hatch). Switching a role or slot to a different runtime clears its template `model`, `effort` and `codex_speed` so the chosen runtime starts with its own defaults.
8. **Templates carry no permission mode.** A role's stored permission flags never reach a launch: chats strip them (`session/manager/direct_spawn.rs`) and missions replace them with broad access (`session/manager/mission_spawn.rs`; see [Missions](../guide/concepts/missions.md#permissions)). A Bypass warning or clamp on import would promise a protection Runner does not give. The preview says **Missions run with broad access** instead.

## Scope

### 1. Template format (`version = 1`, TOML)

Every template is a UTF-8 TOML document with a top-level `version = 1` integer and `kind = "crew"` or `kind = "role"`, so every import entry point knows what it holds without relying on the filename. Both formats share the exact same role table schema (`RoleTemplate`): a standalone role puts one table under `[role]`, while a crew puts one or more identical role tables under `[[roles]]` and adds `[crew]` plus `[[slots]]`.

Optional `[meta]` (`description`, `author`, `tags`, `homepage`) is shown in the Import Preview and used by `runnersh.dev` (#1333). It is not stored on `crews` or `roles` rows, so the schema needs no migration and a re-export has no `[meta]`.

#### Crew template (`kind = "crew"`)

```toml
version = 1
kind = "crew"

[meta]
description = "Three-agent implement, review, and live QA loop on Codex."
author = "Runner"
tags = ["coding", "review", "qa"]

[crew]
name = "Codex Trio"
conventions = """
This crew works as a three-slot delivery loop...
"""

[[slots]]
handle = "coder"
role = "impl-codex"
# Optional per-slot overrides (omitted when inheriting from the role):
# runtime = "claude-code"
# model = "opus"
# effort = "high"
# codex_speed = "fast"

[[slots]]
handle = "reviewer"
role = "review-codex"

[[roles]]
handle = "impl-codex"
display_name = "Implementer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
# codex_speed = "fast"        # Codex only: "standard" | "fast"
system_prompt = """
You are an implementer...
"""

[[roles]]
handle = "review-codex"
display_name = "Reviewer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
system_prompt = """
You own critique only...
"""
```

#### Standalone role template (`kind = "role"`)

```toml
version = 1
kind = "role"

[meta]
description = "Strict code reviewer inspecting working-tree diffs."
author = "Runner"
tags = ["review"]

[role]
handle = "review-codex"
display_name = "Reviewer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
system_prompt = """
You own critique only...
"""
```

#### Serialization and validation

- **Unknown keys are rejected** everywhere except inside `[meta]`, where they are ignored. A typo such as `effrot` fails instead of being dropped, and so do `id`, timestamps, `working_dir`, `env`, `command`, `args`, `lead` and `permission_mode`; the message names the key and its role or slot.
- **Export validates its own output** with the import validator and refuses with the offending handle or field, so every exported file imports. This covers older rows, such as a slot handle saved before handles were checked: the daemon's slot create and update only trim them today.
- **Readable output.** Multiline `system_prompt` and `conventions` are written as multiline TOML strings, empty optional fields are omitted, and `[[roles]]` follows first-referenced slot order.
- **Version.** Any `version` other than `1` fails with `Unsupported template version <n>; update Runner to import this file.`
- **Size.** A template file is at most 256 KiB, and each `system_prompt` at most `MAX_SYSTEM_PROMPT_BYTES` (16 KiB, the limit role create enforces). The prompt limit moves from `runner-daemon` to `runner-core` so both use one constant.
- **Structure.**
  - `kind = "crew"` requires a non-empty `crew.name`, forbids `[role]`, and allows no slots and no roles so an empty crew round-trips. Every `slots[].role` matches one `roles[].handle`, every role is used by a slot, and slot handles are unique.
  - `kind = "role"` requires `[role]` and forbids `[crew]`, `[[slots]]` and `[[roles]]`.
  - Role and slot handles follow `runner_core::slot_handle::validate_slot_handle` and may not be empty; `display_name` may not be empty.
- **Runtimes.** Every role and slot `runtime` must be a built-in agent runtime; `shell` and unknown names are rejected with the role or slot named. `codex_speed` is accepted only when the effective runtime supports it. `model` and `effort` are free text, as in the role form, and are not checked against a catalog. Import takes `command` from the runtime registry and creates every role with `PermissionMode::Default`, as the New role page does.

### 2. Fresh role creation, runtime reassignment and atomic import

Role handles are globally unique (`roles.handle UNIQUE`); crew names are not.

- **Always create fresh roles:** importing a crew or standalone role never reuses or overwrites any existing local role.
- **Auto-suffixed handles:** for each imported role, if `@<handle>` is free, use it; otherwise pick the first free `<handle>-2`, `-3`, … (shortening the base so the result stays within 32 characters). The search covers both local roles and other roles in the same template. For standalone role import, the user can also edit the handle before confirming (validated for uniqueness).
- **Crew names:** when a local crew already has the same name, the preview pre-fills `<name> (2)`, editable before confirming; the CLI does the same unless `--name` is given.
- **Runtime availability check & reassignment:**
  - The preview checks each slot's effective runtime (`slot.runtime` if set, otherwise its `role.runtime`) and standalone role's `runtime` against the installed built-in runtimes detected on this machine.
  - If that effective runtime is installed, the slot's optional `runtime`, `model`, `effort` and `codex_speed` overrides import as written for lossless round-trips.
  - If that effective runtime is not installed, the user can choose any installed built-in runtime for that slot/role (or switch all slots sharing that missing runtime at once in the UI, or keep the template's runtime to install later).
  - When the user switches a slot or role to a different runtime, `model`, `effort` and `codex_speed` overrides on that slot (and on its role if the role's runtime changed) are cleared to `None` so the chosen runtime uses its own defaults. If multiple slots share one role and pick different runtimes, the created role takes the first chosen runtime and differing slots store a `runtime_override` with cleared model, effort and speed.
- **Atomic writes:** `template_import` runs in one immediate SQLite transaction, following `ops/onboarding.rs`'s starter crew. If a handle became taken between preview and confirm on a crew import, the transaction advances to the next free suffix automatically so the import still succeeds cleanly. Any failure rolls back, so no role, crew or slot is left behind.

### 3. Core, daemon and CLI

Parsing, validation, serialization, handle allocation and runtime-override normalization live in `crates/runner-core/src/template/`, which gains a `toml_edit` dependency (already used by the daemon and the CLI). Neither `runner-core`, `runner-daemon` nor `runner-cli` makes any network requests for templates.

#### Daemon requests

- `template_export_crew { id }` and `template_export_role { id }` return `{ filename, toml }`.
- `template_preview { toml }` returns the normalized template, the crew name suggestion, the fresh handle planned for each role, runtime install status per role/slot, the list of installed built-in agent runtimes on this machine, and any validation error with its line when known.
- `template_import { toml, crew_name?, role_handle?, runtime_overrides }` creates the crew and/or roles fresh in one transaction, applying any chosen `runtime_overrides` (clearing `model`/`effort`/`codex_speed` where the runtime changed), and returns the created crew/roles. It emits `role/changed`, `crew/changed` and `slot/changed` after commit, as the existing create requests do, so every window refreshes.

These requests change the client protocol, so Phase 1 takes `core-review`.

#### CLI

- `runner crew export <crew> [-o <path>] [--json]` prints TOML, or writes it atomically to `<path>`. `--json` returns `{ filename, toml }`, or `{ path }` with `-o`.
- `runner crew import <file> [--name <name>] [--map-runtime <from>=<to>]... [--dry-run] [--json]` reads a local file offline, creates all roles fresh (auto-suffixing handles if taken), and maps any specified runtimes (resetting `model`/`effort`/`codex_speed` on mapped roles/slots). Prints a warning if any resulting runtime is not installed. `--dry-run` prints the plan and writes nothing.
- `runner role export <role> [-o <path>] [--json]` and `runner role import <file> [--handle <handle>] [--runtime <runtime>] [--dry-run] [--json]` follow the same rules for one role (when `--runtime` differs from the template's runtime, resets `model`/`effort`/`codex_speed`).
- Regenerate the CLI reference with `make guide-cli`.

### 4. Desktop app UI

Visual source of truth: the canvas above. All strings go through `crates/runner-app/i18n/` (`en.toml` and `zh-hans.toml`).

#### Entry points (`KUm7t`)

- **Crews list (`1A`, `k7Ns4`):** a secondary **Import…** button left of **New crew** in the header, and a quiet **Browse sample crews ↗** link in the subtitle row opening `https://runnersh.dev/crews` in the default browser. Both are hidden while viewing a remote host.
- **Crews empty state (`1C`, `FmDW8`):** **Import…** beside **+ New crew**, with **Browse sample crews on runnersh.dev ↗** below the buttons.
- **Crew row menu (`1A`) and Crew page (`1D`, `IPZyP`):** **Export…** in the row `⋯` menu above a divider before **Delete**. The Crew page's action row gains a `⋯` button with **Export…** and **Delete crew**.
- **Roles list (`1B`, `nT1t8`), row menu and Role page (`1E`, `jpcxC`):** the same **Import…** header button and **Export…** / **Delete role** menus.
- **Export file dialog:** Export uses `cx.prompt_for_new_path` with the suggested filename (`<slug>.crew.toml` or `<slug>.role.toml`) and shows a toast once written.

#### Step 1 — Upload / Drop modal (`2A`, `T1xukv`; drag-over `3A`)

- Clicking **Import…** on either the Crews or Roles page opens the centered **Import template** modal on Step 1.
- **Drop zone:** accepts dragged `.crew.toml` or `.role.toml` files from Finder/Explorer (`ExternalPaths`, using the same `$accent` drag-over highlight as `ui/file_drop.rs`) and has a **Choose file…** button (`cx.prompt_for_paths`). Either entry point accepts either template kind.
- **Samples callout:** a quiet row below the drop zone links to `runnersh.dev/crews` so users who clicked **Import…** first can download starter templates right away.
- Dropping or picking a file validates it locally via `template_preview` and transitions inside the same modal to Step 2 (or the recoverable error state if invalid).

#### Step 2 — Import Preview & Runtime Check (`2B`, `T91Gr`; bulk picker `3B`; standalone role `3C`)

- **Header:** crew mosaic or role avatar, title (`Import crew` or `Import role`), filename, template version, author (when present), a **Change file** button (re-opens file picker / Step 1), and close button.
- **Identity:** crew name, or role display name and `@handle` (auto-suffixed if taken), editable; `[meta].description` below. `homepage` shows as plain text.
- **Slots and roles, in order:**
  - Every role is created fresh, showing its planned handle (`New @coder`, `New @reviewer-2`), runtime mark, `Runtime · model · effort` (with amber dot for slot overrides), prompt size, and chevron. Opening any row opens its full prompt in Step 3.
  - **Missing runtime check:** when a role or slot requests a runtime that is not installed on this machine, that row shows an amber indicator (`<runtime> not installed`) and an inline runtime picker populated with the user's **installed runtimes** (plus **Keep \<runtime\> (not installed)** at the bottom). When 2+ roles share the same missing runtime, an amber banner above the list offers **Switch all \<n\> to \[runtime ▾\]** (`3B`). Switching a runtime clears that role's `model`, `effort` and `codex_speed` to defaults and notes this clearly.
  - If no agent runtimes are installed on the machine at all, the picker shows that none are installed yet and lets the user keep the template runtime and install later in **Settings → Agents**.
- **Team conventions,** when not empty, opens in the reader too.
- **Access note:** `Missions run with broad access. Read each prompt before you import.`
- **Footer:** `Creates 1 crew, 3 new roles` (or `Creates 1 new role`) beside **Cancel** and **Import crew** / **Import role**.
- **Standalone roles (`3C`):** display name + editable handle, runtime check card (with inline runtime picker when uninstalled), prompt excerpt with **Read full prompt ›**, broad access note, and footer.

#### Step 3 — Prompt reader (`2C`, `J2DG3F`; Light `3F`)

- Takes over the modal body: back arrow, role identity, **Source | Rendered**, line count and size, the scrolling text, and **Previous**, **Next** and **Back to import**, which step through every role and the conventions. Esc returns to Step 2; a second Esc closes the modal.
- **Source is the default** and shows the text exactly as it will be stored, with line numbers. Invisible and bidirectional control characters are drawn as visible markers. Rendered uses the unified Markdown renderer in its Compact look, which turns images into alt text and never loads them.

#### Validation error & post-import (`3D`)

- Malformed TOML, a non-template file, an unsupported version, an unknown or excluded key, an unknown runtime, or a file over the size limit stays inside the modal showing what failed, the filename and line when known, that nothing was created, an inline drop retry hint, and **Choose another file…** beside **Close**.
- After confirming import: closes the modal, shows a success toast, refreshes lists, and navigates directly to the newly created Crew or Role page.

### 5. Example crews (`examples/`)

Convert all seven folders under `examples/` (`codex-trio`, `dev-crew`, `docs-crew`, `pair-coding`, `tic-tac-toe`, `tomb-raid`, `werewolf`) from loose Markdown prompts to real template files: each folder becomes `examples/<slug>/<slug>.crew.toml` (replacing the per-role `.md` files and `team-conventions.md`) alongside its existing `examples/<slug>/goal.md`, since a crew template carries no mission goal. Multi-player crews (`tic-tac-toe`, `werewolf`, `tomb-raid`) define one shared role in `[[roles]]` and instantiate multiple `[[slots]]` referencing it (for example `player-1`, `player-2`).

A `runner-core` test validates every `examples/**/*.crew.toml`, and Phase 3 updates the `examples/` links in `docs/guide/guides/build-a-crew.md` and `docs/guide/zh-CN/guides/build-a-crew.md` to point to the `.crew.toml` files.

The Markdown holds prompts only, so Phase 3 chooses a default runtime for each role: Codex for every role in `codex-trio`; elsewhere Claude Code for the roles that produce work and Codex for the roles that review it. Examples omit `model` and `effort` so they do not age.

## Non-goals

- `runner://` URL scheme registration, OS single-instance URL forwarding, and remote URL fetching in the app or CLI (deferred to a follow-up after #1333 is live).
- Role conflict detection, role-equality matching, or reusing existing local roles on import (every imported role is recreated fresh).
- Publishing, hosting, ratings, search or accounts for a marketplace on `runner_server`.
- The samples page itself on `runnersh.dev` (#1333).
- Permission settings in templates, or any change to how chats and missions choose permissions.
- Editing prompts or per-slot overrides beyond missing-runtime reassignment inside the preview; people edit on the Crew or Role page after import.
- Exporting or importing `env`, `working_dir`, custom `command` or raw `args`.
- Import or export on a remote host.
- Starting a mission or installing an agent CLI on import.

## Implementation phases

Three pull requests to `main`; this spec and its index entry land with Phase 1.

1. **Format, daemon requests and CLI** (`runner-core`, `runner-daemon`, `runner-cli`): the `template` module, handle allocator, runtime-override normalization, transaction, protocol requests, CLI commands, CLI reference and tests. `core-review`.
2. **Desktop UI and guides** (`runner-app`, `docs/guide/`): entry points and `runnersh.dev` sample-crews links, Step 1 drag-and-drop upload modal, Step 2 preview with missing-runtime picker & bulk switch, Step 3 prompt reader, recoverable error state, i18n, and guide pages.
3. **Examples:** convert `examples/` with the validation test, ready for downloadable sample templates on `runnersh.dev` (#1333).

## Guide pages

English and `zh-CN`, in Phase 2 (the CLI reference in Phase 1):

- [Crews](../guide/concepts/crews.md): exporting and importing crews.
- [Roles](../guide/concepts/roles.md): exporting and importing one role.
- [Build a crew](../guide/guides/build-a-crew.md): starting from a downloaded or shared template file, and sharing a crew.
- [CLI reference](../guide/reference/cli.md).

## Verification

### Automated tests

- **Format (`runner-core`):** crew and role round-trips preserve order, overrides, runtime, model, effort, Codex speed, conventions and prompts; golden TOML snapshots; an exported file holds no ids, timestamps, env values or paths.
- **Validation (`runner-core`):** rejects other versions, mismatched sections, unknown keys (including every excluded field and `permission_mode`), duplicate or invalid handles, dangling or unused roles, unknown runtimes and `shell`, Codex speed on another runtime, and oversized files and prompts. Unknown `[meta]` keys pass. Export refuses a row that would not import.
- **Import & runtime mapping (`runner-daemon`):** always creates fresh roles even when an identical local role exists; auto-suffixes colliding handles (including 32-character handles and two template roles competing for one suffix); crew name `(2)` suffix; runtime reassignment clears `model`, `effort` and `codex_speed` on switched roles/slots; rollback leaves no rows; emits `role/changed`, `crew/changed` and `slot/changed`.
- **CLI (`runner-cli`):** export to stdout, `-o` and `--json`; import from a local file, `--dry-run`, `--map-runtime`, `--runtime`, `--name`, `--handle` and exit codes, against a real daemon with no network access.
- **App (`runner-app`):** entry points and sample-crews links on local hosts and hidden on remote hosts; Step 1 modal file drop (`ExternalPaths`) and **Choose file…** picker; Step 2 fresh-role creation badges, missing-runtime inline picker, bulk switch banner, and model/effort reset display; Step 3 reader navigation and Source default with control characters marked; recoverable validation error with **Choose another file…**; post-import navigation. Tests that run daemon work wait with `h.wait(…)` and pass 20 runs with `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`.

### Live checks

Phases 1 and 3 have no app UI to operate and need none. Phase 2 needs these, on the development app with `script/qa-lock` and disposable roles and crews:

1. **Upload, drag-and-drop and crew round-trip.** Export a two-slot crew with an override and conventions, then click **Import…** to open Step 1: test both dragging the `.crew.toml` file onto the drop zone (verifying the active drop highlight) and picking it via **Choose file…**. Confirm all roles are created fresh (auto-suffixed if the original roles still exist) and the imported crew matches.
2. **Missing runtime reassignment (crew & standalone role).** Import a crew template and a standalone `.role.toml` targeting a runtime not installed on the test machine: verify the amber missing-runtime indicator, bulk switch banner (when 2+ slots share the missing runtime), per-row dropdown listing installed runtimes, model/effort reset to defaults on switch, and clean import onto the new Crew/Role page.
3. **Reader.** Open each prompt and the conventions in Step 3, switch Source and Rendered, step with Previous and Next, and return with Esc. A fixture prompt with a masked link, an image and a zero-width character shows all three in Source and loads no image.
4. **Menus, sample-crews links and themes.** Verify `⋯` menus on the Crew and Role pages, **Browse sample crews** links on the Crews list, empty state, and Step 1 upload modal, and visual clarity of Step 1–3 in Dark and Runner Light. Remote hiding is covered by the automated test.
5. **Recoverable rejected files.** Drop or pick a template with an unknown runtime, one with `env` or `permission_mode`, and a non-TOML file: each shows the in-modal error with filename/line, creates nothing, and recovers cleanly via **Choose another file…** or dropping a valid template.

Selected regression cases: `MIS-CREW-01` (a mission starts from an imported crew) and `CLI-READ-01` (CLI readback agrees with the imported rows).
