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
use crate::runtimes::helpers::is_broad_trust_root;

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
    match crate::runtimes::test_home() {
        Some(home) => seed_project_trust_at(cwd, &settings_path(&home), None),
        None => Ok(()),
    }
}

pub(crate) fn seed_project_trust_at(
    cwd: &Path,
    settings: &Path,
    home: Option<&Path>,
) -> Result<()> {
    let canonical = std::fs::canonicalize(cwd)
        .map_err(|error| Error::msg(format!("realpath {}: {error}", cwd.display())))?;
    let normalized = normalize_non_canonical(cwd)?;
    if is_broad_trust_root(&canonical, home) || is_broad_trust_root(&normalized, home) {
        log::debug!(
            "skipping broad Antigravity trust root: cwd={}",
            cwd.display()
        );
        return Ok(());
    }
    let normalized_text = normalized.to_string_lossy();
    let canonical_text = canonical.to_string_lossy();
    let candidates: Vec<&str> = if normalized != canonical {
        vec![normalized_text.as_ref(), canonical_text.as_ref()]
    } else {
        vec![canonical_text.as_ref()]
    };

    let _guard = SETTINGS_LOCK
        .lock()
        .map_err(|_| Error::msg("Antigravity trust settings lock poisoned"))?;
    let write_path = crate::runtimes::helpers::resolve_config_write_path(settings)?;
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
    let Some(contents) = with_trusted_workspaces(&raw, &candidates)
        .map_err(|error| Error::msg(format!("{}: {error}", write_path.display())))?
    else {
        return Ok(());
    };
    crate::runtimes::helpers::write_config_atomically(&write_path, contents.as_bytes())
}

fn normalize_non_canonical(cwd: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(cwd)
        .map_err(|error| Error::msg(format!("absolute {}: {error}", cwd.display())))?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => {
                normalized.push(std::path::Component::RootDir.as_os_str());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
fn with_trusted_workspace(raw: &str, workspace: &str) -> Result<Option<String>> {
    with_trusted_workspaces(raw, &[workspace])
}

/// `raw` with each missing entry from `new_workspaces` appended to
/// `trustedWorkspaces`, or `None` when all of them are already there.
fn with_trusted_workspaces(raw: &str, new_workspaces: &[&str]) -> Result<Option<String>> {
    let mut unique = Vec::with_capacity(new_workspaces.len());
    for workspace in new_workspaces {
        if !unique.contains(workspace) {
            unique.push(*workspace);
        }
    }
    if unique.is_empty() {
        return Ok(None);
    }
    let fresh = |entries: &[&str]| {
        let document = serde_json::json!({ TRUSTED_WORKSPACES: entries });
        Ok(Some(format!(
            "{}\n",
            serde_json::to_string_pretty(&document)?
        )))
    };
    if raw.trim().is_empty() {
        return fresh(&unique);
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
    let mut added = false;
    for workspace in unique {
        if !workspaces
            .iter()
            .any(|entry| entry.as_str() == Some(workspace))
        {
            workspaces.push(Value::String(workspace.into()));
            added = true;
        }
    }
    if !added {
        return Ok(None);
    }
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
        let entries: Vec<&str> = workspaces
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        return fresh(&entries);
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
    fn home_is_rejected_as_too_broad() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let settings = settings_path(&home);
        seed_project_trust_at(&home, &settings, Some(&home)).unwrap();
        assert!(!settings.exists());
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_cwd_seeds_both_normalized_and_realpath_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let target = project(&temp);
        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let expected_linked = normalize_non_canonical(&linked).unwrap();
        assert_ne!(expected_linked, target);

        let settings = settings_path(temp.path());
        seed_project_trust_at(&linked, &settings, None).unwrap();
        let first = std::fs::read_to_string(&settings).unwrap();
        let value: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(
            value[TRUSTED_WORKSPACES],
            serde_json::json!([expected_linked.to_string_lossy(), target.to_string_lossy()])
        );

        // Repeat with the same symlink (and trailing `./`) leaves the file byte-for-byte identical.
        seed_project_trust_at(&linked.join("."), &settings, None).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), first);
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_cwd_appends_missing_entry_without_disturbing_other_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let target = project(&temp);
        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let expected_linked = normalize_non_canonical(&linked).unwrap();

        let settings = settings_path(temp.path());
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let raw = format!(
            "{{\n  \"model\": \"x\",\n  \"trustedWorkspaces\": [\n    {}\n  ],\n  \"n\": 1e2\n}}\n",
            serde_json::to_string(&target.to_string_lossy()).unwrap()
        );
        std::fs::write(&settings, &raw).unwrap();

        seed_project_trust_at(&linked, &settings, None).unwrap();
        let updated = std::fs::read_to_string(&settings).unwrap();
        assert!(updated.contains("\"n\": 1e2"));
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(
            value[TRUSTED_WORKSPACES],
            serde_json::json!([target.to_string_lossy(), expected_linked.to_string_lossy()])
        );
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_home_and_root_are_rejected_as_broad_roots() {
        let temp = tempfile::tempdir().unwrap();
        let real_home = temp.path().join("real-home");
        std::fs::create_dir_all(&real_home).unwrap();
        let home_link = temp.path().join("home-link");
        std::os::unix::fs::symlink(&real_home, &home_link).unwrap();
        let cwd_link = temp.path().join("cwd-to-home");
        std::os::unix::fs::symlink(&real_home, &cwd_link).unwrap();

        let settings = settings_path(&real_home);
        seed_project_trust_at(&cwd_link, &settings, Some(&real_home)).unwrap();
        seed_project_trust_at(&cwd_link, &settings, Some(&home_link)).unwrap();
        seed_project_trust_at(&real_home, &settings, Some(&home_link)).unwrap();

        let root_link = temp.path().join("root-link");
        std::os::unix::fs::symlink("/", &root_link).unwrap();
        seed_project_trust_at(&root_link, &settings, Some(&real_home)).unwrap();
        assert!(!settings.exists());
    }
}
