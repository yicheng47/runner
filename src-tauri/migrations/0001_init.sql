-- Migration 0001: initial schema.
--
-- Mirrors docs/arch/v0-arch.md §7.1 verbatim (four tables + the
-- one_lead_per_crew partial unique index). The signal_types column is
-- seeded with the built-in MVP allowlist via DEFAULT so every crew row
-- passes arch §5.3 Layer 2 validation without extra wiring. Users may
-- extend the list post-v0; in MVP the allowlist is write-only from the
-- DB layer.

CREATE TABLE crews (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    purpose TEXT,
    goal TEXT,
    orchestrator_policy TEXT,
    signal_types TEXT NOT NULL DEFAULT '["mission_goal","human_said","ask_lead","ask_human","human_question","human_response","inbox_read"]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE runners (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL,
    runtime TEXT NOT NULL,
    command TEXT NOT NULL,
    args_json TEXT,
    working_dir TEXT,
    system_prompt TEXT,
    env_json TEXT,
    lead INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (crew_id, handle)
);

CREATE UNIQUE INDEX one_lead_per_crew ON runners(crew_id) WHERE lead = 1;

CREATE TABLE missions (
    id TEXT PRIMARY KEY,
    crew_id TEXT NOT NULL REFERENCES crews(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    goal_override TEXT,
    cwd TEXT,
    started_at TEXT NOT NULL,
    stopped_at TEXT
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    pid INTEGER,
    started_at TEXT,
    stopped_at TEXT
);
