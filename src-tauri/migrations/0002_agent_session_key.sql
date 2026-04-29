-- Migration 0002: agent-native session resume key.
--
-- Each agent CLI (claude-code, codex, ...) has its own resumable
-- session/conversation id, separate from Runner's `sessions.id`.
-- We persist that id alongside the Runner session so reopening a
-- direct chat or a mission PTY after app restart can resume the
-- prior agent conversation instead of forcing a fresh one.
--
-- Captured at spawn time by the runtime adapter (router::runtime).
-- Reused on the next spawn for the same (runner_id) for direct chat,
-- or (mission_id, runner_id) for mission slots. NULL when the agent
-- hasn't reported an id yet, the runtime doesn't expose one, or the
-- last spawn crashed before capture.

ALTER TABLE sessions ADD COLUMN agent_session_key TEXT;
