-- Allow direct-chat sessions to be backed by a runtime without a
-- persisted runner template.
--
-- `sessions.runtime` already stores PTY-runtime metadata (`pty`, legacy
-- tmux), so agent identity uses explicit `agent_*` columns.

CREATE TABLE sessions_new (
    id TEXT PRIMARY KEY,
    mission_id TEXT REFERENCES missions(id) ON DELETE SET NULL,
    runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
    slot_id TEXT,
    cwd TEXT,
    status TEXT NOT NULL,
    pid INTEGER,
    started_at TEXT,
    stopped_at TEXT,
    agent_session_key TEXT,
    archived_at TEXT,
    title TEXT,
    pinned_at TEXT,
    runtime TEXT,
    runtime_socket TEXT,
    runtime_session TEXT,
    runtime_window TEXT,
    runtime_pane TEXT,
    runtime_cursor INTEGER,
    agent_runtime TEXT,
    agent_command TEXT
);

INSERT INTO sessions_new (
    id,
    mission_id,
    runner_id,
    slot_id,
    cwd,
    status,
    pid,
    started_at,
    stopped_at,
    agent_session_key,
    archived_at,
    title,
    pinned_at,
    runtime,
    runtime_socket,
    runtime_session,
    runtime_window,
    runtime_pane,
    runtime_cursor,
    agent_runtime,
    agent_command
)
SELECT
    id,
    mission_id,
    runner_id,
    slot_id,
    cwd,
    status,
    pid,
    started_at,
    stopped_at,
    agent_session_key,
    archived_at,
    title,
    pinned_at,
    runtime,
    runtime_socket,
    runtime_session,
    runtime_window,
    runtime_pane,
    runtime_cursor,
    NULL,
    NULL
FROM sessions;

DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;
