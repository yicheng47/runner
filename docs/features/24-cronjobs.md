# 24 — Cronjobs

> Tracking issue: [#193](https://github.com/yicheng47/runner/issues/193)

## Motivation

Today every mission in Runner is reactive: a human types a goal, hits
Start, and the crew runs. There is no way to say "run this crew against
this goal every weekday at 9am" or "sweep the repo for stale TODOs
every Monday." The user who wants periodic agent work must remember to
open the app and launch the mission by hand.

That's fine for one-shot feature work. It doesn't fit the workflows
that justify keeping Runner open all day:

- **Daily code review** — a docs-crew sweep of yesterday's commits.
- **Nightly regression check** — a build-squad run against the test
  suite on a schedule.
- **Weekly summary** — an architect produces a status report every
  Monday morning.
- **Periodic monitoring** — a single-runner crew checks a health
  endpoint every 15 minutes and posts an alert if it's down.

These are all "the same mission, on a schedule, dispatched to a crew."
The primitive is a **cronjob**: a mission template with a cron
expression. Each tick creates a normal Mission through the existing
`mission_start` path — same crew, same PTY lifecycle, same event log,
same workspace UI. The only new concept is the trigger.

## Scope

### In scope (v1)

- **`cronjobs` table.** New SQLite table:
  ```sql
  CREATE TABLE cronjobs (
    id          TEXT PRIMARY KEY,
    crew_id     TEXT NOT NULL REFERENCES crews(id),
    title       TEXT NOT NULL,
    goal        TEXT NOT NULL DEFAULT '',
    cwd         TEXT,
    cron_expr   TEXT NOT NULL,  -- standard 5-field cron
    enabled     INTEGER NOT NULL DEFAULT 1,
    timeout_s   INTEGER,        -- optional max run duration
    last_run_at TEXT,           -- ISO 8601 UTC
    next_run_at TEXT,           -- precomputed on create/tick
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
  );
  ```
  Migration: `0008_cronjobs.sql`.
- **CRUD commands.** Tauri commands mirroring the mission pattern:
  - `cronjob_create(crew_id, title, goal, cwd, cron_expr, timeout_s)`
    → validates the cron expression, computes `next_run_at`, returns
    the row.
  - `cronjob_update(id, ...)` — partial update of any mutable field.
  - `cronjob_delete(id)` — hard delete (cronjobs are cheap to
    recreate; no archive).
  - `cronjob_list()` → all cronjobs, joined with crew name.
  - `cronjob_enable(id, enabled)` — flip the toggle; recomputes
    `next_run_at` on enable.
  - `cronjob_history(id, limit)` → missions spawned by this cronjob
    (via `spawned_by_cronjob_id` FK on missions), most recent first.
- **In-process scheduler.** A Tokio task spawned at app startup that:
  1. On boot, loads all enabled cronjobs and their `next_run_at`.
  2. Sleeps until the earliest `next_run_at` (or until a cronjob is
     created/updated/enabled, which wakes the task via a
     `tokio::sync::Notify`).
  3. When `next_run_at <= now`: calls the existing `start()` function
     (same entry point as `mission_start` command) with the cronjob's
     `crew_id`, `title`, `goal`, and `cwd`. Tags the resulting
     mission row with `spawned_by_cronjob_id`.
  4. Updates `last_run_at` and computes the next `next_run_at`.
  5. Loops.
- **Concurrency guard.** If the previous run's mission is still
  `Running` when the next tick fires, the tick is **skipped** (not
  queued). A `skipped` event is logged to the cronjob's history so
  the user knows. This is the standard cron semantic — avoids
  unbounded queue buildup if a mission runs longer than the interval.
- **Missed-run catch-up.** If the app wasn't running when a tick was
  due, fire **once** on startup (the most recent missed tick only).
  Don't fire N times for N missed ticks — the user didn't ask for a
  backfill, they asked for a schedule.
- **Timeout.** Optional `timeout_s` on the cronjob. If set, the
  scheduler calls `mission_stop` after `timeout_s` seconds if the
  mission is still running. The mission's status flips to `Aborted`.
- **Mission linkage.** New nullable column on missions:
  ```sql
  ALTER TABLE missions ADD COLUMN spawned_by_cronjob_id TEXT
    REFERENCES cronjobs(id);
  ```
  Missions spawned by a cronjob carry this FK. The mission workspace
  shows a small "Cronjob: <title>" badge in the header so the user
  knows this wasn't a manual run. `cronjob_history` queries this FK.
- **Sidebar section.** New "CRONJOB" section in the sidebar, between
  MISSION and CHAT. Each row shows:
  - Title (left-aligned).
  - Next run time as a relative timestamp ("in 3h", "tomorrow 9am").
  - A status indicator: `enabled` (green dot), `disabled` (gray
    dot), `running` (spinning dot, when the current run is live).
  - Right-click context menu: Enable / Disable, Edit, Delete, Run
    Now (manual trigger outside the schedule).
  - Click → opens a detail panel (or modal) showing the cronjob's
    config, schedule, and run history (list of past missions with
    status + duration + link to open the mission workspace).
- **Create cronjob UI.** A modal opened from the `+` button on the
  CRONJOB section header. Fields:
  - **Crew** — dropdown of existing crews.
  - **Title** — short name for the cronjob.
  - **Goal** — the mission goal text (same textarea as Start Mission).
  - **Working directory** — optional, same folder picker as Start
    Mission.
  - **Schedule** — cron expression input with a human-readable
    preview ("Every weekday at 9:00 AM"). Presets: "Every hour",
    "Every day at 9am", "Every Monday at 9am", "Custom".
  - **Timeout** — optional, in minutes.
  - **Enabled** — toggle, default on.

### Out of scope (deferred)

- **Remote/headless execution.** The scheduler runs in-process in
  Tauri; the app must be open for ticks to fire. A headless daemon
  mode (launchd agent on macOS, systemd on Linux) is a follow-up.
- **Cron expression editor UI.** v1 ships a text input with presets
  and a human-readable preview. A visual day/hour picker grid is a
  follow-up.
- **Notification on completion/failure.** Spec 14 (human
  notifications) already covers `ask_human` signals; cronjob
  completion notifications are a follow-up that layers on spec 14.
- **Multi-run queue.** Missed ticks fire at most once; concurrent
  ticks are skipped. A "run N times to catch up" mode is out of
  scope.
- **Cronjob folders / grouping.** When spec 17 (sidebar folders)
  lands, cronjobs should be groupable too; deferred to that spec.
- **Environment variable overrides per cronjob.** The crew's runner
  templates already carry env; per-cronjob env overrides are a
  follow-up.

### Key decisions

1. **In-process Tokio scheduler, not OS-level cron/launchd.** The
   app must be running to spawn PTYs (they're child processes of the
   Tauri backend). An OS-level trigger that launches the app on
   schedule is attractive but adds platform-specific complexity
   (launchd plist on macOS, Task Scheduler on Windows, systemd on
   Linux) that doesn't justify itself in v1. The in-process
   scheduler is ~100 lines of Rust and covers the "app is open all
   day" use case that cronjobs target.
2. **Skip on overlap, don't queue.** If a mission takes 2 hours and
   the cron is every hour, the second tick is skipped. The
   alternative (queue it) risks unbounded mission buildup if the
   crew is slow or stuck. Skip + log is the standard cron semantic
   and the safer default.
3. **One missed-tick catch-up, not N.** If the app was closed for
   a week and a daily cronjob has 7 missed ticks, only one fires on
   startup. The user can always "Run Now" if they want a specific
   backfill. This matches `anacron`'s behavior.
4. **Cronjob is a mission template, not a new execution primitive.**
   Each tick calls the same `start()` that `mission_start` uses.
   The cronjob row is config; the mission row is the run. This means
   every cronjob run gets the full mission infrastructure (event
   log, workspace, feed, terminal tabs) for free. Trade: we add one
   FK column to missions, not a parallel execution table.
5. **Hard delete, no archive.** Cronjobs are cheap to recreate (a
   title + goal + cron expression). Archiving adds a state
   dimension (enabled vs disabled vs archived) that isn't worth the
   UI complexity in v1. Past runs (missions) survive deletion of
   the cronjob — the FK is nullable on the mission side, so
   `spawned_by_cronjob_id` becomes a dangling reference that the
   UI renders as "Deleted cronjob."
6. **5-field cron, not 6/7-field.** Standard `minute hour dom month
   dow`. No seconds field (overkill for agent missions), no year
   field. Matches what users know from `crontab -e`.
7. **Sidebar section between MISSION and CHAT.** Cronjobs are
   closer to missions than to chats conceptually (they produce
   missions). Placing them adjacent to MISSION keeps the "work"
   cluster together. The section is collapsible like the other two.

## Implementation phases

### Phase 1 — schema + CRUD

- Migration `0008_cronjobs.sql`: create the `cronjobs` table and add
  `spawned_by_cronjob_id` to `missions`.
- New `src-tauri/src/commands/cronjob.rs` module with `cronjob_create`,
  `cronjob_update`, `cronjob_delete`, `cronjob_list`,
  `cronjob_enable`, `cronjob_history`.
- Add `Cronjob` struct to `model.rs`.
- Cron expression parsing: use the `cron` crate
  (`cron = "0.13"`) for parsing + next-occurrence computation.
  Validate on create/update; reject invalid expressions with a
  user-facing error message.
- Register commands in `main.rs`.
- Unit tests for CRUD + cron expression validation + next_run_at
  computation.

### Phase 2 — scheduler

- New `src-tauri/src/scheduler.rs` module.
- `CronScheduler` struct holding:
  - A `tokio::sync::Notify` for wake-on-change.
  - A `JoinHandle` for the scheduler loop.
- On app startup (`setup` in `main.rs`), spawn the scheduler as a
  managed Tauri state.
- Scheduler loop:
  1. Load enabled cronjobs from DB.
  2. Find earliest `next_run_at`.
  3. `tokio::select!` on sleep-until-earliest vs `notify.notified()`.
  4. On tick: check concurrency guard (is the previous mission still
     running?), call `start()`, update `last_run_at` / `next_run_at`.
  5. On notify: reload cronjobs from DB (a CRUD command changed
     something) and restart the loop.
- Missed-run catch-up: on boot, for each enabled cronjob where
  `next_run_at < now`, fire once and advance `next_run_at`.
- Timeout: if `timeout_s` is set, spawn a separate `tokio::spawn`
  that sleeps for `timeout_s` and calls `mission_stop` if the
  mission is still running.
- Integration test: create a cronjob with `* * * * *` (every
  minute), assert a mission is created within 70s, assert
  `last_run_at` is updated.

### Phase 3 — sidebar + create modal

- New `CronjobSection` in `Sidebar.tsx`, between MISSION and CHAT.
  Collapsible header with count + `+` button.
- `CronjobRow` component: title, relative next-run time, status dot
  (green/gray/spinning). Click opens the detail view. Right-click
  opens context menu (Enable/Disable, Edit, Delete, Run Now).
- "Run Now" calls `cronjob_create_run` (a new command that calls
  `start()` immediately with the cronjob's config, bypassing the
  schedule).
- `CreateCronjobModal`: crew dropdown, title, goal textarea, cwd
  folder picker, cron expression input with presets + human-readable
  preview, timeout input, enabled toggle. Calls `cronjob_create` on
  submit.
- `CronjobDetail` panel/modal: shows config fields (editable inline
  or via Edit button), schedule with human-readable rendering, and
  a run history table (mission title, status, started_at, duration,
  click-to-open).
- Wire frontend API layer: add `api.cronjob.*` methods calling the
  Tauri commands.

### Phase 4 — verification

- **Functional smoke:**
  1. Create a cronjob with "Every minute" schedule → within 60s a
     mission appears in the MISSION section, linked to the cronjob.
  2. Disable the cronjob → the next tick does not fire; status dot
     goes gray.
  3. Re-enable → `next_run_at` is recomputed; next tick fires.
  4. "Run Now" → mission starts immediately regardless of schedule.
  5. Delete cronjob → row disappears from sidebar; past missions
     still visible with "Deleted cronjob" badge.
  6. Set `timeout_s = 10`, start a crew that takes >10s → mission
     is aborted after 10s.
  7. Start a slow-running crew on a fast schedule (every minute) →
     overlapping tick is skipped; history shows "skipped" entry.
  8. Quit the app, wait past a tick, relaunch → one catch-up run
     fires on startup; only one, not N.
- **Cross-spec compatibility:**
  - Spec 23 (drag reorder): cronjob rows are reorderable in the
    sidebar (add `sort_index` to cronjobs if spec 23 lands first).
  - Spec 22 (collapsed rail): cronjobs don't appear on the rail
    in v1 (they're config, not active work).
  - Spec 14 (notifications): cronjob-spawned missions emit
    `ask_human` normally; notification follow-up layers on top.
- **Backend:** `cargo fmt && cargo clippy --all-targets
  --all-features` clean; `cargo test` passes including the new
  scheduler integration test.
- **Frontend:** `pnpm exec tsc --noEmit` clean.

## Verification

- [ ] `cronjobs` table created; `missions.spawned_by_cronjob_id`
      column added.
- [ ] CRUD commands work: create, update, delete, list, enable,
      history.
- [ ] Cron expression validation rejects invalid expressions with a
      user-facing error.
- [ ] Scheduler fires missions on schedule (tested with every-minute
      cron).
- [ ] Concurrency guard: overlapping tick is skipped, not queued.
- [ ] Missed-run catch-up: one run fires on app relaunch, not N.
- [ ] Timeout: mission is aborted after `timeout_s` seconds.
- [ ] Sidebar CRONJOB section renders between MISSION and CHAT with
      correct status dots.
- [ ] Create Cronjob modal: crew picker, cron presets + preview,
      timeout, enable toggle all work.
- [ ] Right-click context menu: Enable/Disable, Edit, Delete, Run
      Now.
- [ ] Run Now creates a mission immediately.
- [ ] Cronjob detail shows config + run history with links to
      mission workspaces.
- [ ] Deleting a cronjob preserves past missions with "Deleted
      cronjob" badge.
- [ ] `cargo fmt + clippy + test` clean; `tsc --noEmit` clean.
