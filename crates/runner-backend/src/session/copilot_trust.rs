use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn copilot_home(override_home: Option<&str>) -> Result<PathBuf> {
    if let Some(home) = override_home
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("COPILOT_HOME").map(PathBuf::from))
    {
        return Ok(home);
    }
    runner_core::app_paths::home_dir()
        .map(|home| home.join(".copilot"))
        .ok_or_else(|| Error::msg("home directory is not available"))
}

#[cfg(not(test))]
pub(crate) fn seed_project_trust(cwd: &Path, override_home: Option<&str>) -> Result<()> {
    let config_path = copilot_home(override_home)?.join("config.json");
    let home = runner_core::app_paths::home_dir();
    seed_project_trust_at_with_home(cwd, &config_path, home.as_deref())
}

#[cfg(test)]
pub(crate) fn seed_project_trust(cwd: &Path, override_home: Option<&str>) -> Result<()> {
    // Mocked session spawns must not touch the user's Copilot config.
    if let Some(home) = override_home {
        return seed_project_trust_at_with_home(cwd, &Path::new(home).join("config.json"), None);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn seed_project_trust_at(cwd: &Path, config_path: &Path) -> Result<()> {
    seed_project_trust_at_with_home(cwd, config_path, None)
}

fn seed_project_trust_at_with_home(
    cwd: &Path,
    config_path: &Path,
    home: Option<&Path>,
) -> Result<()> {
    let cwd = std::fs::canonicalize(cwd)
        .map_err(|error| Error::msg(format!("realpath {}: {error}", cwd.display())))?;
    if is_broad_trust_root(&cwd, home) {
        log::debug!(
            "skipping broad Copilot project trust root: cwd={}",
            cwd.display()
        );
        return Ok(());
    }

    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| Error::msg("copilot trust config lock poisoned"))?;
    let write_path = super::codex_trust::resolve_config_write_path(config_path)?;
    let raw = match std::fs::read_to_string(&write_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(Error::msg(format!(
                "read {}: {error}",
                write_path.display()
            )))
        }
    };
    let header_len: usize = raw
        .split_inclusive('\n')
        .take_while(|line| line.trim_start().starts_with("//"))
        .map(str::len)
        .sum();
    let (header, body) = raw.split_at(header_len);
    let mut document = if body.trim().is_empty() {
        serde_json::json!({})
    } else {
        crate::runtime_defaults::jsonc_document(body)
            .map_err(|error| Error::msg(format!("parse {}: {error}", write_path.display())))?
    };
    let object = document
        .as_object_mut()
        .ok_or_else(|| Error::msg("Copilot config is not a JSON object"))?;
    let folders = object
        .entry("trustedFolders")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| Error::msg("Copilot trustedFolders is not an array"))?;
    let cwd = cwd.to_string_lossy();
    if folders
        .iter()
        .any(|folder| folder.as_str() == Some(cwd.as_ref()))
    {
        return Ok(());
    }
    folders.push(serde_json::Value::String(cwd.into_owned()));
    let contents = format!("{header}{}\n", serde_json::to_string_pretty(&document)?);
    super::codex_trust::write_config_atomically(&write_path, contents.as_bytes())
}

fn is_broad_trust_root(cwd: &Path, home: Option<&Path>) -> bool {
    if cwd.parent().is_none() {
        return true;
    }
    home.map(|home| std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()))
        .as_deref()
        == Some(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_home_wins_over_the_process_home() {
        assert_eq!(
            copilot_home(Some("/temporary/copilot")).unwrap(),
            PathBuf::from("/temporary/copilot")
        );
    }

    #[test]
    fn seeds_missing_config_with_canonical_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("copilot/config.json");
        let cwd = temp.path().join("project/child");
        std::fs::create_dir_all(&cwd).unwrap();
        seed_project_trust_at(&cwd, &config).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"trustedFolders": [cwd.to_string_lossy()]})
        );
    }

    #[test]
    fn preserves_comment_header_key_order_and_other_values() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let header = "// managed automatically\r\n// keep this header  \r\n";
        std::fs::write(&config, format!("{header}{{\"before\":{{\"url\":\"https://example.com\"}},\"trustedFolders\":[\"/old\"],\"after\":true}}\n")).unwrap();
        seed_project_trust_at(&cwd, &config).unwrap();
        let raw = std::fs::read_to_string(&config).unwrap();
        assert!(raw.starts_with(header));
        let value = crate::runtime_defaults::jsonc_document(&raw).unwrap();
        assert_eq!(
            value["trustedFolders"],
            serde_json::json!(["/old", std::fs::canonicalize(cwd).unwrap()])
        );
        assert_eq!(
            value["before"],
            serde_json::json!({"url":"https://example.com"})
        );
        assert_eq!(value["after"], true);
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["before", "trustedFolders", "after"]
        );
    }

    #[test]
    fn already_trusted_is_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let raw = format!(
            "// keep\n{{ \"trustedFolders\" : [{}], \"other\" : 1e2 }}\n",
            serde_json::to_string(&cwd.to_string_lossy()).unwrap()
        );
        std::fs::write(&config, &raw).unwrap();
        seed_project_trust_at(&cwd, &config).unwrap();
        assert_eq!(std::fs::read_to_string(config).unwrap(), raw);
    }

    #[test]
    fn malformed_config_is_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.json");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        for raw in ["// keep\n{broken", "[]", "{\"trustedFolders\":false}"] {
            std::fs::write(&config, raw).unwrap();
            assert!(seed_project_trust_at(&cwd, &config).is_err());
            assert_eq!(std::fs::read_to_string(&config).unwrap(), raw);
        }
    }

    #[test]
    fn home_and_filesystem_root_are_rejected_as_too_broad() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = home.join(".copilot/config.json");
        std::fs::create_dir_all(&home).unwrap();

        seed_project_trust_at_with_home(&home, &config, Some(&home)).unwrap();

        assert!(!config.exists());
        assert!(is_broad_trust_root(Path::new("/"), None));
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_cwd_seeds_realpath() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let linked = temp.path().join("linked");
        let config = temp.path().join("config.json");
        std::fs::create_dir_all(&target).unwrap();
        symlink(&target, &linked).unwrap();

        seed_project_trust_at(&linked, &config).unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        assert_eq!(
            value["trustedFolders"],
            serde_json::json!([std::fs::canonicalize(target).unwrap()])
        );
    }
}
