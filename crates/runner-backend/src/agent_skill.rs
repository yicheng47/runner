use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

const SKILL_FILE: &str = "SKILL.md";
pub const SKILL_ROOTS: &[&str] = runner_core::RUNNER_SKILL_ROOTS;
pub const SKILL_MARKER: &str = runner_core::RUNNER_SKILL_MARKER;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillRootState {
    Missing,
    Current,
    Stale,
    Foreign,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkillRootStatus {
    pub root: PathBuf,
    pub folder: PathBuf,
    pub state: SkillRootState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    Refreshed,
    Current,
    Foreign,
}

pub fn skill_name(debug: bool) -> &'static str {
    runner_core::runner_skill_name(debug)
}

pub fn sidecar_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join("bin")
        .join(format!("runner{}", std::env::consts::EXE_SUFFIX))
}

pub fn render(debug: bool, sidecar: &Path) -> String {
    let name = skill_name(debug);
    let description = if debug {
        "Development build of Runner (the `make run` app, separate data from the installed app): operate its local cockpit for CLI coding agents through the `runner-dev` skill. Use when the user says dev, development build, `make run` or `runner-dev`."
    } else {
        "Operate Runner, the local cockpit for CLI coding agents, through the `runner` CLI: start, follow and steer missions, list crews and roles, start chats. Use when the user mentions Runner, a mission, a crew or a role, or asks to hand work to another agent such as Codex, Claude Code or pi."
    };
    let invocation = if debug {
        format!(
            "This is Runner's development build. It talks only to the development app.\n\nAlways invoke \"{}\"; a bare `runner` on PATH may be the installed app.\n\nRun \"{}\" help agents for the version-matched command guide.",
            sidecar.display(),
            sidecar.display()
        )
    } else {
        format!(
            "Use `runner` when it is on PATH. Otherwise invoke \"{}\".\n\nRun `runner help agents` for the version-matched command guide.",
            sidecar.display()
        )
    };
    format!(
        r#"---
name: {name}
description: "{description}"
---

# Runner

Runner is the local cockpit for coordinating CLI coding agents. Crews are reusable teams; missions are their live runs.

{invocation}

Rules:

1. Prefer `--json` for structured output.
2. Exit code 3 means Runner is not running; ask the user to open Runner. Exit code 5 means a sandbox kept the command from reaching Runner, so nothing is known about Runner yet: run the same command again outside the sandbox (in Codex, request escalated permissions for it) before telling the user anything.
3. Use `--help` rather than guessing commands.
4. Inside a mission, mission commands carry your own handle. Outside a mission you act for the user: your posts and answers appear as the person. Never pass `--as` to speak as a slot you were not given.
"#
    )
}

pub fn statuses(home: &Path, app_data_dir: &Path, debug: bool) -> Vec<SkillRootStatus> {
    let expected = render(debug, &sidecar_path(app_data_dir));
    runner_core::RUNNER_SKILL_ROOTS
        .iter()
        .map(|relative| status_root(&home.join(relative), debug, &expected))
        .collect()
}

pub fn status_root(root: &Path, debug: bool, expected: &str) -> SkillRootStatus {
    let folder = root.join(skill_name(debug));
    let state = if !folder.exists() {
        SkillRootState::Missing
    } else if !folder.join(runner_core::RUNNER_SKILL_MARKER).is_file() {
        SkillRootState::Foreign
    } else if fs::read_to_string(folder.join(SKILL_FILE)).is_ok_and(|content| content == expected) {
        SkillRootState::Current
    } else {
        SkillRootState::Stale
    };
    SkillRootStatus {
        root: root.to_path_buf(),
        folder,
        state,
    }
}

pub fn install_root(root: &Path, debug: bool, content: &str) -> Result<InstallOutcome> {
    fs::create_dir_all(root)?;
    let folder = root.join(skill_name(debug));
    if folder.exists() {
        if !folder.join(runner_core::RUNNER_SKILL_MARKER).is_file() {
            return Ok(InstallOutcome::Foreign);
        }
        if fs::read_to_string(folder.join(SKILL_FILE)).is_ok_and(|current| current == content) {
            return Ok(InstallOutcome::Current);
        }
        atomic_write(&folder.join(SKILL_FILE), content.as_bytes())?;
        return Ok(InstallOutcome::Refreshed);
    }

    let staging = tempfile::Builder::new()
        .prefix(".runner-skill-")
        .tempdir_in(root)?;
    fs::write(staging.path().join(SKILL_FILE), content)?;
    fs::write(
        staging.path().join(runner_core::RUNNER_SKILL_MARKER),
        b"Runner manages this skill folder.\n",
    )?;
    fs::rename(staging.path(), &folder)?;
    Ok(InstallOutcome::Installed)
}

pub fn install(home: &Path, app_data_dir: &Path, debug: bool) -> Result<Vec<InstallOutcome>> {
    let content = render(debug, &sidecar_path(app_data_dir));
    runner_core::RUNNER_SKILL_ROOTS
        .iter()
        .map(|relative| install_root(&home.join(relative), debug, &content))
        .collect()
}

pub fn refresh(home: &Path, app_data_dir: &Path, debug: bool) -> Result<Vec<SkillRootStatus>> {
    let content = render(debug, &sidecar_path(app_data_dir));
    for relative in runner_core::RUNNER_SKILL_ROOTS {
        let root = home.join(relative);
        let status = status_root(&root, debug, &content);
        if status.state == SkillRootState::Stale {
            install_root(&root, debug, &content)?;
        }
    }
    Ok(statuses(home, app_data_dir, debug))
}

