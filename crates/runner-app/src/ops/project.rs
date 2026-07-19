use crate::error::{Error, Result};
use crate::repo;
use crate::repo::project::ProjectRow;
use crate::AppCore;

fn clean_value(value: String, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::msg(format!("project {label} cannot be empty")));
    }
    Ok(value.to_owned())
}

fn emit_changed(state: &AppCore) {
    state.events.emit("project/changed", &serde_json::json!({}));
}

pub fn list(conn: &rusqlite::Connection) -> Result<Vec<ProjectRow>> {
    Ok(repo::project::list(conn)?)
}

pub fn get(conn: &rusqlite::Connection, id: &str) -> Result<ProjectRow> {
    repo::project::get(conn, id)?.ok_or_else(|| Error::msg(format!("project not found: {id}")))
}

pub(crate) fn resolve_cwd(
    conn: &rusqlite::Connection,
    project_id: Option<&str>,
    cwd: Option<String>,
) -> Result<Option<String>> {
    let Some(project_id) = project_id else {
        return Ok(cwd);
    };
    let project = get(conn, project_id)?;
    Ok(cwd.or(Some(project.cwd)))
}

pub fn project_list(state: &AppCore) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    list(&conn)
}

pub fn project_create(state: &AppCore, name: String, cwd: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    let row = repo::project::create(
        &conn,
        &clean_value(name, "name")?,
        &clean_value(cwd, "cwd")?,
    )?;
    emit_changed(state);
    Ok(row)
}

pub fn project_rename(state: &AppCore, id: String, name: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::rename(&conn, &id, &clean_value(name, "name")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(state);
    Ok(row)
}

pub fn project_set_cwd(state: &AppCore, id: String, cwd: String) -> Result<ProjectRow> {
    let conn = state.db.get()?;
    if repo::project::set_cwd(&conn, &id, &clean_value(cwd, "cwd")?)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    let row = repo::project::get(&conn, &id)?.ok_or_else(|| Error::msg("project disappeared"))?;
    emit_changed(state);
    Ok(row)
}

pub fn project_reorder(state: &AppCore, ordered_ids: Vec<String>) -> Result<Vec<ProjectRow>> {
    let conn = state.db.get()?;
    repo::project::reorder(&conn, &ordered_ids)?;
    let rows = repo::project::list(&conn)?;
    emit_changed(state);
    Ok(rows)
}

pub fn project_delete(state: &AppCore, id: String) -> Result<()> {
    let conn = state.db.get()?;
    if repo::project::delete(&conn, &id)? == 0 {
        return Err(Error::msg(format!("project not found: {id}")));
    }
    emit_changed(state);
    state.events.emit("mission/changed", &serde_json::json!({}));
    state.events.emit("session/updated", &serde_json::json!({}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clean_value, resolve_cwd};
    use crate::{db, repo};

    #[test]
    fn clean_value_trims_and_rejects_blank() {
        assert_eq!(clean_value("  Runner  ".into(), "name").unwrap(), "Runner");
        assert!(clean_value("  ".into(), "cwd").is_err());
    }

    #[test]
    fn resolve_cwd_defaults_from_project_and_preserves_override() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();
        let project = repo::project::create(&conn, "Runner", "/project").unwrap();

        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), None).unwrap(),
            Some("/project".into())
        );
        assert_eq!(
            resolve_cwd(&conn, Some(&project.id), Some("/override".into())).unwrap(),
            Some("/override".into())
        );
    }

    #[test]
    fn resolve_cwd_rejects_unknown_project() {
        let pool = db::open_in_memory().unwrap();
        let conn = pool.get().unwrap();

        let error = resolve_cwd(&conn, Some("missing"), None).unwrap_err();

        assert_eq!(error.to_string(), "project not found: missing");
    }
}
