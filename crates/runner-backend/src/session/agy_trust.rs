// Folder-trust preseed for Antigravity CLI (spec 644 decision 2). An
// untrusted cwd opens agy's "Do you trust the contents of this project?"
// dialog, which would park an unattended slot and races the `-i` first turn,
// so every spawn adds its exact directory to `trustedWorkspaces` in
// `~/.gemini/antigravity-cli/settings.json` first. Only that member's value is
// rewritten; every other byte of agy's file stays as agy wrote it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{Error, Result};

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());
const TRUSTED_WORKSPACES: &str = "trustedWorkspaces";

pub(crate) fn settings_path(home: &Path) -> PathBuf {
    home.join(".gemini/antigravity-cli/settings.json")
}

#[cfg(not(test))]
pub(crate) fn seed_project_trust(cwd: &Path) -> Result<()> {
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory is not available"))?;
    seed_project_trust_at(cwd, &settings_path(&home), Some(&home))
}

#[cfg(test)]
pub(crate) fn seed_project_trust(cwd: &Path) -> Result<()> {
    // Mocked session spawns must not touch the user's agy settings.
    match crate::router::runtime::test_home() {
        Some(home) => seed_project_trust_at(cwd, &settings_path(&home), None),
        None => Ok(()),
    }
}

pub(crate) fn seed_project_trust_at(
    cwd: &Path,
    settings: &Path,
    home: Option<&Path>,
) -> Result<()> {
    let cwd = std::fs::canonicalize(cwd)
        .map_err(|error| Error::msg(format!("realpath {}: {error}", cwd.display())))?;
    if is_broad_trust_root(&cwd, home) {
        log::debug!(
            "skipping broad Antigravity trust root: cwd={}",
            cwd.display()
        );
        return Ok(());
    }

    let _guard = SETTINGS_LOCK
        .lock()
        .map_err(|_| Error::msg("Antigravity trust settings lock poisoned"))?;
    let write_path = super::codex_trust::resolve_config_write_path(settings)?;
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
    let Some(contents) = with_trusted_workspace(&raw, &cwd.to_string_lossy())
        .map_err(|error| Error::msg(format!("{}: {error}", write_path.display())))?
    else {
        return Ok(());
    };
    super::codex_trust::write_config_atomically(&write_path, contents.as_bytes())
}

/// `raw` with `workspace` appended to `trustedWorkspaces`, or `None` when it
/// is already there.
fn with_trusted_workspace(raw: &str, workspace: &str) -> Result<Option<String>> {
    let fresh = || {
        let document = serde_json::json!({ TRUSTED_WORKSPACES: [workspace] });
        Ok(Some(format!(
            "{}\n",
            serde_json::to_string_pretty(&document)?
        )))
    };
    if raw.trim().is_empty() {
        return fresh();
    }
    let document: Value =
        serde_json::from_str(raw).map_err(|error| Error::msg(format!("parse: {error}")))?;
    let object = document
        .as_object()
        .ok_or_else(|| Error::msg("agy settings are not a JSON object"))?;
    let mut workspaces = match object.get(TRUSTED_WORKSPACES) {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| Error::msg("trustedWorkspaces is not an array"))?
            .clone(),
    };
    if workspaces
        .iter()
        .any(|entry| entry.as_str() == Some(workspace))
    {
        return Ok(None);
    }
    workspaces.push(Value::String(workspace.into()));
    let workspaces = Value::Array(workspaces);

    let (members, _) = crate::ops::mcp::json_members(raw)?;
    let mut out = raw.to_owned();
    if let Some(member) = members
        .iter()
        .find(|member| member.key == TRUSTED_WORKSPACES)
    {
        let indent = line_indent(raw, member.key_start);
        out.replace_range(member.value.clone(), &render(&workspaces, indent)?);
    } else if let Some(last) = members.last() {
        let indent = line_indent(raw, last.key_start);
        let (separator, colon) = match indent {
            Some(indent) => (format!(",\n{indent}"), ": "),
            None => (",".into(), ":"),
        };
        out.insert_str(
            last.value.end,
            &format!(
                "{separator}{}{colon}{}",
                serde_json::to_string(TRUSTED_WORKSPACES)?,
                render(&workspaces, indent)?
            ),
        );
    } else {
        return fresh();
    }
    Ok(Some(out))
}

/// The whitespace a member's line starts with, or `None` when the member
/// shares its line with other JSON (a compact file).
fn line_indent(raw: &str, member_start: usize) -> Option<&str> {
    let line_start = raw[..member_start].rfind('\n')? + 1;
    let indent = &raw[line_start..member_start];
    indent.chars().all(char::is_whitespace).then_some(indent)
}

