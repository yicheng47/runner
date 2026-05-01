-- Migration 0001: initial schema (pre-v0.1.1 squash).
--
-- We're pre-release with no production data, so all the
-- 0001..0008 migrations were collapsed into this single file
-- ahead of v0.1.1's auto-update pipeline. The schema below is the
-- current end state — see the table-by-table summary at the bottom
-- for which historical migration introduced what. Dev users with a
-- pre-squash DB ($APPDATA/runner/runner.db) delete the file once
-- to pick up the new shape.
--
-- Overview:
--   - crews     — named groups of slots, with orchestrator policy +
--                 mission-event signal allowlist.
--   - runners   — global, shareable agent templates. One handle =
--                 one runner everywhere it appears in the event log.
--                 Carries optional model / effort overrides for
--                 agents that take CLI flags for those (claude-code
--                 → --model + --effort; codex → --model only).
--   - slots     — crew↔runner join with per-slot identity. The
--                 `slot_handle` is the in-crew name used by mission
--                 events, the RUNNER_HANDLE env var, and router
--                 routing. Replaces the original `crew_runners`.
--   - missions  — scoped to a crew. Spawns one session per slot.
--   - sessions  — a PTY run of a runner. mission_id is nullable:
--                 "direct chat" sessions exist without a mission
--                 (cwd lives on the session row in that case).

CREATE TABLE crews (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    purpose TEXT,
    goal TEXT,
    orchestrator_policy TEXT,
    signal_types TEXT NOT NULL DEFAULT '["mission_goal","human_said","ask_lead","ask_human","human_question","human_response","runner_status","inbox_read"]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE runners (
    id TEXT PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    runtime TEXT NOT NULL,
    command TEXT NOT NULL,
    args_json TEXT,
    working_dir TEXT,
    system_prompt TEXT,
    env_json TEXT,
    -- Per-runner agent-CLI tuning. Both nullable: when NULL the
    -- runtime adapter omits the corresponding flag and the agent
    -- falls back to its own default. Folded in from 0008.
    model TEXT,
    effort TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE slots (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    slot_handle TEXT NOT NULL,
    position INTEGER NOT NULL,
    lead INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    -- Folded in from 0006. "At most one lead per crew" is enforced
    -- in the slot commands (transactional clear-others-then-set)
    -- rather than a partial unique index; keeps the SQL portable
    -- and the invariant visible in code.
    UNIQUE (crew_id, slot_handle),
    UNIQUE (crew_id, position)
);

CREATE TABLE missions (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    goal_override TEXT,
    cwd TEXT,
    started_at TEXT NOT NULL,
    stopped_at TEXT,
    -- Sidebar pin. Pinned missions sort above unpinned ones; NULL =
    -- unpinned. Folded in from 0007.
    pinned_at TEXT
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    -- Mission sessions hook to a slot; direct chats stay NULL.
    -- Folded in from 0006.
    slot_id TEXT,
    cwd TEXT,
    status TEXT NOT NULL,
    pid INTEGER,
    started_at TEXT,
    stopped_at TEXT,
    -- Agent-native conversation id (claude-code session, codex
    -- conversation, ...). Persisted at spawn time so reopening a
    -- session after restart can resume the prior conversation
    -- instead of starting fresh. NULL when the agent hasn't
    -- reported one or the runtime doesn't expose one. Folded in
    -- from 0002.
    agent_session_key TEXT,
    -- Soft-delete timestamp. NULL = visible in the SESSION tray;
    -- non-NULL = hidden (a future Archived workspace surfaces
    -- them). Folded in from 0003.
    archived_at TEXT,
    -- Optional user-authored title for direct chats. Mission
    -- sessions don't use this (the mission row has its own
    -- title), but the column lives here so the schema stays
    -- uniform across both kinds. Folded in from 0004.
    title TEXT,
    -- Pin direct-chat sessions to the top of the SESSION tray.
    -- Storing the timestamp (vs a boolean) lets a future
    -- "recent pins" affordance order pins by creation time.
    -- Folded in from 0005.
    pinned_at TEXT
);

-- Historical migrations folded into this file (in case the
-- archaeology is ever useful):
--   0001 — original schema (crews / runners / crew_runners /
--          missions / sessions). The `crew_runners` join, the
--          `runners.role` column, and the `one_lead_per_crew`
--          partial unique index are NOT carried forward — see
--          0006.
--   0002 — sessions.agent_session_key.
--   0003 — sessions.archived_at.
--   0004 — sessions.title.
--   0005 — sessions.pinned_at.
--   0006 — `crew_runners` → `slots`; runners.role dropped;
--          sessions.slot_id added.
--   0007 — missions.pinned_at.
--   0008 — runners.model + runners.effort.
