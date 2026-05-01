-- Migration 0001: initial schema.
--
-- Pre-release squash of the original 0001..0008. Dev users with an
-- older DB ($APPDATA/runner/runner.db) delete the file once to
-- re-init.

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
    pinned_at TEXT
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    slot_id TEXT,
    cwd TEXT,
    status TEXT NOT NULL,
    pid INTEGER,
    started_at TEXT,
    stopped_at TEXT,
    agent_session_key TEXT,
    archived_at TEXT,
    title TEXT,
    pinned_at TEXT
);
