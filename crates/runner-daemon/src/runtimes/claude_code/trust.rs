use std::ffi::OsString;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};
use crate::runtimes::helpers::{
    is_broad_trust_root, resolve_config_write_path, resolve_project_trust_root,
    write_config_atomically,
};

const ACCEPTED: &str = "hasTrustDialogAccepted";
const TRUSTED_PROJECT: &str = concat!(
    "{\"allowedTools\":[],\"mcpContextUris\":[],\"mcpServers\":{},",
    "\"enabledMcpjsonServers\":[],\"disabledMcpjsonServers\":[],",
    "\"hasTrustDialogAccepted\":true,\"hasClaudeMdExternalIncludesApproved\":false,",
    "\"hasClaudeMdExternalIncludesWarningShown\":false}"
);

pub(super) fn seed_project_trust(cwd: &Path, config_dir: Option<&str>) -> Result<()> {
    #[cfg(test)]
    let Some(home) = crate::runtimes::test_home() else {
        // Mocked spawns must never write to a real Claude account.
        return Ok(());
    };
    #[cfg(not(test))]
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory is not available"))?;

    #[cfg(not(test))]
    let inherited = std::env::var_os("CLAUDE_CONFIG_DIR");
    #[cfg(test)]
    let inherited = None;
    let config_path = config_path(cwd, &home, config_dir, inherited);
    seed_project_trust_at(cwd, &config_path, Some(&home))
}

fn config_path(
    cwd: &Path,
    home: &Path,
    config_dir: Option<&str>,
    inherited: Option<OsString>,
) -> PathBuf {
    let dir = config_dir
        .map(OsString::from)
        .or(inherited)
        .filter(|dir| !dir.is_empty());
    match dir {
        Some(dir) => cwd.join(dir).join(".claude.json"),
        None => home.join(".claude.json"),
    }
}

fn project_key(root: &Path) -> String {
    #[cfg(windows)]
    let root = crate::runtimes::helpers::ordinary_windows_path(root);
    let key = root.to_string_lossy().into_owned();
    #[cfg(windows)]
    let key = key.replace('\\', "/");
    key.nfc().collect()
}

fn seed_project_trust_at(cwd: &Path, config_path: &Path, home: Option<&Path>) -> Result<()> {
    let root = resolve_project_trust_root(cwd)?;
    if is_broad_trust_root(&root, home) {
        log::debug!(
            "skipping broad Claude Code project trust root: cwd={} root={}",
            cwd.display(),
            root.display()
        );
        return Ok(());
    }
    let key = project_key(&root);
    let _guard = super::onboarding::CONFIG_LOCK
        .lock()
        .map_err(|_| Error::msg("Claude Code config lock poisoned"))?;
    let path = resolve_config_write_path(config_path)?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if let Some(contents) = with_project_trust(&raw, &key)? {
        write_config_atomically(&path, contents.as_bytes())?;
    }
    Ok(())
}

fn with_project_trust(raw: &str, key: &str) -> Result<Option<String>> {
    let raw = if raw.trim().is_empty() { "{}\n" } else { raw };
    let document: serde_json::Value = serde_json::from_str(raw)?;
    let object = document
        .as_object()
        .ok_or_else(|| Error::msg("Claude Code configuration is not a JSON object"))?;
    if let Some(projects) = object.get("projects") {
        let projects = projects
            .as_object()
            .ok_or_else(|| Error::msg("Claude Code projects is not a JSON object"))?;
        if let Some(project) = projects.get(key) {
            let project = project
                .as_object()
                .ok_or_else(|| Error::msg("Claude Code project is not a JSON object"))?;
            if project.get(ACCEPTED).and_then(serde_json::Value::as_bool) == Some(true) {
                return Ok(None);
            }
        }
    }
    let (members, _) = crate::ops::mcp::json_members(raw)?;
    let projects = members
        .iter()
        .rfind(|member| member.key == "projects")
        .map(|member| &raw[member.value.clone()])
        .unwrap_or("{}");
    let (members, _) = crate::ops::mcp::json_members(projects)?;
    let project = match members.iter().rfind(|member| member.key == key) {
        Some(member) => set_member(&projects[member.value.clone()], ACCEPTED, "true")?,
        None => TRUSTED_PROJECT.to_owned(),
    };
    let projects = set_member(projects, key, &project)?;
    Ok(Some(set_member(raw, "projects", &projects)?))
}

