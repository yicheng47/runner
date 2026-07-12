use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_row;

use super::{de_err, select_list};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabRow {
    pub id: String,
    pub folder_id: Option<String>,
    pub name: String,
    pub position: i64,
    pub layout: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredLayout {
    #[serde(default)]
    slots: Vec<Option<String>>,
}

const COLUMNS: &[&str] = &[
    "id",
    "folder_id",
    "name",
    "position",
    "layout",
    "created_at",
];

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<TabRow>> {
    let sql = format!(
        "SELECT {} FROM tabs ORDER BY CASE WHEN folder_id IS NULL THEN 1 ELSE 0 END, position, created_at",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], |row| from_row(row).map_err(de_err))?
        .collect();
    rows
}

pub fn list_with_active_sessions(conn: &mut Connection) -> rusqlite::Result<Vec<TabRow>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    ensure_active_sessions(&tx)?;
    let rows = list(&tx)?;
    tx.commit()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<TabRow>> {
    let sql = format!("SELECT {} FROM tabs WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, [id], |row| from_row(row).map_err(de_err))
        .optional()
}

pub fn upsert(conn: &Connection, row: &TabRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tabs (id, folder_id, name, position, layout, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             folder_id = excluded.folder_id,
             name = excluded.name,
             position = excluded.position,
             layout = excluded.layout",
        rusqlite::params![
            row.id,
            row.folder_id,
            row.name,
            row.position,
            row.layout,
            row.created_at
        ],
    )?;
    Ok(())
}

pub fn upsert_move_not_copy(conn: &Connection, row: &TabRow) -> rusqlite::Result<()> {
    for session_id in session_ids_from_layout(&row.layout) {
        remove_session(conn, &session_id)?;
    }
    upsert(conn, row)
}

pub fn create(
    conn: &Connection,
    folder_id: Option<&str>,
    name: &str,
    position: i64,
    layout: &str,
) -> rusqlite::Result<TabRow> {
    let row = TabRow {
        id: ulid::Ulid::new().to_string(),
        folder_id: folder_id.map(str::to_owned),
        name: name.to_owned(),
        position,
        layout: layout.to_owned(),
        created_at: Utc::now().to_rfc3339(),
    };
    upsert(conn, &row)?;
    Ok(row)
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM tabs WHERE id = ?1", [id])
}

pub fn move_to_folder(
    conn: &Connection,
    id: &str,
    folder_id: Option<&str>,
) -> rusqlite::Result<usize> {
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM tabs WHERE folder_id IS ?1",
        [folder_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE tabs SET folder_id = ?2, position = ?3 WHERE id = ?1",
        rusqlite::params![id, folder_id, position],
    )
}

pub fn move_and_reorder(
    tx: &Transaction<'_>,
    id: &str,
    folder_id: Option<&str>,
    ordered_ids: &[String],
) -> rusqlite::Result<()> {
    if tx.execute(
        "UPDATE tabs SET folder_id = ?2 WHERE id = ?1",
        rusqlite::params![id, folder_id],
    )? == 0
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    let actual: Vec<String> = tx
        .prepare("SELECT id FROM tabs WHERE folder_id IS ?1")?
        .query_map([folder_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let expected: HashSet<&str> = actual.iter().map(String::as_str).collect();
    let provided: HashSet<&str> = ordered_ids.iter().map(String::as_str).collect();
    if ordered_ids.len() != actual.len()
        || provided.len() != ordered_ids.len()
        || provided != expected
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "ordered tab ids do not match destination group".to_owned(),
        ));
    }
    for (position, tab_id) in ordered_ids.iter().enumerate() {
        tx.execute(
            "UPDATE tabs SET position = ?2 WHERE id = ?1",
            rusqlite::params![tab_id, position as i64],
        )?;
    }
    Ok(())
}

pub fn session_ids(row: &TabRow) -> Vec<String> {
    session_ids_from_layout(&row.layout)
}

pub fn session_ids_from_layout(layout: &str) -> Vec<String> {
    serde_json::from_str::<StoredLayout>(layout)
        .map(|layout| layout.slots.into_iter().flatten().collect())
        .unwrap_or_default()
}

