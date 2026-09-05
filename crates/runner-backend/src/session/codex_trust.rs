use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[cfg(not(test))]
pub(crate) fn seed_project_trust(cwd: &Path) -> Result<()> {
    let config_path = crate::ops::mcp::codex_path()?;
    let home = config_path.parent().and_then(Path::parent).ok_or_else(|| {
        Error::msg(format!(
            "invalid codex config path: {}",
            config_path.display()
        ))
    })?;
    seed_project_trust_at_with_home(cwd, &config_path, Some(home))
}

#[cfg(test)]
pub(crate) fn seed_project_trust(_cwd: &Path) -> Result<()> {
    // SessionManager tests use mocked runtimes and must not mutate the developer's Codex config.
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
    let project_root = resolve_project_trust_root(cwd)?;
    if is_broad_trust_root(&project_root, home) {
        log::debug!(
            "skipping broad codex project trust root: cwd={} root={}",
            cwd.display(),
            project_root.display()
        );
        return Ok(());
    }

    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| Error::msg("codex trust config lock poisoned"))?;
    let write_path = resolve_config_write_path(config_path)?;
    let raw = if write_path.exists() {
        fs::read_to_string(&write_path)
            .map_err(|e| Error::msg(format!("read {}: {e}", write_path.display())))?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| Error::msg(format!("parse {}: {e}", write_path.display())))?;

    if doc.get("projects").is_none() {
        let mut projects = toml_edit::Table::new();
        projects.set_implicit(true);
        doc["projects"] = toml_edit::Item::Table(projects);
    }
    let projects = doc["projects"]
        .as_table_mut()
        .ok_or_else(|| Error::msg("projects is not a table"))?;
    let project_key = project_root.to_string_lossy();

    if let Some(project) = projects.get_mut(project_key.as_ref()) {
        let table = project
            .as_table_mut()
            .ok_or_else(|| Error::msg(format!("projects.{project_key} is not a table")))?;
        if table.contains_key("trust_level") {
            return Ok(());
        }
        table["trust_level"] = toml_edit::value("trusted");
    } else {
        let mut project = toml_edit::Table::new();
        project["trust_level"] = toml_edit::value("trusted");
        projects.insert(project_key.as_ref(), toml_edit::Item::Table(project));
    }

    write_config_atomically(&write_path, doc.to_string().as_bytes())?;
    Ok(())
}

fn resolve_project_trust_root(cwd: &Path) -> Result<PathBuf> {
    let cwd = fs::canonicalize(cwd)
        .map_err(|e| Error::msg(format!("realpath {}: {e}", cwd.display())))?;
    for ancestor in cwd.ancestors() {
        let git_marker = ancestor.join(".git");
        if git_marker.is_dir() {
            return Ok(ancestor.to_path_buf());
        }
        if git_marker.is_file() {
            return Ok(
                resolve_worktree_main_root(ancestor).unwrap_or_else(|| ancestor.to_path_buf())
            );
        }
    }
    Ok(cwd)
}

fn is_broad_trust_root(project_root: &Path, home: Option<&Path>) -> bool {
    if project_root.parent().is_none() {
        return true;
    }
    home.map(|home| fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()))
        .as_deref()
        == Some(project_root)
}

fn resolve_config_write_path(config_path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(config_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(config_path)
            .map_err(|e| Error::msg(format!("realpath {}: {e}", config_path.display()))),
        Ok(_) => Ok(config_path.to_path_buf()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(config_path.to_path_buf()),
        Err(e) => Err(Error::msg(format!(
            "metadata {}: {e}",
            config_path.display()
        ))),
    }
}

fn write_config_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::msg(format!("config path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|e| Error::msg(format!("mkdir {}: {e}", parent.display())))?;
    let permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| Error::msg(format!("create temp file in {}: {e}", parent.display())))?;
    if let Some(permissions) = permissions {
        temp.as_file()
            .set_permissions(permissions)
            .map_err(|e| Error::msg(format!("set temp permissions for {}: {e}", path.display())))?;
    }
    temp.write_all(contents)
        .map_err(|e| Error::msg(format!("write temp config for {}: {e}", path.display())))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| Error::msg(format!("sync temp config for {}: {e}", path.display())))?;
    temp.persist(path)
        .map_err(|e| Error::msg(format!("persist {}: {}", path.display(), e.error)))?;
    Ok(())
}