/// Pretty-printed at the member's own indent, like agy's own writer, or
/// compact when the file is.
fn render(value: &Value, indent: Option<&str>) -> Result<String> {
    Ok(match indent {
        Some(indent) => serde_json::to_string_pretty(value)?.replace('\n', &format!("\n{indent}")),
        None => serde_json::to_string(value)?,
    })
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

    fn project(temp: &tempfile::TempDir) -> PathBuf {
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::canonicalize(cwd).unwrap()
    }

    #[test]
    fn seeds_a_missing_or_empty_file_with_the_canonical_cwd() {
        for existing in [None, Some(""), Some("\n")] {
            let temp = tempfile::tempdir().unwrap();
            let settings = settings_path(temp.path());
            if let Some(raw) = existing {
                std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
                std::fs::write(&settings, raw).unwrap();
            }
            let cwd = project(&temp);
            seed_project_trust_at(&cwd, &settings, None).unwrap();
            assert_eq!(
                std::fs::read_to_string(&settings).unwrap(),
                format!(
                    "{{\n  \"trustedWorkspaces\": [\n    {}\n  ]\n}}\n",
                    serde_json::to_string(&cwd.to_string_lossy()).unwrap()
                )
            );
        }
    }

    #[test]
    fn appends_to_agys_own_layout_and_keeps_every_other_byte() {
        let raw = "{\n  \"model\": \"Gemini 3.8 Flash (High)\",\n  \"trustedWorkspaces\": [\n    \"/Users/jason\",\n    \"/Users/jason/repos\"\n  ],\n  \"toolPermission\": \"request-review\",\n  \"n\": 1e2\n}";
        assert_eq!(
            with_trusted_workspace(raw, "/Users/jason/repos/runner")
                .unwrap()
                .unwrap(),
            "{\n  \"model\": \"Gemini 3.8 Flash (High)\",\n  \"trustedWorkspaces\": [\n    \"/Users/jason\",\n    \"/Users/jason/repos\",\n    \"/Users/jason/repos/runner\"\n  ],\n  \"toolPermission\": \"request-review\",\n  \"n\": 1e2\n}"
        );
    }

    #[test]
    fn adds_the_member_after_the_last_one_when_absent() {
        let raw = "{\n  \"model\": \"x\",\n  \"toolPermission\": \"strict\"\n}\n";
        assert_eq!(
            with_trusted_workspace(raw, "/p").unwrap().unwrap(),
            "{\n  \"model\": \"x\",\n  \"toolPermission\": \"strict\",\n  \"trustedWorkspaces\": [\n    \"/p\"\n  ]\n}\n"
        );
        assert_eq!(
            with_trusted_workspace("{\"a\":{\"b\":[1]},\"trustedWorkspaces\":[\"/o\"]}", "/p")
                .unwrap()
                .unwrap(),
            "{\"a\":{\"b\":[1]},\"trustedWorkspaces\":[\"/o\",\"/p\"]}"
        );
        assert_eq!(
            with_trusted_workspace("{\"a\": true}", "/p")
                .unwrap()
                .unwrap(),
            "{\"a\": true,\"trustedWorkspaces\":[\"/p\"]}"
        );
        assert_eq!(
            with_trusted_workspace("{ }", "/p").unwrap().unwrap(),
            "{\n  \"trustedWorkspaces\": [\n    \"/p\"\n  ]\n}\n"
        );
    }

    #[test]
    fn already_trusted_is_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings_path(temp.path());
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let cwd = project(&temp);
        let raw = format!(
            "{{ \"trustedWorkspaces\" : [{}], \"other\" : 1e2 }}",
            serde_json::to_string(&cwd.to_string_lossy()).unwrap()
        );
        std::fs::write(&settings, &raw).unwrap();
        seed_project_trust_at(&cwd, &settings, None).unwrap();
        assert_eq!(std::fs::read_to_string(settings).unwrap(), raw);
    }

    #[test]
    fn malformed_settings_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings_path(temp.path());
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let cwd = project(&temp);
        for raw in ["{broken", "[]", "{\"trustedWorkspaces\":false}"] {
            std::fs::write(&settings, raw).unwrap();
            assert!(seed_project_trust_at(&cwd, &settings, None).is_err());
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), raw);
        }
    }

    #[test]
    fn home_and_filesystem_root_are_rejected_as_too_broad() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let settings = settings_path(&home);
        seed_project_trust_at(&home, &settings, Some(&home)).unwrap();
        assert!(!settings.exists());
        assert!(is_broad_trust_root(Path::new("/"), None));
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_cwd_seeds_realpath() {
        let temp = tempfile::tempdir().unwrap();
        let target = project(&temp);
        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let settings = settings_path(temp.path());
        seed_project_trust_at(&linked, &settings, None).unwrap();
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        assert_eq!(
            value[TRUSTED_WORKSPACES],
            serde_json::json!([target.to_string_lossy()])
        );
    }
}
