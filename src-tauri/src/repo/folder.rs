use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_row;

use super::{de_err, select_list};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderRow {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub collapsed: bool,
    pub created_at: String,
}

const COLUMNS: &[&str] = &["id", "name", "position", "collapsed", "created_at"];

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<FolderRow>> {
    let sql = format!(
        "SELECT {} FROM folders ORDER BY position, created_at",
        select_list(COLUMNS)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], |row| from_row(row).map_err(de_err))?
        .collect();
    rows
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<FolderRow>> {
    let sql = format!("SELECT {} FROM folders WHERE id = ?1", select_list(COLUMNS));
    conn.query_row(&sql, [id], |row| from_row(row).map_err(de_err))
        .optional()
}

pub fn create(conn: &Connection, name: &str) -> rusqlite::Result<FolderRow> {
    let id = ulid::Ulid::new().to_string();
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM folders",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO folders (id, name, position, collapsed, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        rusqlite::params![id, name, position, Utc::now().to_rfc3339()],
    )?;
    get(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn rename(conn: &Connection, id: &str, name: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE folders SET name = ?2 WHERE id = ?1",
        rusqlite::params![id, name],
    )
}

pub fn set_collapsed(conn: &Connection, id: &str, collapsed: bool) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE folders SET collapsed = ?2 WHERE id = ?1",
        rusqlite::params![id, collapsed],
    )
}

pub fn reorder(conn: &Connection, ordered_ids: &[String]) -> rusqlite::Result<()> {
    for (position, id) in ordered_ids.iter().enumerate() {
        conn.execute(
            "UPDATE folders SET position = ?2 WHERE id = ?1",
            rusqlite::params![id, position as i64],
        )?;
    }
    Ok(())
}

pub fn delete_after_tabs(tx: &Transaction<'_>, id: &str) -> rusqlite::Result<usize> {
    tx.execute("DELETE FROM folders WHERE id = ?1", [id])
}

#[cfg(test)]
mod tests {
    use crate::db;

    #[test]
    fn folders_keep_position_and_collapse_state() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let a = super::create(&conn, "A").unwrap();
        let b = super::create(&conn, "B").unwrap();
        super::set_collapsed(&conn, &b.id, true).unwrap();
        super::reorder(&conn, &[b.id.clone(), a.id.clone()]).unwrap();
        let rows = super::list(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert!(rows[0].collapsed);
    }
}
