# Settings as a YAML file under `~/.runner/`

Status: planned, before `v0.6.0` (Jason, 2026-08-22). Program slot: M6.14 in [m6-consolidation.md](../impls/gpui-rewrite/m6-consolidation.md).

## Motivation

Runner's settings should be a plain text file a human or an agent can open, read, and edit — not rows in SQLite and not an app-private JSON blob. The immediate driver is agent-editability: "set the terminal font to X and turn off auto-resume" should be a file edit Claude Code or Codex can make from a terminal, with the app picking it up live.

Where things are today (native line, 2026-08-22):

- `~/Library/Application Support/com.wycstudios.runner/ui-settings.json` — `AppSettings` (`crates/runner-app/src/app_settings.rs`): theme, fonts, zoom, terminal theme/font/size/cursor, sidebar and panel geometry and open/collapsed state, `last_mission_terminal_ids`, default crew and working directory, resume-on-launch, automatic update checks, default runtime, enabled/disabled agents, keymap overrides. camelCase JSON, written whole on every change, read once at launch. (`main`'s Tauri app kept all of this in the webview's localStorage; nothing migrated — the native line started from defaults.)
- SQLite `_app_state` (`crates/runner-backend/src/db.rs:189-191`), a key/value table holding three things of different kinds: `runtime_overrides` (per-agent executable path overrides from Settings → Agents — a **setting**), `login_shell_env_lkg` (the last-known-good login-shell environment — a **cache**), `default_crew_seeded` (a one-time seed **marker**).
- Everything else in the DB is data (runners, crews, slots, projects, missions, sessions, nodes) and stays there.

## Scope

- **Home**: `~/.runner/` (override with `RUNNER_HOME`), created on first launch. It holds configuration only; the data dir (`runner.db`, logs, mission event logs, window files) stays under Application Support.
- **`~/.runner/settings.yaml`**: one file, snake_case keys, grouped by section — `appearance` (theme, fonts, zoom), `terminal`, `sidebar` / `panels` (geometry and open/collapsed state), `chat` (default runtime, default working dir, resume on launch), `agents` (enabled/disabled set, executable overrides — the `runtime_overrides` row moves here), `updates`, `keymap` (the overrides map), `missions` (default crew, `last_mission_terminal_ids`). The app owns the schema: `serde` structs, unknown keys preserved on round-trip where serde allows (`#[serde(flatten)]` catch-all) or at least warned about, never silently dropped on save.
- **Read**: at launch; then a `notify` watcher on the file (the crate is already a dependency for event logs) triggers a debounced reload. A reload that fails to parse keeps the last good settings and surfaces the existing `settings_error` toast with the line/column; the app never overwrites a file it could not parse.
- **Write**: on every settings change from the UI, the whole file is serialized and written atomically (temp file + rename). `serde_yaml` drops comments, so the app regenerates a short commented header (file purpose, link to the schema doc, "edited by Runner at <time>") rather than promising to preserve hand-written comments. Geometry fields (widths, collapsed sets) churn on every drag — coalesce writes (the current JSON path already writes whole-file; keep a ≥250 ms debounce so an agent editing the file does not race a drag).
- **Schema doc**: `docs/arch/settings.md` listing every key, type, default and which pane owns it — the thing an agent reads before editing.
- **Migration**, first launch after the change: if `settings.yaml` is absent and `ui-settings.json` exists, convert it (plus the `runtime_overrides` row) into the YAML, write it, rename the JSON to `ui-settings.json.migrated`, delete the DB row only after the YAML write succeeded. No migration needed from the Tauri line (it never shared a format).
- **Stays in the DB**: `login_shell_env_lkg` (derived cache), `default_crew_seeded` (marker). `_app_state` keeps existing for those.
- **One store**: the settings panes, `AppStore` revision fingerprints and the multi-window broadcast keep working unchanged — only the persistence layer behind `AppSettings::load/save` and the runtime-override ops moves.

## Non-goals

- Syncing settings between machines, or per-project settings files.
- Moving the database, logs or mission event logs under `~/.runner/`.
- Crew / runner / role definitions as files under `~/.runner/` — attractive (they are the other thing agents want to edit, and Jason's crew prompts already live in a repo), but a separate feature with its own sync story against the DB rows.
- Preserving hand-written comments across app writes.

## Open questions

- Whether `keymap` overrides get their own `keymap.yaml` (Zed convention) or stay a section of `settings.yaml`. Default: one file until it is unwieldy.
- Whether the app should refuse to write while the file is newer on disk than what it last read (an agent mid-edit). Default: last writer wins, with the debounce above; revisit if it bites.
