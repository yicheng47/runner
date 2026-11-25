# 804 — Crew and role template import and export

> Tracking issue: [#804](https://github.com/yicheng47/runner/issues/804) (folds [#1334](https://github.com/yicheng47/runner/issues/1334) in-app Export/Import; supersedes [#1002](https://github.com/yicheng47/runner/issues/1002) offline export). Related: [#1333](https://github.com/yicheng47/runner/issues/1333) (curated crew samples page on `runnersh.dev`). Platforms: macOS, Windows and Linux.
> Status: draft spec — TOML v1 template format agreed on 2026-11-24; desktop Import Preview layout and policy choices below await Jason's canvas sign-off before implementation begins.
> Design: `runner/desktop/specs/804-crew-role-templates.pen` in `yeelinks_design`, revision `e937105`. Boards: `KUm7t` (Board 1 — Entry points across Crews/Roles lists, empty states, and detail-page menus), `vIBqb` (Board 2 — Import Preview Option A centered modal `T91Gr` vs Option B two-column review page `j9Xj0`), and `g8ycMu` (Board 3 — Standalone role import `yauFl`, external URL / Bypass permission / missing runtime warnings `Loy8J`, fetching and validation error states `a3Eb2`, and Runner Light check `sY6Y6`).

## Motivation

Crews, roles, and slots currently live only in Runner's local SQLite database, so users cannot back up a setup, share a working crew with a teammate offline, or start from curated multi-agent crews without recreating every role and prompt by hand. First-run onboarding (#1171) detects agents cleanly, but stops short of giving a new user a ready-to-run crew.

A versioned, self-contained template file solves local backup and offline sharing immediately and gives `runnersh.dev` (#1333) — and later a catalog on `runner_server` — a single canonical format for one-click crew and role imports. The prompt folders under `examples/` become real template files that users can import directly.

## Decisions and open sign-off items

Settled with Jason on 2026-11-24:

1. **TOML v1 format.** Templates use a single self-contained `.toml` file (`version = 1`, `kind = "crew" | "role"`), with suggested filenames `<slug>.crew.toml` and `<slug>.role.toml`. Multiline Markdown prompts stay readable and git-diffable without escaping, and `toml`/`toml_edit` plus tree-sitter TOML highlighting already live in the workspace.
2. **Stage split.** Stage 1 owns the template format, CLI, and desktop import/export UI (including `runner://` handoff readiness); Stage 2 publishes sample crews on `runnersh.dev` (#1333) that import into Runner through this flow.
3. **Never overwrite existing roles; strip machine-local state.** Export omits IDs, timestamps, `working_dir`, `env`, `command`, raw `args`, and retired `lead`. Import never overwrites an existing role handle and never starts a mission automatically.

Open items awaiting sign-off from `runner/desktop/specs/804-crew-role-templates.pen` before implementation starts:

1. **Import Preview surface (Board 2 `vIBqb`):** **Option A (`T91Gr`, recommended)** — centered 640 px modal dialog that leaves active chats and missions undisturbed when a `runner://` link wakes Runner, vs **Option B (`j9Xj0`)** — full-page two-column review screen (`Crews › Import crew`) matching the `New crew` page with wider Markdown prompt reading.
2. **`runner://import?url=…` host policy:** Allow **any public HTTPS URL** (recommended — blocks loopback/private IPs and marks `runnersh.dev` as verified vs other hosts as `External · <host>` so GitHub gists and team links work), vs restricting strictly to an **allowlist** (`runnersh.dev` and future Runner marketplace hosts).
3. **`permission_mode = "bypass"` on import (Board 3 `Loy8J`):** Show an amber warning banner with a default-checked **"Clamp Bypass permissions to Auto on import"** toggle for external templates (recommended), vs stripping `bypass` unconditionally on export or import.

## Scope

### 1. Template format (`version = 1`, TOML)

Every template is a UTF-8 TOML document with a top-level `version = 1` integer and `kind = "crew"` or `kind = "role"`. Because `kind` is explicit, any import entry point (Crews page, Roles page, CLI, or `runner://`) identifies whether the document describes a crew or a standalone role without relying on the filename.

Optional `[meta]` metadata (`description`, `author`, `tags`, `homepage`) is displayed in the Import Preview and consumed directly by `runnersh.dev` (#1333) and future catalog surfaces. Runner's SQLite schema needs no migration: `[meta]` is not stored on local `crews` or `roles` rows in v1.

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

[[slots]]
handle = "qa"
role = "qa-codex"
effort = "high"

[[roles]]
handle = "impl-codex"
display_name = "Implementer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
# codex_speed = "fast"        # Codex only: "standard" | "fast"
permission_mode = "auto"      # "default" | "accept_edits" | "auto" | "bypass"
system_prompt = """
You are an implementer...
"""

[[roles]]
handle = "review-codex"
display_name = "Reviewer (Codex)"
runtime = "codex"
model = "gpt-6.2-astra"
effort = "xhigh"
permission_mode = "auto"
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
permission_mode = "auto"
system_prompt = """
You own critique only...
"""
```

#### Serialization and validation contract

- **Excluded fields:** Export never writes `id`, `created_at`, `updated_at`, `added_at`, `working_dir`, `env`, `command`, raw `args`, or `lead`. Import ignores nothing silently on security-sensitive fields: presence of `command`, `args`, `env`, or `working_dir` on a role or slot is rejected with a clear validation error so a hand-edited template cannot appear to inject executables or secrets.
- **Readable multiline formatting:** Export uses `toml_edit` (already in the workspace) to emit multiline `system_prompt` and `conventions` values as multiline TOML strings (`"""..."""` or literal `'''...'''` where valid), omits `None`/empty optional fields, and sorts `[[roles]]` in first-referenced slot order for clean diffs.
- **Version gating:** `version` must equal `1`; unknown or newer versions fail cleanly with `"Unsupported template version <n>; update Runner to import this file."`
- **Structural checks:**
  - `kind = "crew"` requires non-empty `crew.name`, forbids `[role]`, and allows an empty `[[slots]]` list (with empty `[[roles]]`) so an exported empty crew round-trips cleanly. Every `slots[].role` must match a unique `roles[].handle`, every `roles[]` entry must be referenced by at least one slot, and `slots[].handle` must be unique within the crew.
  - `kind = "role"` requires `[role]` and forbids `[crew]`, `[[slots]]`, and `[[roles]]`.
  - Handles follow Runner's existing `validate_role_handle` and `slot_handle_error` rules (lowercase ASCII slug).
- **Runtime and capability checks:**
  - Every `role.runtime` and `slot.runtime` must resolve to a known built-in agent runtime in `runner_core::protocol::runtime_metadata` (`shell` and unknown keys are rejected with a clear message naming the offending role/slot, and nothing is created).
  - `codex_speed` is accepted only when the effective runtime supports Codex Speed (`codex`).
  - `permission_mode` defaults to `"default"` when omitted on import; export writes the role's inferred `role_permission_mode(&role)` when the runtime supports permission modes. On import, `command` comes from the runtime registry and `args` are generated exclusively through `apply_permission_mode`.
  - A known runtime that is not installed on the current machine does **not** block import; preview marks that role/slot with a non-blocking **Not installed** hint (`Install in Settings → Agents before starting a mission`).

### 2. Collision resolution and atomic import (`runner-core` and `runnerd`)

Role handles are globally unique in SQLite (`roles.handle UNIQUE`), whereas crew names are non-unique display labels (`crews.name`).

- **Portable role equality:** Two roles are considered identical when `(display_name.trim(), runtime, model, effort, codex_speed, effective_permission_mode, normalized_prompt)` all match, where `normalized_prompt` normalizes CRLF to LF and trims trailing whitespace/newlines. Local-only columns (`working_dir`, `env`, IDs, timestamps) do not break reuse detection.
- **Crew import role resolution (per referenced role in `[[roles]]`):**
  1. **No local role with that handle:** Plan is `Create { handle }` (`New @<handle>`).
  2. **Existing local role is identical:** Plan defaults to `ReuseExisting { handle }` (`Reuses @<handle>`), avoiding duplicate role clutter when importing multiple crews that share a role.
  3. **Existing local role differs:** Plan defaults to `CreateRenamed { original, target }` where `target` is the first unused `<handle>-2`, `<handle>-3`, … suffix. Never overwrites the local role. In the preview (and via CLI options), the user can switch a conflicting role to `ReuseExisting { handle }` (pointing the imported crew's slot at their existing local role) or keep `CreateRenamed`.
- **Standalone role import resolution:**
  - If `@<handle>` does not exist, creates `@<handle>`.
  - If `@<handle>` already exists (identical or differing), defaults to auto-renamed `<handle>-2` (editable inline in the preview or via `--handle <new-handle>` in the CLI), with an explicit note that the existing role is untouched.
- **Crew name collisions:** If a local crew already has the same name, the preview pre-fills `<name> (2)` (editable before confirming) so the user can distinguish the new crew immediately.
- **Atomic writes:** `template_import` runs inside a single SQLite transaction in `runnerd`. Any validation or DB failure rolls back completely so zero orphan roles, crews, or slots remain. Importing creates database records only and never spawns a session or mission.

### 3. Daemon protocol and CLI (`runner-core`, `runner-daemon`, `runner-cli`)

Template parsing, validation, serialization, and portable-equality logic live in `runner-core` (`crates/runner-core/src/template/`) so the daemon, CLI, and app share one canonical implementation.

#### Daemon RPC (`crates/runner-daemon/src/ops/`)

- `template_export_crew { id }` → `{ filename, toml }`
- `template_export_role { id }` → `{ filename, toml }`
- `template_preview { toml, source? }` → structured `TemplatePreview` containing normalized template summary, per-role resolution plan (`Create | ReuseExisting | CreateRenamed`), runtime installation status from the local catalog, `bypass` permission warnings, crew name suggestion, and any validation error with field/line context.
- `template_import { toml, crew_name?, role_decisions, clamp_bypass }` → atomically creates the crew/slots/roles according to the confirmed plan and returns the created `Crew` (with slots) or `Role` plus a summary of created vs reused role handles.
- `template_fetch_url { url }` (daemon-side or shared `reqwest` helper behind the HTTPS/size/IP guards below) → `{ toml, source_host, verified_official }`.

#### CLI (`crates/runner-cli/src/command/crew.rs`, `role.rs`)

- `runner crew export <crew> [-o <path>] [--json]` — Resolves `<crew>` by name or ID. Prints TOML to stdout by default, or writes atomically to `<path>` when `-o` is passed. With `--json`, returns `{ "filename": "...", "toml": "..." }` (or `{ "path": "..." }` when `-o` is set).
- `runner crew import <file|url> [--name <name>] [--on-conflict rename|reuse|fail] [--allow-bypass] [--dry-run] [--json]` — Reads a local path (works offline with zero network calls) or fetches an HTTPS URL (applying the same URL safety checks). Defaults to `--on-conflict rename` (reusing identical roles, renaming differing ones) and clamps `bypass` to `auto`/`default` unless `--allow-bypass` is passed. `--dry-run` prints the preview plan without writing.
- `runner role export <role> [-o <path>] [--json]` — Resolves `<role>` by handle or ID; same output rules.
- `runner role import <file|url> [--handle <handle>] [--allow-bypass] [--dry-run] [--json]` — Same offline/URL and safety rules for standalone roles.
- Regenerate `docs/guide/reference/cli.md` with `make guide-cli`.

### 4. Desktop app UI (`crates/runner-app/src/surfaces/`)

Visual source of truth: `runner/desktop/specs/804-crew-role-templates.pen` in `yeelinks_design` (`e937105`). All UI strings go through `crates/runner-app/i18n/` (`en.toml` and `zh-hans.toml`).

#### Entry points (Board 1 `KUm7t`)

- **Crews list (`surfaces/crews/list.rs`):** Add a secondary **`Import…`** button (`download` icon) left of **`+ New crew`** in the page header and beside **`+ New crew`** in the `No crews yet` empty state, plus a quiet external link `Browse sample crews on runnersh.dev ↗` under the empty-state buttons. Hidden when viewing a remote host (preserving the existing read-only remote rule).
- **Crews row menu & Crew detail page (`surfaces/crews/list.rs`, `editor.rs`):** Add **`Export…`** (`upload` icon) to the list row `⋯` context menu above a divider before `Delete`. Add a compact 32×32 `⋯` button at the end of the Crew detail page's 272 px action row (`[ ▶ Start mission ] [ ✎ Edit ] [ ⋯ ]`) opening `Export…` and `Delete crew`.
- **Roles list & Role detail page (`surfaces/roles/list.rs`, `menu.rs`, `detail.rs`):** Mirror Crews exactly: secondary **`Import…`** button left of **`+ New role`** in header and empty state, **`Export…`** in row `⋯` menu, and `[ 💬 Chat now ] [ ✎ Edit ] [ ⋯ ]` (`Export…`, `Delete role`) on the Role detail page.
- **Native file dialogs:** `Export…` opens `cx.prompt_for_new_path` pre-filled with `<slug>.crew.toml` or `<slug>.role.toml` and shows a success toast on write. `Import…` opens `cx.prompt_for_paths` filtering `.toml` files, reads the file offline, calls `template_preview`, and opens the Import Preview (accepting either `kind = "crew"` or `kind = "role"` from either page).

#### Import Preview (Board 2 `vIBqb` & Board 3 `g8ycMu`)

- **Layout:** Follows whichever option Jason signs off on Board 2 (`T91Gr` Option A centered 640 px modal — recommended — or `j9Xj0` Option B full-page two-column review screen). Both present the exact same controls and data:
  - Header with crew pixel mosaic (or role pixel avatar), `Import crew` / `Import role` title, source badge (`Local file · offline`, green `runnersh.dev` verified badge, or amber `External · <host>` badge), template filename/version/author metadata, and dismiss control.
  - Editable Crew name field (or Role display name + editable `@handle` field for standalone roles) and `[meta].description`.
  - Non-blocking warning banner when any role requests `permission_mode = "bypass"` (with the default-checked clamp checkbox if approved) and inline **Not installed** badges for known uninstalled runtimes.
  - Ordered **Slots & Roles** list showing pixel avatar, `@slot`, role display name, runtime mark, `Runtime · model · effort · permission` (with Runner's amber override cue when a slot overrides its role), collision status badge (`New @handle`, `Reuses @handle`, or `@handle differs`), inline segmented conflict toggle (`Create @<handle>-2` | `Use existing @<handle>`), and Markdown prompt inspection rendered via the unified renderer (`crates/runner-app/src/ui/markdown/`).
  - **Team conventions** prompt preview (when non-empty).
  - Footer summarizing exact writes (`Creates 1 crew, 2 roles · reuses 1 role · no env or paths`) beside **`Cancel`** and primary **`Import crew`** / **`Import role`**.
- **Fetching and error states (Board 3 `a3Eb2`):** Opening a `runner://` URL shows a cancellable loading dialog (`Fetching template from <host>…` with the exact URL) before transitioning into the preview. Malformed TOML, unsupported versions, forbidden fields (`env`/`command`/`args`/`working_dir`), unknown runtimes, or fetch failures show a clear danger banner stating what failed (with line number when available) and confirming `Nothing was created.`
- **Post-import navigation:** On confirm, shows a toast (`Imported crew "Codex Trio"` / `Imported role "@review-codex-2"`), refreshes Crews/Roles lists, and opens the newly imported Crew or Role detail page so the user can inspect or immediately click **Start mission** / **Chat now**.
- **Themes:** Verified against both Dark and Runner Light tokens (`3D` `sY6Y6`).

### 5. `runner://` URL scheme (macOS, Windows, Linux)

Supports one-click web import from `runnersh.dev` (#1333) and future catalog links via:

```text
runner://import?url=https%3A%2F%2Frunnersh.dev%2Fcrews%2Fcodex-trio.crew.toml
```

- **Why URL fetch instead of inline payloads:** Real multi-role crews (`examples/codex-trio/`) are ~15 KB of Markdown prompts, which exceed safe Windows registry/command-line URL limits and produce unreadable browser consent prompts.
- **Fetch security boundaries:**
  - Scheme must be `https://` (`http://`, `file://`, `data:`, and credentials in URL are rejected).
  - Enforces a 256 KiB response body cap, 10 s total timeout, and at most 3 redirects (each redirect target re-checked against scheme and IP rules).
  - Resolves target IP addresses before connecting and refuses loopback, link-local, RFC 1918 private, and CGNAT ranges to prevent local-network SSRF.
  - Host allowlist vs open public HTTPS follows Jason's sign-off on Open Item 2; in open-HTTPS mode, `runnersh.dev` receives the verified green source badge while all other hosts display the amber `External · <host>` warning badge.
  - **Never auto-imports:** Every `runner://import?...` link always stops at the Import Preview for explicit confirmation.
- **Window routing:** Focuses an existing **local** Runner window (or opens a new local window if every open window is connected to a remote host — matching #1413's rule so a web link never disconnects an active remote session) and opens the Import Preview there.
- **Platform registration & cold/warm launch:**
  - **macOS:** Register `CFBundleURLTypes` (`CFBundleURLSchemes: ["runner"]`) in `script/bundle-mac` (`Info.plist`), and wire `application.on_open_urls(...)` in `crates/runner-app/src/startup.rs` (queuing URLs received before the first window opens so cold launch works identically to warm launch).
  - **Windows:** Register `HKCU\Software\Classes\runner` (`URL Protocol`, shell open command `"Runner.exe" "%1"`) in `script/windows/runner.iss`, parse `runner://` from startup argv in `startup.rs`, and extend `crates/runner-app/src/single_instance.rs` to forward the `runner://` URL payload over the existing named pipe when an instance is already running.
  - **Linux:** Add `MimeType=x-scheme-handler/runner;` and `%u` to `Exec=` in `packaging/linux/runner.desktop` (preserving `script/linux/install.sh`'s path-rewriting logic), parse `runner://` from argv on launch, and forward or handle cleanly.

### 6. Example crews conversion (`examples/`)

Convert the existing prompt folders under `examples/` (`codex-trio`, `dev-crew`, `docs-crew`, `pair-coding`, `tic-tac-toe`, `tomb-raid`, `werewolf`) into valid `.crew.toml` template files (retaining sample `goal.md` notes where helpful, since a crew carries no default goal). Add an automated test that parses and validates every `examples/**/*.crew.toml` file through `runner-core`'s template validator so shipped examples never drift from the v1 schema. These files become the initial seed set for `runnersh.dev` (#1333).

## Non-goals

- Publishing, hosting, ratings, search, or authentication for a crew marketplace on `runner_server` (separate future track).
- The `runnersh.dev` crew samples web page itself (tracked in #1333, built in Stage 2 after this feature lands).
- Editing role prompts, runtimes, or slot overrides inside the Import Preview (users edit freely on the Crew/Role detail page right after import).
- Exporting or importing machine-local or secret fields (`env`, `working_dir`, custom `command`, raw `args`).
- Remote-host crew or role import/export (Crews and Roles stay read-only when viewing a remote computer).
- Auto-starting a mission or auto-installing missing agent CLIs on import.

## Implementation phases

Three focused pull requests to `main`, matching the phases on #804 (the spec and active index entry land with Phase 1):

1. **Phase 1 — Template format, daemon ops, and CLI (`runner-core`, `runner-daemon`, `runner-cli`):** Implement `template` v1 TOML parser/serializer, validation, portable role equality, collision planner, atomic SQLite import transaction, URL fetch guard helper, daemon RPCs, `runner crew/role export|import` commands, `make guide-cli`, and comprehensive unit/integration tests.
2. **Phase 2 — Desktop UI and `runner://` URL scheme (`runner-app`, `script/bundle-mac`, `script/windows/runner.iss`, `packaging/linux/runner.desktop`, `docs/guide/`):** Add `Import…` and `Export…` entry points across Crews and Roles lists/detail pages/empty states, build the signed-off Import Preview (with unified Markdown prompt inspection, conflict toggles, warnings, fetching/error states, Dark/Light themes, and EN/zh-CN i18n), wire `runner://` registration and cold/warm launch handling across macOS/Windows/Linux, and update the user guides.
3. **Phase 3 — Convert `examples/` templates:** Convert the crews in `examples/` into validated `.crew.toml` files with an automated CI validation test, ready for Stage 2's `runnersh.dev` crew samples page (#1333).

## Guide pages

Update the English and `zh-CN` pages in Phase 2 (and regenerate the CLI reference in Phase 1 with `make guide-cli`):

- [Crews](../guide/concepts/crews.md) (exporting and importing crew templates)
- [Roles](../guide/concepts/roles.md) (exporting and importing standalone role templates)
- [Build a crew](../guide/guides/build-a-crew.md) (starting from a template file or `runner://` link and sharing a crew)
- [CLI reference](../guide/reference/cli.md) (`runner crew export|import`, `runner role export|import`)

## Verification

### Automated tests

- **Format & round-trip (`runner-core`):** Crew and standalone role export → import round-trip preserving slot order, overrides, runtime, model, effort, Codex speed, permission mode, conventions, and multiline Markdown prompts; golden TOML snapshots verifying readable multiline serialization and omission of empty optional fields.
- **Security & validation (`runner-core`):** Rejects `version != 1`, mismatched `kind` sections, duplicate slot handles, dangling slot role references, unknown runtimes, `shell` runtime, and forbidden fields (`env`, `working_dir`, `command`, `args`); confirms exported files never contain local paths, env vars, IDs, or timestamps.
- **Collision & atomicity (`runner-daemon`):** Verifies new role creation, identical-role auto-reuse (including CRLF/whitespace normalization and differing local `working_dir`/`env`), differing-role auto-rename (`-2`, `-3`), crew conflict override to `ReuseExisting`, standalone role handle rename, crew name suffix suggestion, and transactional rollback leaving zero rows on mid-import error.
- **URL fetch guard (`runner-core` / `runner-daemon`):** Unit tests rejecting `http://`, `file://`, embedded credentials, loopback/private/link-local IPs, bodies over 256 KiB, redirect to disallowed host/IP, and verifying `runnersh.dev` vs external host classification.
- **CLI (`runner-cli`):** End-to-end daemon tests for `crew/role export` (stdout, `-o`, `--json`) and `crew/role import` (offline local file, `--dry-run`, `--on-conflict`, `--allow-bypass`, exit codes).
- **App UI (`runner-app`):** GPUI harness tests for `Import…` / `Export…` visibility on local vs remote hosts, crew/role row and detail-page menus, Import Preview states (new, reuse, conflict toggle, standalone role rename, Bypass warning + clamp toggle, uninstalled runtime hint, fetching, error, Dark/Light), local-window selection on `runner://` while a remote window is active, and `examples/*.crew.toml` validation. Any async/daemon-backed app test passes 20 consecutive runs via `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`.

### Live checks

Phase 1 changes no UI screens; its CLI export/import round-trip against the development daemon is checked alongside `CLI-READ-01`. Phase 2 changes the Crews/Roles UI and adds OS URL-scheme handling, so `@qa` runs these checks on the macOS development app (using `script/qa-lock` and disposable roles/crews cleaned up afterward):

1. **Crew export → offline import round-trip (new & reuse):** Create a disposable two-slot crew with overrides and conventions, export via the row `⋯` menu (`Export…`), delete the crew (keeping one role, deleting the other), disconnect network or import locally via Crews `Import…`, verify preview shows 1 `New` role and 1 `Reuses` role plus conventions/prompt Markdown, confirm import, and verify the restored crew matches and opens cleanly. Reason: exercises export dialog, TOML round-trip, offline import, and reuse detection.
2. **Role handle conflict & standalone role export/import:** Edit the kept role's prompt so it differs from the exported template, re-import the crew, verify preview flags `@<handle> differs` with `Create @<handle>-2` selected by default (and switching to `Use existing` updates the footer count), confirm import, and verify the original local role is untouched. Repeat export/import on a single role from the Roles page and verify standalone auto-rename. Reason: verifies the non-overwrite guarantee and standalone role flow.
3. **Detail-page `⋯` menu, remote read-only guard, and Dark/Light check:** Open a Crew page and Role page to confirm `[ ⋯ ]` exposes `Export…` without crowding `Start mission`/`Chat now` or `Edit`, verify Dark and Runner Light contrast in the Import Preview, and confirm `Import…`/`Export…` stay hidden when viewing a remote host (or verify via automated test if no relay fixture is in scope). Reason: covers new UI surfaces on detail pages and remote boundary.
4. **Warnings & validation rejection:** Import a local fixture requesting `permission_mode = "bypass"` and an uninstalled built-in runtime (verify amber warning, safe clamp toggle, and non-blocking `Not installed` hint), then import a malformed fixture with an unknown runtime (verify error banner and zero created rows). Reason: covers security/validation UX.
5. **`runner://` URL handoff (warm & cold launch):** Trigger `open "runner://import?url=..."` against a local test fixture / HTTPS template URL (or `runner://` handler test on a bundled candidate `.app`) while Runner is running and after quitting Runner; verify Runner comes forward into a local window, opens the Import Preview with source badge, and creates nothing on Cancel. Reason: verifies macOS `CFBundleURLTypes` + `on_open_urls` registration; Windows Inno Setup registry/pipe forwarding and Linux `.desktop` `x-scheme-handler/runner` require native Windows/Linux verification (recorded explicitly as platform gaps if untested on macOS QA).

Selected regression cases: `MIS-CREW-01` (creating/editing a crew and starting a bounded mission from an imported crew still works end-to-end) and `CLI-READ-01` (CLI crew/role readback agrees with imported DB state).
