# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

## Index

- [01 — Archived tab](./01-archived-tab.md) — view and unarchive
  missions and chats that fell off the active sidebar.
- [05 — Skills + MCPs management per runner](./05-runner-skills.md) —
  attach reusable skills and MCP servers to runner templates; injected
  natively at spawn via a per-session synthetic agent home.
- [12 — Multi-window frontend](./12-multi-window.md) — spawn
  additional Tauri windows for missions / chats; Arc-style overlay
  when two windows look at the same subject; PTY mounts only in the
  primary.
- [19 — Mission split view](./19-mission-split-view.md) — per-mission
  pane tree with drag-tab-to-edge splitting; render two PTYs (or
  feed + PTY) side-by-side inside one mission window; complements
  spec 12 (multi-window can't side-by-side same-mission PTYs).
- [21 — Resume existing sessions for the cwd](./21-resume-cwd-sessions.md)
  — Start Chat modal surfaces recent claude-code / codex sessions
  whose recorded cwd matches the picked working dir; selecting one
  passes the runtime's resume flag to the spawned child so the user
  continues their prior conversation in Runner.
- [24 — Cronjobs](./24-cronjobs.md)
  — scheduled recurring missions dispatched to a crew on a cron
  expression; in-process Tokio scheduler, skip-on-overlap, one
  missed-tick catch-up; new sidebar section between MISSION and CHAT.
- [29 — Runner and crew list pagination and search](./29-runner-crew-list-pagination-search.md)
  — add shared search filters and client-side pagination to the
  Runners and Crews list pages so large template/team inventories
  stay scannable.
- [30 — OpenCode runtime](./30-opencode-runtime.md)
  — add OpenCode as a first-class runtime for direct chats, runner
  templates, and missions with conservative model/prompt support
  before permission/resume mappings.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
