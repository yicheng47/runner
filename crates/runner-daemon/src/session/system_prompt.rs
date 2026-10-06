use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

const DIRECTORY: &str = "session-prompts";

pub(crate) fn path(app_data_dir: &Path, session_id: &str) -> PathBuf {
    app_data_dir
        .join(DIRECTORY)
        .join(format!("{session_id}.md"))
}

pub(crate) fn write(app_data_dir: &Path, session_id: &str, body: &str) -> Result<PathBuf> {
    let path = path(app_data_dir, session_id);
    fs::create_dir_all(path.parent().expect("session prompt path has a parent"))?;
    fs::write(&path, body)?;
    Ok(path)
}

pub(crate) fn remove(app_data_dir: &Path, session_id: &str) {
    let path = path(app_data_dir, session_id);
    if let Err(error) = fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("remove session prompt {}: {error}", path.display());
        }
    }
}

pub(crate) fn clear_leftovers(app_data_dir: &Path) -> Result<()> {
    let directory = app_data_dir.join(DIRECTORY);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_files_are_written_removed_and_swept() {
        let app_data = tempfile::tempdir().unwrap();
        let first = write(app_data.path(), "one", "persona").unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), "persona");
        remove(app_data.path(), "one");
        assert!(!first.exists());

        let second = write(app_data.path(), "two", "worker").unwrap();
        let third = write(app_data.path(), "three", "lead").unwrap();
        clear_leftovers(app_data.path()).unwrap();
        assert!(!second.exists());
        assert!(!third.exists());
    }
}
