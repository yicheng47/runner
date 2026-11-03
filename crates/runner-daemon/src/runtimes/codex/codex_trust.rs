use crate::runtimes::helpers::{
    is_broad_trust_root, resolve_config_write_path, write_config_atomically,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[cfg(not(test))]
pub(crate) fn seed_project_trust(cwd: &Path) -> Result<()> {
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory is not available"))?;
    let config_path = super::config_path(&home);
    seed_project_trust_at_with_home(cwd, &config_path, Some(&home))
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

/// Codex's thread-title generator starts a hidden thread that sets every
/// MCP server it knows to `{ enabled = false }` and reloads config without
/// the session's `-c` overrides, so a server that exists only in Runner's
/// `-c mcp_servers.runner_hooks` loses its command there and the title is
/// never generated. A disabled entry in config.toml gives it one; sessions
/// re-enable the server through their own override.
#[cfg(not(test))]
pub(crate) fn seed_hook_server() -> Result<()> {
    let home = runner_core::app_paths::home_dir()
        .ok_or_else(|| Error::msg("home directory is not available"))?;
    seed_hook_server_at(&super::config_path(&home))
}

#[cfg(test)]
pub(crate) fn seed_hook_server() -> Result<()> {
    Ok(())
}

fn seed_hook_server_at(config_path: &Path) -> Result<()> {
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

    if doc.get("mcp_servers").is_none() {
        let mut servers = toml_edit::Table::new();
        servers.set_implicit(true);
        doc["mcp_servers"] = toml_edit::Item::Table(servers);
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| Error::msg("mcp_servers is not a table"))?;
    if servers.contains_key("runner_hooks") {
        return Ok(());
    }
    let mut server = toml_edit::Table::new();
    let gap = if raw.trim().is_empty() { "" } else { "\n" };
    server.decor_mut().set_prefix(format!(
        "{gap}# Runner's status hooks: Runner enables this per session.\n"
    ));
    server["command"] = toml_edit::value("runner");
    server["args"] = toml_edit::value(toml_edit::Array::from_iter(["hook", "serve"]));
    server["enabled"] = toml_edit::value(false);
    servers.insert("runner_hooks", toml_edit::Item::Table(server));

    write_config_atomically(&write_path, doc.to_string().as_bytes())?;
    Ok(())
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

/// Whether `config_path` holds a trust decision for a Codex started in `cwd`, so Codex
/// shows no folder-trust screen (`should_show_trust_screen` in its TUI). The project is
/// found as Codex's `get_active_project` finds it: an entry for `cwd` itself wins, even
/// one with no decision, before the entry for its repository root.
pub(crate) fn trust_decided(cwd: &Path, config_path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(projects) = doc.get("projects").and_then(|item| item.as_table_like()) else {
        return false;
    };
    let entry = |path: &Path| {
        lookup_keys(path).into_iter().find_map(|key| {
            projects.get(&key).or_else(|| {
                let mut matches = projects
                    .iter()
                    .filter(|(name, _)| normalize_lookup_key(name) == key)
                    .collect::<Vec<_>>();
                matches.sort_by_key(|(name, _)| *name);
                matches.first().map(|(_, item)| *item)
            })
        })
    };
    entry(cwd)
        .or_else(|| {
            resolve_project_trust_root(cwd)
                .ok()
                .and_then(|root| entry(&root))
        })
        .and_then(|project| project.get("trust_level"))
        .and_then(|level| level.as_str())
        .is_some_and(|level| matches!(level, "trusted" | "untrusted"))
}

/// Codex's keys for a path: its canonical form, then the path as given.
fn lookup_keys(path: &Path) -> Vec<String> {
    let given = normalize_lookup_key(&path.to_string_lossy());
    let canonical = fs::canonicalize(path)
        .map(|path| normalize_lookup_key(&path.to_string_lossy()))
        .unwrap_or_else(|_| given.clone());
    if given == canonical {
        vec![canonical]
    } else {
        vec![canonical, given]
    }
}

fn normalize_lookup_key(key: &str) -> String {
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    }
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
    fn a_trust_decision_counts_where_codex_looks_for_it() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let cwd = repo.join("crates/app");
        let config_path = temp.path().join("codex/config.toml");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(temp.path().join("other")).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        #[cfg(windows)]
        assert!(
            !cwd.to_string_lossy().starts_with(r"\\?\"),
            "a non-verbatim cwd against the canonical keys the seed writes"
        );
        let key = |path: &Path| fs::canonicalize(path).unwrap().display().to_string();
        let decided = |config: String| {
            fs::write(&config_path, config).unwrap();
            trust_decided(&cwd, &config_path)
        };
        assert!(!trust_decided(&cwd, &config_path), "no config");
        seed_project_trust_at(&cwd, &config_path).unwrap();
        assert!(trust_decided(&cwd, &config_path), "seeded at the repo root");
        assert!(
            decided(format!(
                "[projects.{:?}]\ntrust_level = \"untrusted\"\n",
                key(&repo)
            )),
            "an untrusted decision shows no trust screen either"
        );
        assert!(
            decided(format!(
                "[projects.{:?}]\ntrust_level = \"trusted\"\n",
                key(&cwd)
            )),
            "the folder's own entry"
        );
        assert!(
            !decided(format!(
                "[projects.{:?}]\ntrust_level = \"trusted\"\n[projects.{:?}]\nmodel = \"x\"\n",
                key(&repo),
                key(&cwd)
            )),
            "the folder's undecided entry wins over a trusted root"
        );
        assert!(
            !decided(format!(
                "[projects.{:?}]\ntrust_level = \"trusted\"\n",
                key(&temp.path().join("other"))
            )),
            "another folder"
        );
    }

    #[test]
    fn the_folder_s_canonical_entry_wins_over_its_entry_as_given() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        let cwd = {
            let link = temp.path().join("link");
            symlink(&real, &link).unwrap();
            link
        };
        #[cfg(windows)]
        let cwd = real.clone();
        let canonical = fs::canonicalize(&cwd).unwrap().display().to_string();
        let given = cwd.display().to_string();
        assert_ne!(canonical, given);
        let config_path = temp.path().join("config.toml");
        let entry = |key: &str, level: Option<&str>| match level {
            Some(level) => format!("[projects.{key:?}]\ntrust_level = {level:?}\n"),
            None => format!("[projects.{key:?}]\nmodel = \"x\"\n"),
        };
        let decided = |config: String| {
            fs::write(&config_path, config).unwrap();
            trust_decided(&cwd, &config_path)
        };
        assert!(
            decided(entry(&canonical, Some("trusted")) + &entry(&given, None)),
            "the canonical entry decides"
        );
        assert!(
            !decided(entry(&canonical, None) + &entry(&given, Some("trusted"))),
            "an undecided canonical entry wins over the given path's"
        );
        assert!(
            decided(entry(&given, Some("trusted"))),
            "the path as given when there is no canonical entry"
        );
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
    fn hook_server_stub_is_seeded_disabled_once() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("codex/config.toml");

        seed_hook_server_at(&config_path).unwrap();
        seed_hook_server_at(&config_path).unwrap();

        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            "# Runner's status hooks: Runner enables this per session.\n[mcp_servers.runner_hooks]\ncommand = \"runner\"\nargs = [\"hook\", \"serve\"]\nenabled = false\n"
        );
    }

    #[test]
    fn hook_server_stub_keeps_other_servers_and_existing_entries() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let existing = "# keep this comment\nmodel = \"gpt-5\"\n\n[mcp_servers.pencil]\ncommand = \"/pencil\"\n";
        fs::write(&config_path, existing).unwrap();

        seed_hook_server_at(&config_path).unwrap();

        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            format!("{existing}\n# Runner's status hooks: Runner enables this per session.\n[mcp_servers.runner_hooks]\ncommand = \"runner\"\nargs = [\"hook\", \"serve\"]\nenabled = false\n")
        );

        let own = "[mcp_servers.runner_hooks]\ncommand = \"/custom/runner\" # operator choice\n";
        fs::write(&config_path, own).unwrap();
        seed_hook_server_at(&config_path).unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), own);
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