fn set_member(raw: &str, key: &str, value: &str) -> Result<String> {
    let (members, close) = crate::ops::mcp::json_members(raw)?;
    let mut contents = raw.to_owned();
    if let Some(member) = members.iter().rfind(|member| member.key == key) {
        contents.replace_range(member.value.clone(), value);
    } else {
        contents.insert_str(close, &format!("{}:{value}", serde_json::to_string(key)?));
        if let Some(last) = members.last() {
            contents.insert(last.value.end, ',');
        }
    }
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Barrier};

    fn project(config: &Path, cwd: &Path) -> serde_json::Value {
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config).unwrap()).unwrap();
        value["projects"][project_key(cwd)].clone()
    }

    #[test]
    fn project_keys_match_claudes_nfc_unicode_spelling() {
        assert_eq!(
            project_key(Path::new("/project/cafe\u{301}")),
            "/project/café"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_project_keys_match_claudes_forward_slash_spelling() {
        assert_eq!(
            project_key(Path::new(r"\\?\C:\repo\project")),
            "C:/repo/project"
        );
        assert_eq!(
            project_key(Path::new(r"\\?\UNC\server\share\repo")),
            "//server/share/repo"
        );
    }

    #[test]
    fn empty_config_directory_uses_native_without_falling_through() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let home = temp.path().join("home");
        fs::create_dir(&cwd).unwrap();
        let daemon_dir = temp.path().join("daemon-account");
        for (explicit, inherited) in [
            (Some(""), Some(daemon_dir.clone().into_os_string())),
            (None, Some(OsString::new())),
        ] {
            let path = config_path(&cwd, &home, explicit, inherited);
            assert_eq!(path, home.join(".claude.json"));
            seed_project_trust_at(&cwd, &path, Some(&home)).unwrap();
            assert_eq!(
                project(&path, &fs::canonicalize(&cwd).unwrap())[ACCEPTED],
                true
            );
        }
        assert!(!cwd.join(".claude.json").exists());
        assert!(!daemon_dir.exists());
    }

    #[test]
    fn missing_config_and_parent_get_claudes_full_project_shape() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let config = temp.path().join("account/missing/.claude.json");
        seed_project_trust_at(&cwd, &config, None).unwrap();
        let expected = serde_json::json!({
            "allowedTools": [],
            "mcpContextUris": [],
            "mcpServers": {},
            "enabledMcpjsonServers": [],
            "disabledMcpjsonServers": [],
            "hasTrustDialogAccepted": true,
            "hasClaudeMdExternalIncludesApproved": false,
            "hasClaudeMdExternalIncludesWarningShown": false,
        });
        assert_eq!(project(&config, &fs::canonicalize(cwd).unwrap()), expected);
    }

    #[test]
    fn empty_configs_and_missing_project_maps_are_seeded() {
        for raw in [
            "",
            " \r\n",
            "{}",
            " { \"other\":1e+02  }\r\n",
            "{\"projects\":{}}",
        ] {
            let updated = with_project_trust(raw, "/project").unwrap().unwrap();
            let value: serde_json::Value = serde_json::from_str(&updated).unwrap();
            assert_eq!(value["projects"]["/project"][ACCEPTED], true);
            if raw.contains("other") {
                assert!(updated.contains("\"other\":1e+02"));
            }
        }
    }

    #[test]
    fn repository_subdirectory_seeds_repository_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let cwd = root.join("src/nested");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        let config = temp.path().join(".claude.json");
        seed_project_trust_at(&cwd, &config, None).unwrap();
        assert_eq!(
            project(&config, &fs::canonicalize(root).unwrap())[ACCEPTED],
            true
        );
        assert!(project(&config, &fs::canonicalize(cwd).unwrap()).is_null());
    }

    #[test]
    fn linked_worktree_subdirectory_seeds_main_root_only() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let worktree = temp.path().join("feature");
        let git_dir = main.join(".git/worktrees/feature");
        let cwd = worktree.join("src/nested");
        fs::create_dir_all(&git_dir).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        fs::write(
            git_dir.join("gitdir"),
            format!("{}\n", worktree.join(".git").display()),
        )
        .unwrap();
        let config = temp.path().join(".claude.json");
        seed_project_trust_at(&cwd, &config, None).unwrap();
        assert_eq!(
            project(&config, &fs::canonicalize(main).unwrap())[ACCEPTED],
            true
        );
        assert!(project(&config, &fs::canonicalize(worktree).unwrap()).is_null());
        assert!(project(&config, &fs::canonicalize(cwd).unwrap()).is_null());
    }

    #[test]
    fn forged_gitdir_does_not_widen_trust() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let forged = temp.path().join("forged/.git/worktrees/feature");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&forged).unwrap();
        fs::write(cwd.join(".git"), format!("gitdir: {}\n", forged.display())).unwrap();
        let config = temp.path().join(".claude.json");
        seed_project_trust_at(&cwd, &config, None).unwrap();
        assert_eq!(
            project(&config, &fs::canonicalize(cwd).unwrap())[ACCEPTED],
            true
        );
        assert!(project(
            &config,
            &fs::canonicalize(temp.path().join("forged")).unwrap()
        )
        .is_null());
    }

    #[test]
    fn home_and_filesystem_root_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir(&home).unwrap();
        let config = home.join(".claude.json");
        seed_project_trust_at(&home, &config, Some(&home)).unwrap();
        let canonical = fs::canonicalize(&home).unwrap();
        let root = canonical.ancestors().last().unwrap();
        seed_project_trust_at(root, &config, Some(&home)).unwrap();
        assert!(!config.exists());
        fs::create_dir(home.join(".git")).unwrap();
        fs::create_dir(home.join("child")).unwrap();
        seed_project_trust_at(&home.join("child"), &config, Some(&home)).unwrap();
        assert!(!config.exists());
    }

    #[test]
    fn existing_true_is_untouched_including_file_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let key = serde_json::to_string(&project_key(&fs::canonicalize(&cwd).unwrap())).unwrap();
        let raw =
            format!(" {{\"projects\":{{{key}:{{\"{ACCEPTED}\" : true, \"other\":1e+02}}}}}}\r\n");
        let config = temp.path().join(".claude.json");
        fs::write(&config, &raw).unwrap();
        let before = fs::metadata(&config).unwrap().modified().unwrap();
        seed_project_trust_at(&cwd, &config, None).unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), raw);
        assert_eq!(fs::metadata(config).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn existing_false_changes_only_the_flag_bytes() {
        let raw = " {\r\n \"before\":1e+02, \"projects\" : { \"/other\": {\"hasTrustDialogAccepted\":false}, \"/project\" : {\"allowedTools\":[ \"Read\" ], \"hasTrustDialogAccepted\" : false, \"after\":\"\\u0041\"} }, \"after\": [ 1, 2 ]\r\n}\r\n";
        assert_eq!(
            with_project_trust(raw, "/project").unwrap().unwrap(),
            raw.replace(" : false", " : true")
        );
    }

    #[test]
    fn existing_project_with_missing_flag_keeps_other_bytes() {
        let raw = "{\"projects\":{\"/project\":{\"other\":1e+02  }}}\n";
        assert_eq!(
            with_project_trust(raw, "/project").unwrap().unwrap(),
            "{\"projects\":{\"/project\":{\"other\":1e+02,  \"hasTrustDialogAccepted\":true}}}\n"
        );
    }

    #[test]
    fn inserting_a_project_preserves_all_existing_bytes() {
        let raw = " {\"projects\" : {\"/other\":{\"escaped\":\"\\u0041\"}  }, \"after\":1e+02}\r\n";
        let updated = with_project_trust(raw, "/a/\"project").unwrap().unwrap();
        assert_eq!(updated, format!(" {{\"projects\" : {{\"/other\":{{\"escaped\":\"\\u0041\"}},  \"/a/\\\"project\":{TRUSTED_PROJECT}}}, \"after\":1e+02}}\r\n"));
        let _: serde_json::Value = serde_json::from_str(&updated).unwrap();
    }

    #[test]
    fn explicit_account_writes_only_its_own_config() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let account = temp.path().join("account");
        let cwd = temp.path().join("project");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        let native = home.join(".claude.json");
        let original = "{\"keep\":1e+02}\r\n";
        fs::write(&native, original).unwrap();
        crate::runtimes::with_conversation_home(&home, || {
            seed_project_trust(&cwd, account.to_str()).unwrap();
        });
        assert_eq!(
            project(
                &account.join(".claude.json"),
                &fs::canonicalize(cwd).unwrap()
            )[ACCEPTED],
            true
        );
        assert_eq!(fs::read_to_string(native).unwrap(), original);
    }

    #[test]
    fn invalid_config_is_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let config = temp.path().join(".claude.json");
        let key = serde_json::to_string(&project_key(&fs::canonicalize(&cwd).unwrap())).unwrap();
        for raw in [
            "{broken".into(),
            "[]".into(),
            "{\"projects\":false}".into(),
            format!("{{\"projects\":{{{key}:false}}}}"),
        ] {
            fs::write(&config, &raw).unwrap();
            assert!(seed_project_trust_at(&cwd, &config, None).is_err());
            assert_eq!(fs::read_to_string(&config).unwrap(), raw);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cwd_seeds_its_realpath_and_keeps_config_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let cwd = temp.path().join("linked");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &cwd).unwrap();
        let config_target = temp.path().join("dotfiles.json");
        fs::write(&config_target, "{\"keep\":1e+02}\n").unwrap();
        let config = temp.path().join(".claude.json");
        std::os::unix::fs::symlink(&config_target, &config).unwrap();
        seed_project_trust_at(&cwd, &config, None).unwrap();
        assert!(fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            project(&config_target, &fs::canonicalize(target).unwrap())[ACCEPTED],
            true
        );
        assert!(project(&config_target, &cwd).is_null());
    }

    #[test]
    fn concurrent_seeds_keep_every_entry() {
        let temp = tempfile::tempdir().unwrap();
        let config = Arc::new(temp.path().join(".claude.json"));
        let projects: Vec<PathBuf> = (0..8)
            .map(|i| temp.path().join(format!("project-{i}")))
            .collect();
        for project in &projects {
            fs::create_dir(project).unwrap();
        }
        let barrier = Arc::new(Barrier::new(projects.len()));
        let threads: Vec<_> = projects
            .iter()
            .cloned()
            .map(|cwd| {
                let config = Arc::clone(&config);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    seed_project_trust_at(&cwd, &config, None).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        for cwd in projects {
            assert_eq!(
                project(&config, &fs::canonicalize(cwd).unwrap())[ACCEPTED],
                true
            );
        }
    }
}
