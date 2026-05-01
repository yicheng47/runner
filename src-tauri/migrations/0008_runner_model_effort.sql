-- Per-runner model + effort selection.
--
-- Why: today every spawn inherits whatever the underlying agent CLI's
-- own default is (claude-code → Sonnet, codex → its default), so users
-- can't pin a runner template to e.g. Opus + xhigh effort. Both fields
-- are agent-specific strings: claude-code maps `model` to `--model`
-- and `effort` to its xhigh / high / medium thinking flag; codex's
-- mapping lands in the runtime adapter (router/runtime.rs).
--
-- Both columns are nullable so existing rows keep their current
-- "agent default" behavior unchanged. The runtime adapter omits the
-- corresponding flags when the column is NULL.

ALTER TABLE runners ADD COLUMN model TEXT;
ALTER TABLE runners ADD COLUMN effort TEXT;