fn resolve_worktree_main_root(cwd: &Path) -> Option<PathBuf> {
    let git_file = cwd.join(".git");
    let git_dir_reference = fs::read_to_string(&git_file).ok()?;
    let git_dir_path = git_dir_reference.trim().strip_prefix("gitdir:")?.trim();
    if git_dir_path.is_empty() {
        return None;
    }
    let git_dir_path = Path::new(git_dir_path);
    let git_dir = if git_dir_path.is_absolute() {
        git_dir_path.to_path_buf()
    } else {
        cwd.join(git_dir_path)
    };
    let worktrees_dir = git_dir.parent()?;
    if worktrees_dir.file_name()? != "worktrees" {
        return None;
    }

    let backlink_reference = fs::read_to_string(git_dir.join("gitdir")).ok()?;
    let backlink_path = backlink_reference.trim();
    if backlink_path.is_empty() {
        return None;
    }
    let backlink_path = Path::new(backlink_path);
    let backlink = if backlink_path.is_absolute() {
        backlink_path.to_path_buf()
    } else {
        git_dir.join(backlink_path)
    };
    let canonical_backlink = fs::canonicalize(backlink).ok()?;
    let canonical_git_file = fs::canonicalize(&git_file).ok()?;
    if canonical_backlink != canonical_git_file {
        return None;
    }

    let main_root = worktrees_dir.parent()?.parent()?;
    fs::canonicalize(main_root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn trust_level(config_path: &Path, project_path: &Path) -> Option<String> {
        let raw = fs::read_to_string(config_path).unwrap();
        let doc: toml_edit::DocumentMut = raw.parse().unwrap();
        doc.get("projects")?
            .as_table()?
            .get(project_path.to_string_lossy().as_ref())?
            .as_table()?
            .get("trust_level")?
            .as_str()
            .map(str::to_owned)
    }

    #[test]
    fn empty_config_creates_explicit_project_block() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let config_path = temp.path().join("codex/config.toml");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "").unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                cwd.display()
            )
        );
        #[cfg(windows)]
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            format!(
                "[projects.'{}']\ntrust_level = \"trusted\"\n",
                cwd.display()
            )
        );
    }

    #[test]
    fn missing_config_creates_parent_and_project_block() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let config_path = temp.path().join("codex/config.toml");
        fs::create_dir_all(&cwd).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        assert_eq!(trust_level(&config_path, &cwd).as_deref(), Some("trusted"));
    }

    #[test]
    fn subdirectory_of_repo_seeds_repo_root() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let cwd = repo.join("packages/app");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&cwd).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let repo = fs::canonicalize(repo).unwrap();
        let cwd = fs::canonicalize(cwd).unwrap();
        assert_eq!(trust_level(&config_path, &repo).as_deref(), Some("trusted"));
        assert_eq!(trust_level(&config_path, &cwd), None);
    }

    #[test]
    fn subdirectory_of_linked_worktree_seeds_main_repo_root() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let worktree = temp.path().join("feature");
        let cwd = worktree.join("packages/app");
        let git_dir = main.join(".git/worktrees/feature");
        let config_path = temp.path().join("config.toml");
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

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let main = fs::canonicalize(main).unwrap();
        let worktree = fs::canonicalize(worktree).unwrap();
        let cwd = fs::canonicalize(cwd).unwrap();
        assert_eq!(trust_level(&config_path, &main).as_deref(), Some("trusted"));
        assert_eq!(trust_level(&config_path, &worktree), None);
        assert_eq!(trust_level(&config_path, &cwd), None);
    }

    #[test]
    fn no_git_ancestor_seeds_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("plain/nested");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&cwd).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        assert_eq!(trust_level(&config_path, &cwd).as_deref(), Some("trusted"));
    }

    #[test]
    fn home_git_marker_does_not_widen_trust_to_home() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cwd = home.join("Downloads/project");
        let config_path = home.join(".codex/config.toml");
        fs::create_dir_all(home.join(".git")).unwrap();
        fs::create_dir_all(&cwd).unwrap();

        seed_project_trust_at_with_home(&cwd, &config_path, Some(&home)).unwrap();

        assert!(!config_path.exists());
    }

    #[test]
    fn filesystem_root_is_rejected_as_too_broad() {
        assert!(is_broad_trust_root(Path::new("/"), None));
    }

    #[test]
    fn unrelated_config_is_preserved_verbatim() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&cwd).unwrap();
        let existing = "# keep this comment\nmodel = \"gpt-5\"\n\n[mcp_servers.runner]\ncommand = \"/runner\"\n";
        fs::write(&config_path, existing).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            format!(
                "{existing}\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                cwd.display()
            )
        );
        #[cfg(windows)]
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            format!(
                "{existing}\n[projects.'{}']\ntrust_level = \"trusted\"\n",
                cwd.display()
            )
        );
    }

    #[test]
    fn existing_trusted_entry_is_untouched() {
        assert_existing_entry_is_untouched("trusted");
    }

    #[test]
    fn existing_untrusted_entry_is_untouched() {
        assert_existing_entry_is_untouched("untrusted");
    }

    fn assert_existing_entry_is_untouched(level: &str) {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&cwd).unwrap();
        let cwd = fs::canonicalize(cwd).unwrap();
        #[cfg(unix)]
        let existing = format!(
            "# unchanged\n[projects.\"{}\"]\ntrust_level = \"{level}\" # operator choice\n",
            cwd.display()
        );
        #[cfg(windows)]
        let existing = format!(
            "# unchanged\n[projects.'{}']\ntrust_level = \"{level}\" # operator choice\n",
            cwd.display()
        );
        fs::write(&config_path, &existing).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        assert_eq!(fs::read_to_string(config_path).unwrap(), existing);
    }

    #[test]
    fn linked_worktree_seeds_main_repo_root() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let worktree = temp.path().join("feature");
        let git_dir = main.join(".git/worktrees/feature");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&git_dir).unwrap();
        fs::create_dir_all(&worktree).unwrap();
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

        seed_project_trust_at(&worktree, &config_path).unwrap();

        let main = fs::canonicalize(main).unwrap();
        let worktree = fs::canonicalize(worktree).unwrap();
        assert_eq!(trust_level(&config_path, &main).as_deref(), Some("trusted"));
        assert_eq!(trust_level(&config_path, &worktree), None);
    }

    #[test]
    fn forged_gitdir_without_backlink_does_not_widen_trust() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let forged_main = temp.path().join("forged-main");
        let forged_git_dir = forged_main.join(".git/worktrees/forged");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&forged_git_dir).unwrap();
        fs::write(
            cwd.join(".git"),
            format!("gitdir: {}\n", forged_git_dir.display()),
        )
        .unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        let forged_main = fs::canonicalize(forged_main).unwrap();
        assert_eq!(trust_level(&config_path, &cwd).as_deref(), Some("trusted"));
        assert_eq!(trust_level(&config_path, &forged_main), None);
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_cwd_seeds_realpath() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let linked = temp.path().join("linked");
        let config_path = temp.path().join("config.toml");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &linked).unwrap();

        seed_project_trust_at(&linked, &config_path).unwrap();

        let target = fs::canonicalize(target).unwrap();
        assert_eq!(
            trust_level(&config_path, &target).as_deref(),
            Some("trusted")
        );
        assert_eq!(trust_level(&config_path, &linked), None);
    }

    #[test]
    #[cfg(unix)]
    fn atomic_write_preserves_config_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let target = temp.path().join("dotfiles/codex-config.toml");
        let config_path = temp.path().join("home/.codex/config.toml");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&target, "model = \"gpt-5\"\n").unwrap();
        symlink(&target, &config_path).unwrap();

        seed_project_trust_at(&cwd, &config_path).unwrap();

        let cwd = fs::canonicalize(cwd).unwrap();
        assert!(fs::symlink_metadata(&config_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(trust_level(&target, &cwd).as_deref(), Some("trusted"));
    }

    #[test]
    fn concurrent_seeds_preserve_every_project_entry() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = Arc::new(temp.path().join("config.toml"));
        let projects: Vec<PathBuf> = (0..8)
            .map(|index| temp.path().join(format!("project-{index}")))
            .collect();
        for project in &projects {
            fs::create_dir_all(project).unwrap();
        }
        let barrier = Arc::new(Barrier::new(projects.len()));
        let threads: Vec<_> = projects
            .iter()
            .cloned()
            .map(|project| {
                let barrier = Arc::clone(&barrier);
                let config_path = Arc::clone(&config_path);
                thread::spawn(move || {
                    barrier.wait();
                    seed_project_trust_at(&project, &config_path).unwrap();
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }
        for project in projects {
            let project = fs::canonicalize(project).unwrap();
            assert_eq!(
                trust_level(&config_path, &project).as_deref(),
                Some("trusted")
            );
        }
    }
}
