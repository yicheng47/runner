-- Migration 0003: archived flag for sessions.
--
-- Pre-resume, "session ended" effectively meant "gone" — the SESSION
-- sidebar tray was driven by runner_activity.direct_session_id, which is
-- only populated for running PTYs. Once 0002 added agent-native session
-- resume, a stopped session can be re-launched and pick up the prior
-- agent conversation, so it has user-visible identity beyond its PTY
-- lifetime. The sidebar should list those un-archived sessions, not just
-- the live ones.
--
-- archived_at is the soft-delete timestamp. NULL = visible in SESSION;
-- non-NULL = hidden from the live tray (a future Archived workspace
-- surface — see docs/impls/v0-mvp.md "Out of scope" — will list them).

ALTER TABLE sessions ADD COLUMN archived_at TEXT;