pub fn ensure_active_sessions(conn: &Connection) -> rusqlite::Result<()> {
    let covered: HashSet<String> = list(conn)?.iter().flat_map(session_ids).collect();
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions
         WHERE mission_id IS NULL AND slot_id IS NULL AND archived_at IS NULL
         ORDER BY COALESCE(started_at, stopped_at), id",
    )?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM tabs WHERE folder_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    for (offset, id) in ids
        .into_iter()
        .filter(|id| !covered.contains(id))
        .enumerate()
    {
        let layout = serde_json::json!({
            "preset": "single",
            "slots": [id],
            "sizes": {},
        })
        .to_string();
        create(conn, None, "", position + offset as i64, &layout)?;
    }
    Ok(())
}

pub fn remove_session(conn: &Connection, session_id: &str) -> rusqlite::Result<()> {
    for row in list(conn)? {
        let Ok(mut layout) = serde_json::from_str::<serde_json::Value>(&row.layout) else {
            continue;
        };
        let Some(slots) = layout.get_mut("slots").and_then(|v| v.as_array_mut()) else {
            continue;
        };
        let mut changed = false;
        for slot in slots.iter_mut() {
            if slot.as_str() == Some(session_id) {
                *slot = serde_json::Value::Null;
                changed = true;
            }
        }
        if !changed {
            continue;
        }
        if slots.iter().all(serde_json::Value::is_null) {
            delete(conn, &row.id)?;
        } else {
            conn.execute(
                "UPDATE tabs SET layout = ?2 WHERE id = ?1",
                rusqlite::params![row.id, layout.to_string()],
            )?;
        }
    }
    Ok(())
}

