UPDATE runners
SET runtime = 'shell'
WHERE runtime NOT IN ('claude-code', 'codex', 'trae', 'shell');

UPDATE slots
SET runtime_override = NULL
WHERE runtime_override NOT IN ('claude-code', 'codex', 'trae');

UPDATE sessions
SET agent_runtime = 'shell'
WHERE agent_runtime IS NOT NULL
  AND agent_runtime NOT IN ('claude-code', 'codex', 'trae', 'shell');