pub fn remove_root(root: &Path, debug: bool) -> Result<bool> {
    let folder = root.join(skill_name(debug));
    if !folder.exists() || !folder.join(runner_core::RUNNER_SKILL_MARKER).is_file() {
        return Ok(false);
    }
    let removed = root.join(format!(
        ".{}-remove-{}",
        skill_name(debug),
        ulid::Ulid::new()
    ));
    fs::rename(&folder, &removed)?;
    fs::remove_dir_all(removed)?;
    Ok(true)
}

pub fn remove(home: &Path, debug: bool) -> Result<Vec<bool>> {
    runner_core::RUNNER_SKILL_ROOTS
        .iter()
        .map(|relative| remove_root(&home.join(relative), debug))
        .collect()
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::msg(format!("{} has no parent directory", path.display())))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut temp, contents)?;
    temp.persist(path).map_err(|error| Error::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_all_roots_with_marker_and_rendered_quoted_sidecar() {
        let home = tempfile::tempdir().unwrap();
        let app_data = home.path().join("Library/Application Support/runner");
        let outcomes = install(home.path(), &app_data, false).unwrap();
        assert_eq!(outcomes, vec![InstallOutcome::Installed; 3]);
        for relative in runner_core::RUNNER_SKILL_ROOTS {
            let folder = home.path().join(relative).join("runner");
            assert!(folder.join(runner_core::RUNNER_SKILL_MARKER).is_file());
            let text = fs::read_to_string(folder.join(SKILL_FILE)).unwrap();
            assert!(text.contains("name: runner\n"));
            assert!(text.contains(&format!("\"{}\"", sidecar_path(&app_data).display())));
        }
    }

    #[test]
    fn debug_render_has_separate_name_and_development_sidecar() {
        let release = render(false, Path::new("C:\\Runner\\bin\\runner.exe"));
        let debug = render(true, Path::new("C:\\Runner Dev\\bin\\runner.exe"));
        let release_description = frontmatter_description(&release);
        let debug_description = frontmatter_description(&debug);

        assert!(debug.contains("name: runner-dev\n"));
        assert!(debug.contains("development build"));
        assert!(debug.contains(r#""C:\Runner Dev\bin\runner.exe""#));
        assert!(!debug.contains("Use `runner` when it is on PATH"));
        assert!(debug.contains(
            r#"Run "C:\Runner Dev\bin\runner.exe" help agents for the version-matched command guide."#
        ));
        assert!(release.contains("Use `runner` when it is on PATH"));
        assert!(release.contains("Run `runner help agents`"));
        assert!(debug_description.starts_with(
            "Development build of Runner (the `make run` app, separate data from the installed app)"
        ));
        assert!(debug_description.contains("dev, development build, `make run` or `runner-dev`"));
        assert!(!release_description.contains("Development build"));
        assert!(!release_description.contains("runner-dev"));
        assert_ne!(release_description, debug_description);
        for rendered in [&release, &debug] {
            assert!(rendered.contains(
                "Inside a mission, mission commands carry your own handle. Outside a mission you act for the user: your posts and answers appear as the person. Never pass `--as` to speak as a slot you were not given."
            ));
            assert!(!rendered.contains("takes a seat"));
        }
    }

    #[test]
    fn refresh_repairs_a_marked_debug_skill_with_the_current_sidecar() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(".agents/skills/runner-dev");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(runner_core::RUNNER_SKILL_MARKER), "managed").unwrap();
        fs::write(
            folder.join(SKILL_FILE),
            render(true, Path::new("/tmp/dead-app/bin/runner")),
        )
        .unwrap();
        let app_data = home.path().join("current-dev-app");

        refresh(home.path(), &app_data, true).unwrap();

        assert_eq!(
            fs::read_to_string(folder.join(SKILL_FILE)).unwrap(),
            render(true, &sidecar_path(&app_data))
        );
    }

    #[test]
    fn refresh_updates_marked_folder_and_leaves_foreign_folder_alone() {
        let home = tempfile::tempdir().unwrap();
        let app_data = home.path().join("app");
        let marked = home.path().join(".claude/skills/runner");
        fs::create_dir_all(&marked).unwrap();
        fs::write(marked.join(runner_core::RUNNER_SKILL_MARKER), "managed").unwrap();
        fs::write(marked.join(SKILL_FILE), "stale").unwrap();
        let foreign = home.path().join(".agents/skills/runner");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join(SKILL_FILE), "mine").unwrap();

        let statuses = refresh(home.path(), &app_data, false).unwrap();
        assert_eq!(statuses[0].state, SkillRootState::Current);
        assert_eq!(statuses[1].state, SkillRootState::Foreign);
        assert_eq!(
            fs::read_to_string(foreign.join(SKILL_FILE)).unwrap(),
            "mine"
        );
    }

    #[test]
    fn remove_deletes_only_marked_folders() {
        let home = tempfile::tempdir().unwrap();
        let app_data = home.path().join("app");
        install(home.path(), &app_data, false).unwrap();
        let foreign = home.path().join(".agents/skills/runner");
        fs::remove_file(foreign.join(runner_core::RUNNER_SKILL_MARKER)).unwrap();

        let removed = remove(home.path(), false).unwrap();
        assert_eq!(removed, vec![true, false, true]);
        assert!(foreign.is_dir());
        assert_eq!(
            status_root(
                &home.path().join(".agents/skills"),
                false,
                &render(false, &sidecar_path(&app_data))
            )
            .state,
            SkillRootState::Foreign
        );
    }

    fn frontmatter_description(text: &str) -> &str {
        text.lines()
            .find_map(|line| line.strip_prefix("description: "))
            .map(|description| description.trim_matches('"'))
            .unwrap()
    }
}
