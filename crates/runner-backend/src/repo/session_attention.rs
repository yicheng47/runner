use rusqlite::{params, Connection};

pub fn any_unread(conn: &Connection, session_ids: &[String]) -> rusqlite::Result<bool> {
    for id in session_ids {
        if conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_attention WHERE session_id = ?1 AND unread_since IS NOT NULL)",
            [id],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn record_completion(
    conn: &Connection,
    session_id: &str,
    viewed: bool,
    since: i64,
) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO session_attention(session_id, unread_since) SELECT id, ?2 FROM sessions WHERE id = ?1 ON CONFLICT(session_id) DO UPDATE SET unread_since = CASE WHEN ?3 THEN session_attention.unread_since ELSE excluded.unread_since END", params![session_id, (!viewed).then_some(since), viewed])?;
    Ok(())
}

pub fn mark_viewed(conn: &Connection, session_id: &str, now: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO session_attention(session_id, error_acknowledged_at) SELECT id, ?2 FROM sessions WHERE id = ?1 ON CONFLICT(session_id) DO UPDATE SET unread_since = NULL, error_acknowledged_at = excluded.error_acknowledged_at", params![session_id, now])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_attention_updates_for_a_deleted_session_are_noops() {
        let core = crate::test_support::test_core();
        let conn = core.db.get().unwrap();
        record_completion(&conn, "deleted", false, 1).unwrap();
        mark_viewed(&conn, "deleted", "2026-09-14T00:00:01Z").unwrap();
        assert!(!any_unread(&conn, &["deleted".into()]).unwrap());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM session_attention", [], |row| row
                .get::<_, usize>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn pane_unread_and_error_acknowledgement_survive_a_fresh_manager() {
        let core = crate::test_support::test_core();
        {
            let conn = core.db.get().unwrap();
            conn.execute("INSERT INTO sessions(id, status, agent_runtime, stopped_at) VALUES ('a', 'crashed', 'codex', '2026-09-14T00:00:00Z'), ('b', 'running', 'codex', NULL)", []).unwrap();
            record_completion(&conn, "a", false, 100).unwrap();
            record_completion(&conn, "b", false, 200).unwrap();
            mark_viewed(&conn, "b", "2026-09-14T00:00:01Z").unwrap();
        }
        let statuses = crate::ops::session::session_status_snapshot(&core).unwrap();
        assert_eq!(statuses["a"].unread_since, Some(100));
        assert!(statuses["a"].error_since.is_some());
        assert_eq!(statuses["b"].unread_since, None);
        {
            let conn = core.db.get().unwrap();
            mark_viewed(&conn, "a", "2026-09-14T00:00:01Z").unwrap();
        }
        let statuses = crate::ops::session::session_status_snapshot(&core).unwrap();
        assert_eq!(statuses["a"].unread_since, None);
        assert_eq!(statuses["a"].error_since, None);
        assert_eq!(
            statuses["a"].lifecycle,
            crate::session::status::Lifecycle::Error
        );
    }
}
