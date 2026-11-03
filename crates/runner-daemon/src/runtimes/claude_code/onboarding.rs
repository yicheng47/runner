use std::{path::Path, sync::Mutex};

use runner_core::protocol::ClaudeAccount;

use crate::error::{Error, Result};

pub(super) static CONFIG_LOCK: Mutex<()> = Mutex::new(());
const COMPLETED: &str = "hasCompletedOnboarding";

pub(crate) fn complete(account: &ClaudeAccount, home: &Path) -> Result<()> {
    if account.native {
        return Ok(());
    }
    let folder = std::fs::canonicalize(&account.folder)?;
    let home = std::fs::canonicalize(home)?;
    let native = home.join(".claude");
    let native = std::fs::canonicalize(&native).unwrap_or(native);
    let native_config = home.join(".claude.json");
    let native_config = std::fs::canonicalize(&native_config).unwrap_or(native_config);
    let _guard = CONFIG_LOCK.lock().unwrap();
    let path = super::super::helpers::resolve_config_write_path(&folder.join(".claude.json"))?;
    if folder.starts_with(&native) || path.starts_with(&native) || path == native_config {
        return Err(Error::msg(
            "cannot preseed the native Claude Code configuration",
        ));
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if let Some(contents) = with_completed_onboarding(&raw)? {
        super::super::helpers::write_config_atomically(&path, contents.as_bytes())?;
    }
    Ok(())
}

fn with_completed_onboarding(raw: &str) -> Result<Option<String>> {
    if raw.trim().is_empty() {
        return Ok(Some(format!("{{\"{COMPLETED}\":true}}\n")));
    }
    let document: serde_json::Value = serde_json::from_str(raw)?;
    let object = document
        .as_object()
        .ok_or_else(|| Error::msg("Claude Code configuration is not a JSON object"))?;
    if object.get(COMPLETED).and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let (members, close) = crate::ops::mcp::json_members(raw)?;
    let mut contents = raw.to_owned();
    if let Some(member) = members.iter().rfind(|member| member.key == COMPLETED) {
        contents.replace_range(member.value.clone(), "true");
    } else {
        contents.insert_str(close, &format!("\"{COMPLETED}\":true"));
        if let Some(last) = members.last() {
            contents.insert(last.value.end, ',');
        }
    }
    Ok(Some(contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(folder: &Path, native: bool) -> ClaudeAccount {
        ClaudeAccount {
            id: "fixture".into(),
            name: "Fixture".into(),
            folder: folder.to_string_lossy().into_owned(),
            native,
        }
    }

    #[test]
    fn changes_only_the_onboarding_value_and_preserves_other_bytes() {
        let raw = " {\r\n  \"before\":1e+02,\r\n  \"hasCompletedOnboarding\" : false,\r\n  \"after\":{\"escaped\":\"\\u0041\", \"array\":[ 1, 2 ]}\r\n}\r\n";
        assert_eq!(
            with_completed_onboarding(raw).unwrap().unwrap(),
            raw.replace("false", "true")
        );
        let missing = " {\"before\":1e+02,\"after\":\"\\u0041\"  }\r\n";
        assert_eq!(
            with_completed_onboarding(missing).unwrap().unwrap(),
            " {\"before\":1e+02,\"after\":\"\\u0041\",  \"hasCompletedOnboarding\":true}\r\n"
        );
        assert!(
            with_completed_onboarding("{\"hasCompletedOnboarding\":true,\"other\":1e+02}\n")
                .unwrap()
                .is_none()
        );
        assert!(with_completed_onboarding("[]").is_err());
        assert!(with_completed_onboarding("invalid").is_err());
    }

    #[test]
    fn seeds_only_the_completion_flag_and_leaves_completed_files_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("work folder");
        std::fs::create_dir(&folder).unwrap();
        let account = account(&folder, false);
        complete(&account, temp.path()).unwrap();
        let path = folder.join(".claude.json");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"hasCompletedOnboarding\":true}\n"
        );
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        complete(&account, temp.path()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        let invalid = "{ malformed user file }";
        std::fs::write(&path, invalid).unwrap();
        assert!(complete(&account, temp.path()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    }

    #[test]
    fn never_writes_the_native_account_or_folder() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join(".claude");
        std::fs::create_dir(&native).unwrap();
        let original = "{\"nativeUserValue\":1e+02}\n";
        std::fs::write(temp.path().join(".claude.json"), original).unwrap();
        complete(&account(&native, true), temp.path()).unwrap();
        assert!(complete(&account(&native, false), temp.path()).is_err());
        assert!(!native.join(".claude.json").exists());
        assert_eq!(
            std::fs::read_to_string(temp.path().join(".claude.json")).unwrap(),
            original
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_links_to_the_native_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let native = temp.path().join(".claude");
        let work = temp.path().join("work");
        std::fs::create_dir(&native).unwrap();
        std::fs::create_dir(&work).unwrap();
        let global = temp.path().join(".claude.json");
        std::fs::write(&global, "{\"preserve\":true}\n").unwrap();
        std::os::unix::fs::symlink(&global, work.join(".claude.json")).unwrap();
        assert!(complete(&account(&work, false), temp.path()).is_err());
        let alias = temp.path().join("native-alias");
        std::os::unix::fs::symlink(&native, &alias).unwrap();
        assert!(complete(&account(&alias, false), temp.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(&global).unwrap(),
            "{\"preserve\":true}\n"
        );
        assert!(!native.join(".claude.json").exists());
    }
}