pub fn delete_folder_tabs_and_archive(
    tx: &Transaction<'_>,
    folder_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let rows = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {} FROM tabs WHERE folder_id = ?1 ORDER BY position, created_at",
            select_list(COLUMNS)
        ))?;
        let rows = stmt
            .query_map([folder_id], |row| from_row(row).map_err(de_err))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let session_ids: HashSet<String> = rows.iter().flat_map(session_ids).collect();
    let archived_at = Utc::now().to_rfc3339();
    for id in &session_ids {
        let updated = tx.execute(
            "UPDATE sessions SET archived_at = ?2
             WHERE id = ?1 AND mission_id IS NULL AND slot_id IS NULL AND status != 'running'",
            rusqlite::params![id, archived_at],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "session {id} is missing or still running"
            )));
        }
    }
    tx.execute("DELETE FROM tabs WHERE folder_id = ?1", [folder_id])?;
    Ok(session_ids.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use crate::db;

    #[test]
    fn ensure_active_sessions_creates_stable_single_tabs_once() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, archived_at) VALUES ('s1', 'stopped', NULL)",
            [],
        )
        .unwrap();
        super::ensure_active_sessions(&conn).unwrap();
        let first = super::list(&conn).unwrap();
        super::ensure_active_sessions(&conn).unwrap();
        let second = super::list(&conn).unwrap();
        assert_eq!(first, second);
        assert_eq!(super::session_ids(&first[0]), ["s1"]);
    }

    #[test]
    fn concurrent_active_session_lists_seed_one_tab() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open_pool(&dir.path().join("runner.db")).unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, status, archived_at) VALUES ('s1', 'stopped', NULL)",
                [],
            )
            .unwrap();
        }

        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let pool = pool.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut conn = pool.get().unwrap();
                    barrier.wait();
                    super::list_with_active_sessions(&mut conn).unwrap()
                })
            })
            .collect();
        barrier.wait();
        for handle in handles {
            let rows = handle.join().unwrap();
            assert_eq!(
                rows.iter().flat_map(super::session_ids).collect::<Vec<_>>(),
                ["s1"]
            );
        }

        let conn = pool.get().unwrap();
        let rows = super::list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(super::session_ids(&rows[0]), ["s1"]);
    }

    #[test]
    fn remove_session_deletes_empty_tab_and_preserves_other_members() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let row = super::create(
            &conn,
            None,
            "pair",
            0,
            r#"{"preset":"cols-2","slots":["a","b"],"sizes":{}}"#,
        )
        .unwrap();
        super::remove_session(&conn, "a").unwrap();
        assert_eq!(
            super::session_ids(&super::get(&conn, &row.id).unwrap().unwrap()),
            ["b"]
        );
        super::remove_session(&conn, "b").unwrap();
        assert!(super::get(&conn, &row.id).unwrap().is_none());
    }

    #[test]
    fn folder_delete_archives_members_before_restricted_delete() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, archived_at) VALUES ('s1', 'stopped', NULL)",
            [],
        )
        .unwrap();
        let folder = crate::repo::folder::create(&conn, "Project").unwrap();
        super::create(
            &conn,
            Some(&folder.id),
            "",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        assert!(conn
            .execute("DELETE FROM folders WHERE id = ?1", [&folder.id])
            .is_err());

        let tx = conn.transaction().unwrap();
        let ids = super::delete_folder_tabs_and_archive(&tx, &folder.id).unwrap();
        assert_eq!(ids, ["s1"]);
        crate::repo::folder::delete_after_tabs(&tx, &folder.id).unwrap();
        tx.commit().unwrap();

        let archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM sessions WHERE id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(archived.is_some());
        assert!(crate::repo::folder::get(&conn, &folder.id)
            .unwrap()
            .is_none());
        assert!(super::list(&conn).unwrap().is_empty());
    }

    #[test]
    fn upsert_moves_sessions_out_of_other_tabs() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let source = super::create(
            &conn,
            None,
            "source",
            0,
            r#"{"preset":"cols-2","slots":["a","b"],"sizes":{}}"#,
        )
        .unwrap();
        let mut target = super::create(
            &conn,
            None,
            "target",
            1,
            r#"{"preset":"single","slots":[null],"sizes":{}}"#,
        )
        .unwrap();
        target.layout = r#"{"preset":"single","slots":["b"],"sizes":{}}"#.to_owned();
        super::upsert_move_not_copy(&conn, &target).unwrap();

        assert_eq!(
            super::session_ids(&super::get(&conn, &source.id).unwrap().unwrap()),
            ["a"]
        );
        assert_eq!(
            super::session_ids(&super::get(&conn, &target.id).unwrap().unwrap()),
            ["b"]
        );
    }

    #[test]
    fn move_and_reorder_changes_group_and_persists_exact_order() {
        let pool = db::open_in_memory().unwrap();
        let mut conn = pool.get().unwrap();
        let folder = crate::repo::folder::create(&conn, "Project").unwrap();
        let a = super::create(
            &conn,
            None,
            "A",
            0,
            r#"{"preset":"single","slots":["a"],"sizes":{}}"#,
        )
        .unwrap();
        let b = super::create(
            &conn,
            None,
            "B",
            1,
            r#"{"preset":"single","slots":["b"],"sizes":{}}"#,
        )
        .unwrap();
        let c = super::create(
            &conn,
            Some(&folder.id),
            "C",
            0,
            r#"{"preset":"single","slots":["c"],"sizes":{}}"#,
        )
        .unwrap();

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::move_and_reorder(&tx, &b.id, Some(&folder.id), &[b.id.clone(), c.id.clone()])
            .unwrap();
        tx.commit().unwrap();

        let rows = super::list(&conn).unwrap();
        let grouped: Vec<_> = rows
            .iter()
            .filter(|row| row.folder_id.as_deref() == Some(folder.id.as_str()))
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(grouped, [b.id.as_str(), c.id.as_str()]);
        assert_eq!(
            rows.iter()
                .filter(|row| row.folder_id.is_none())
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            [a.id.as_str()]
        );

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            super::move_and_reorder(&tx, &a.id, Some(&folder.id), std::slice::from_ref(&a.id),)
                .is_err()
        );
        drop(tx);
        assert!(super::get(&conn, &a.id)
            .unwrap()
            .unwrap()
            .folder_id
            .is_none());
    }
}
