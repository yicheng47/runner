CREATE TABLE session_attention (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    unread_since INTEGER,
    error_acknowledged_at TEXT
);

INSERT OR IGNORE INTO session_attention(session_id, unread_since)
SELECT s.id, CAST(unixepoch(n.last_completed_at, 'subsec') * 1000 AS INTEGER)
FROM nodes n, json_each(CASE WHEN json_valid(n.layout) THEN n.layout ELSE '{}' END, '$.slots') slot
JOIN sessions s ON s.id = slot.value
WHERE n.type = 'tab' AND n.last_completed_at IS NOT NULL
  AND (n.last_viewed_at IS NULL OR julianday(n.last_completed_at) > julianday(n.last_viewed_at));
