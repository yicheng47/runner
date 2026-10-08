use crate::model::Runtime;
use std::collections::HashSet;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub use runner_core::protocol::skills::GlobalState;

pub use runner_core::protocol::skills::SkillEntry;

pub use runner_core::protocol::skills::SkillCatalog;

pub use runner_core::protocol::skills::SkillDocument;

pub use runner_core::protocol::skills::parse_skill_document;

fn skill_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next()?;
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.');
    (!clean.is_empty()).then(|| clean.to_owned())
}

pub fn skill_catalog(
    runtime: Runtime,
    home: &Path,
    codex_home: Option<&Path>,
) -> Option<SkillCatalog> {
    let support = crate::runtimes::adapter(runtime).skills();
    let relatives = support.roots;
    if relatives.is_empty() {
        return None;
    }
    let roots: Vec<_> = relatives
        .iter()
        .map(|relative| {
            if let (Some(codex_home), Ok(suffix)) =
                (codex_home, Path::new(relative).strip_prefix(".codex"))
            {
                codex_home.join(suffix)
            } else {
                home.join(relative)
            }
        })
        .collect();
    let state = (support.load)(home, codex_home);
    let mut catalog = SkillCatalog {
        runtime,
        root_exists: roots.iter().any(|root| root.is_dir()),
        roots,
        entries: Vec::new(),
    };
    for root in &catalog.roots {
        let mut visited = HashSet::new();
        if let Ok(path) = root.canonicalize() {
            visited.insert(path);
        }
        if let Ok(children) = std::fs::read_dir(root) {
            let mut paths: Vec<_> = children.flatten().map(|entry| entry.path()).collect();
            paths.sort();
            for path in paths {
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
                    || !path.is_dir()
                {
                    continue;
                }
                if let Ok(canonical) = path.canonicalize() {
                    if !visited.insert(canonical) {
                        continue;
                    }
                }
                let filenames: Vec<_> = std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|entry| entry.path().is_file())
                    .map(|entry| entry.file_name())
                    .collect();
                let marker = if filenames.iter().any(|name| name == "SKILL.md") {
                    "SKILL.md"
                } else if filenames.iter().any(|name| name == "skill.md") {
                    "skill.md"
                } else if path.join("skills").is_dir() {
                    continue;
                } else {
                    "SKILL.md"
                };
                let mut entry = read_entry(&path, marker, runtime);
                entry.global = state(&entry);
                catalog.entries.push(entry);
            }
        }
    }
    catalog.entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    Some(catalog)
}

fn read_entry(path: &Path, marker: &str, runtime: Runtime) -> SkillEntry {
    let mut files: Vec<_> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort_by_key(|name| name.to_lowercase());
    let document = if !files.iter().any(|file| file == marker) {
        SkillDocument {
            problem: Some("no SKILL.md".into()),
            ..SkillDocument::default()
        }
    } else {
        match std::fs::read_to_string(path.join(marker)) {
            Ok(text) => parse_skill_document(&text),
            Err(error) => SkillDocument {
                problem: Some(if error.kind() == std::io::ErrorKind::NotFound {
                    "no SKILL.md".into()
                } else {
                    format!("read {marker}: {error}")
                }),
                ..SkillDocument::default()
            },
        }
    };
    let value = |key| {
        document
            .frontmatter
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    SkillEntry {
        name: value("name").and_then(skill_name).unwrap_or_else(|| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        }),
        description: value("description").unwrap_or_default().into(),
        path: path.into(),
        marker: marker.into(),
        symlink: std::fs::read_link(path).ok().map(|target| {
            path.canonicalize()
                .unwrap_or_else(|_| path.parent().unwrap_or(path).join(target))
        }),
        manual: (crate::runtimes::adapter(runtime).skills().manual)(path, &document),
        hidden: (crate::runtimes::adapter(runtime).skills().hidden)(&document),
        problem: match (document.problem, marker == "skill.md") {
            (Some(problem), true) => Some(format!("legacy skill.md; {problem}")),
            (None, true) => Some("legacy skill.md".into()),
            (problem, false) => problem,
        },
        files,
        global: GlobalState::On,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtimes::codex::skills::codex_manual;

    fn write(home: &Path, name: &str, marker: &str, text: &str) -> PathBuf {
        let path = home.join(".claude/skills").join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(marker), text).unwrap();
        path
    }

    #[test]
    fn parses_names_descriptions_flags_and_lists_files() {
        let home = tempfile::tempdir().unwrap();
        let path = write(home.path(), "folder", "SKILL.md", "---\nname: ../../my-skill\ndescription: 'First sentence. More.'\ndisable-model-invocation: true\nuser-invocable: false\nignored: [anything]\n---\n# Body\n");
        std::fs::write(path.join("script.py"), "pass").unwrap();
        let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        let entry = &catalog.entries[0];
        assert_eq!(entry.name, "my-skill");
        assert_eq!(entry.description, "First sentence. More.");
        assert!(entry.manual && entry.hidden);
        assert_eq!(entry.files, ["script.py", "SKILL.md"]);
        assert_eq!(entry.path, path);
        assert_eq!(entry.global, GlobalState::On);
        assert_eq!(entry.problem, None);
    }

    #[test]
    fn codex_manual_policy_scans_only_the_policy_child() {
        for text in [
            "policy:\n  allow_implicit_invocation: false\n",
            "interface:\n  display_name: Example\npolicy: # invocation\n\n  # Require an explicit request\n  allow_implicit_invocation: false # manual\n",
            "policy :\r\n    allow_implicit_invocation : false\r\ninterface:\r\n  display_name: Example\r\n",
        ] {
            assert!(codex_manual(text), "{text}");
        }
        for text in [
            "",
            "policy:\n  allow_implicit_invocation: true\n",
            "policy:\n  other: false\n",
            "allow_implicit_invocation: false\n",
            "interface:\n  allow_implicit_invocation: false\n",
            "policy:\n  other: true\ninterface:\n  allow_implicit_invocation: false\n",
            "description: |\n  policy:\n    allow_implicit_invocation: false\n",
            "policy:\n  other:\n    allow_implicit_invocation: false\n",
            "policy:\n  allow_implicit_invocation: 'false'\n",
            "policy:\n  allow_implicit_invocation: invalid\n",
            "# policy:\n  allow_implicit_invocation: false\n",
        ] {
            assert!(!codex_manual(text), "{text}");
        }
    }

    #[test]
    fn invocation_flags_belong_to_the_selected_runtime() {
        let home = tempfile::tempdir().unwrap();
        let frontmatter = "---\nname: demo\ndisable-model-invocation: true\nuser-invocable: false\n---\n# Skill\n";
        let claude = write(home.path(), "demo", "SKILL.md", frontmatter);
        let codex = home.path().join(".agents/skills/demo");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("SKILL.md"), frontmatter).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"skillOverrides":{"demo":"user-invocable-only"}}"#,
        )
        .unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        assert!(!catalog.entries[0].manual);
        assert!(!catalog.entries[0].hidden);
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        for policy in [
            "policy:\n  allow_implicit_invocation: false\n",
            "policy:\n  allow_implicit_invocation: true\n",
            "policy:\n  other: false\n",
        ] {
            for path in [&claude, &codex] {
                std::fs::create_dir_all(path.join("agents")).unwrap();
                std::fs::write(path.join("agents/openai.yaml"), policy).unwrap();
            }
            let catalog = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
            assert_eq!(
                catalog.entries[0].manual,
                policy.contains("allow_implicit_invocation: false")
            );
            assert!(!catalog.entries[0].hidden);
            assert_eq!(catalog.entries[0].global, GlobalState::On);
            assert!(catalog.entries[0].problem.is_none());
            let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
            assert!(catalog.entries[0].manual && catalog.entries[0].hidden);
            assert_eq!(
                catalog.entries[0].global,
                GlobalState::Other("user-invocable-only".into())
            );
        }
        std::fs::write(claude.join("SKILL.md"), "# No invocation flags\n").unwrap();
        std::fs::write(
            claude.join("agents/openai.yaml"),
            "policy:\n  allow_implicit_invocation: false\n",
        )
        .unwrap();
        let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        assert!(!catalog.entries[0].manual && !catalog.entries[0].hidden);
        std::fs::write(codex.join("agents/openai.yaml"), [0xff]).unwrap();
        assert!(
            !skill_catalog(Runtime::Codex, home.path(), None)
                .unwrap()
                .entries[0]
                .manual
        );
    }

    #[test]
    fn fallback_legacy_malformed_missing_and_walk_filters() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "Zoo", "SKILL.md", "# No frontmatter");
        write(home.path(), "alpha", "skill.md", "---\nname: ..\n---\n");
        write(home.path(), "bad", "SKILL.md", "---\nname: [bad\n---\n");
        write(home.path(), "missing", "README.md", "# Not a skill");
        write(home.path(), ".hidden", "SKILL.md", "");
        write(home.path(), "bundle/skills/embedded", "SKILL.md", "");
        std::fs::write(home.path().join(".claude/skills/plain.txt"), "").unwrap();
        let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "bad", "missing", "Zoo"]
        );
        assert_eq!(catalog.entries[0].marker, "skill.md");
        assert_eq!(
            catalog.entries[0].problem.as_deref(),
            Some("legacy skill.md")
        );
        assert!(catalog.entries[1].problem.is_some());
        assert_eq!(catalog.entries[2].problem.as_deref(), Some("no SKILL.md"));
        assert!(catalog.entries[3].problem.is_none());
    }

    #[test]
    fn other_marker_casing_is_not_accepted_on_case_insensitive_filesystems() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "mixed",
            "Skill.md",
            "---\nname: not-a-skill\n---\n",
        );
        let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        assert_eq!(catalog.entries[0].name, "mixed");
        assert_eq!(catalog.entries[0].problem.as_deref(), Some("no SKILL.md"));
    }

    #[test]
    fn global_states_and_bad_settings_are_tolerated() {
        let home = tempfile::tempdir().unwrap();
        for name in ["absent", "on", "off", "other"] {
            write(home.path(), name, "SKILL.md", "");
        }
        let path = home.path().join(".claude/settings.json");
        std::fs::write(
            &path,
            r#"{"skillOverrides":{"on":"on","off":"off","other":"name-only"}}"#,
        )
        .unwrap();
        let entries = skill_catalog(Runtime::ClaudeCode, home.path(), None)
            .unwrap()
            .entries;
        assert_eq!(
            entries.iter().map(|e| e.global.clone()).collect::<Vec<_>>(),
            [
                GlobalState::On,
                GlobalState::Off,
                GlobalState::On,
                GlobalState::Other("name-only".into())
            ]
        );
        std::fs::write(&path, "{broken").unwrap();
        assert!(skill_catalog(Runtime::ClaudeCode, home.path(), None)
            .unwrap()
            .entries
            .iter()
            .all(|e| e.global == GlobalState::On));
    }

    #[test]
    fn roots_and_unsupported_runtimes() {
        let home = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let empty = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        assert_eq!(
            empty.roots,
            [
                home.path().join(".agents/skills"),
                home.path().join(".codex/skills")
            ]
        );
        assert!(!empty.root_exists && empty.entries.is_empty());
        std::fs::create_dir_all(custom.path().join("skills/demo")).unwrap();
        std::fs::write(custom.path().join("skills/demo/SKILL.md"), "").unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), Some(custom.path())).unwrap();
        assert_eq!(
            catalog.roots,
            [
                home.path().join(".agents/skills"),
                custom.path().join("skills")
            ]
        );
        assert!(catalog.root_exists);
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        std::fs::write(
            custom.path().join("config.toml"),
            format!(
                "[[skills.config]]\npath = {}\nenabled = false\n",
                serde_json::json!(custom.path().join("skills/demo/SKILL.md"))
            ),
        )
        .unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), Some(custom.path())).unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::Off);
        std::fs::write(custom.path().join("config.toml"), "invalid = [").unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), Some(custom.path())).unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        let trae = skill_catalog(Runtime::Trae, home.path(), None).unwrap();
        assert_eq!(trae.roots, [home.path().join(".trae/skills")]);
        assert!(trae.entries.is_empty());
        assert!(skill_catalog(Runtime::Shell, home.path(), None).is_none());
    }

    #[test]
    fn trae_catalog_reads_only_its_documented_user_root() {
        let home = tempfile::tempdir().unwrap();
        for root in [".trae/skills", ".coco/skills", ".trae-cn/skills"] {
            let path = home.path().join(root).join("demo");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "---\ndescription: demo\n---\nbody").unwrap();
        }
        let catalog = skill_catalog(Runtime::Trae, home.path(), None).unwrap();
        assert_eq!(catalog.roots, [home.path().join(".trae/skills")]);
        assert_eq!(catalog.entries.len(), 1);
        assert!(catalog.entries[0]
            .path
            .starts_with(home.path().join(".trae/skills")));
    }

    #[test]
    fn pi_catalog_reads_both_personal_roots_without_global_toggles() {
        let home = tempfile::tempdir().unwrap();
        for (root, name) in [
            (".pi/agent/skills", "pi-skill"),
            (".agents/skills", "shared-skill"),
        ] {
            let path = home.path().join(root).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "---\ndescription: demo\n---\nbody").unwrap();
        }
        let catalog = skill_catalog(Runtime::Pi, home.path(), None).unwrap();
        assert_eq!(
            catalog.roots,
            [
                home.path().join(".pi/agent/skills"),
                home.path().join(".agents/skills")
            ]
        );
        assert_eq!(catalog.entries.len(), 2);
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::On));
    }

    #[test]
    fn antigravity_catalog_reads_both_personal_roots_without_global_toggles() {
        let home = tempfile::tempdir().unwrap();
        for (root, name) in [
            (".gemini/antigravity-cli/skills", "agy-skill"),
            (".gemini/skills", "gemini-skill"),
        ] {
            let path = home.path().join(root).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "---\ndescription: demo\n---\nbody").unwrap();
        }
        let catalog = skill_catalog(Runtime::Antigravity, home.path(), None).unwrap();
        assert_eq!(
            catalog.roots,
            [
                home.path().join(".gemini/antigravity-cli/skills"),
                home.path().join(".gemini/skills")
            ]
        );
        assert_eq!(catalog.entries.len(), 2);
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::On));
    }

    #[test]
    fn cursor_managed_skill_is_installed_once_in_shared_root() {
        let home = tempfile::tempdir().unwrap();
        crate::agent_skill::install(home.path(), &home.path().join("runner-data"), false).unwrap();
        let catalog = skill_catalog(Runtime::Cursor, home.path(), None).unwrap();
        assert_eq!(Runtime::Cursor.managed_skill_root(), Some(".agents/skills"));
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "runner");
        assert_eq!(
            catalog.entries[0].path,
            home.path().join(".agents/skills/runner")
        );
        assert!(!home.path().join(".cursor/skills").exists());
    }

    #[test]
    fn cursor_catalog_reads_both_personal_roots_without_global_toggles() {
        let home = tempfile::tempdir().unwrap();
        for (root, name) in [
            (".cursor/skills", "cursor-skill"),
            (".agents/skills", "shared-skill"),
        ] {
            let path = home.path().join(root).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "---\ndescription: demo\n---\nbody").unwrap();
        }
        let catalog = skill_catalog(Runtime::Cursor, home.path(), None).unwrap();
        assert_eq!(
            catalog.roots,
            [
                home.path().join(".cursor/skills"),
                home.path().join(".agents/skills")
            ]
        );
        assert_eq!(catalog.entries.len(), 2);
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::On));
    }

    #[test]
    fn copilot_catalog_reads_disabled_skills_from_settings() {
        let home = tempfile::tempdir().unwrap();
        for (root, name) in [
            (".copilot/skills", "copilot-skill"),
            (".agents/skills", "shared-skill"),
        ] {
            let path = home.path().join(root).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "---\ndescription: demo\n---\nbody").unwrap();
        }
        let catalog = skill_catalog(Runtime::Copilot, home.path(), None).unwrap();
        assert_eq!(
            catalog.roots,
            [
                home.path().join(".copilot/skills"),
                home.path().join(".agents/skills")
            ]
        );
        assert_eq!(catalog.entries.len(), 2);
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::On));
        let settings = home.path().join(".copilot/settings.json");
        std::fs::write(
            &settings,
            "// Copilot settings\n{\n  \"model\": \"gpt-5.4\",\n  \"disabledSkills\": [\"shared-skill\", 3]\n}\n",
        )
        .unwrap();
        let catalog = skill_catalog(Runtime::Copilot, home.path(), None).unwrap();
        assert_eq!(catalog.entries[0].name, "copilot-skill");
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        assert_eq!(catalog.entries[1].name, "shared-skill");
        assert_eq!(catalog.entries[1].global, GlobalState::Off);
        std::fs::write(&settings, "{\"disabledSkills\": \"shared-skill\"").unwrap();
        let catalog = skill_catalog(Runtime::Copilot, home.path(), None).unwrap();
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::On));
    }

    #[test]
    fn catalogs_sort_alphabetically_across_roots_and_runtimes() {
        let home = tempfile::tempdir().unwrap();
        let agents = home.path().join(".agents/skills");
        let legacy = home.path().join(".codex/skills");
        for (root, names) in [
            (&agents, vec!["Zebra", "Demo", ".hidden"]),
            (&legacy, vec!["apple", "demo", ".system"]),
        ] {
            for name in names {
                std::fs::create_dir_all(root.join(name)).unwrap();
                std::fs::write(
                    root.join(name).join("SKILL.md"),
                    if name.eq_ignore_ascii_case("demo") {
                        "---\nname: demo\n---\n"
                    } else {
                        ""
                    },
                )
                .unwrap();
            }
        }
        let catalog = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        assert_eq!(catalog.roots, [agents.clone(), legacy.clone()]);
        assert!(catalog.root_exists);
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["apple", "demo", "demo", "Zebra"]
        );
        assert_eq!(catalog.entries[1].path, agents.join("Demo"));
        assert_eq!(catalog.entries[2].path, legacy.join("demo"));
        for (index, entry) in catalog.entries.iter().rev().enumerate() {
            write(
                home.path(),
                &format!("folder-{index}"),
                "SKILL.md",
                &format!("---\nname: {}\n---\n", entry.name),
            );
        }
        let claude = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        assert_eq!(
            claude
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<Vec<_>>(),
            catalog
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<Vec<_>>()
        );
        let custom = home.path().join("custom-codex");
        let catalog = skill_catalog(Runtime::Codex, home.path(), Some(&custom)).unwrap();
        assert_eq!(catalog.roots, [agents.clone(), custom.join("skills")]);
        assert!(catalog.root_exists);
        assert_eq!(catalog.entries.len(), 2);
        std::fs::rename(&agents, home.path().join("moved-agents")).unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        assert!(catalog.root_exists);
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["apple", "demo"]
        );
        let catalog = skill_catalog(Runtime::Codex, home.path(), Some(&custom)).unwrap();
        assert!(!catalog.root_exists);
        assert!(catalog.entries.is_empty());
    }

    #[test]
    fn codex_missing_or_nonboolean_enabled_reads_on_without_failing_catalog() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".codex/skills/demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "").unwrap();
        for enabled in [
            "",
            "enabled = 'unexpected'",
            "enabled = true",
            "enabled = false",
        ] {
            std::fs::write(
                home.path().join(".codex/config.toml"),
                format!(
                    "[[skills.config]]\npath = {}\n{enabled}\n",
                    serde_json::json!(path.join("SKILL.md"))
                ),
            )
            .unwrap();
            assert_eq!(
                skill_catalog(Runtime::Codex, home.path(), None)
                    .unwrap()
                    .entries[0]
                    .global,
                if enabled == "enabled = false" {
                    GlobalState::Off
                } else {
                    GlobalState::On
                }
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_keep_targets_and_loops_terminate() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "").unwrap();
        let root = home.path().join(".claude/skills");
        std::fs::create_dir_all(&root).unwrap();
        symlink(&target, root.join("linked")).unwrap();
        symlink(&root, root.join("loop")).unwrap();
        symlink("self", root.join("self")).unwrap();
        symlink("missing", root.join("broken")).unwrap();
        let catalog = skill_catalog(Runtime::ClaudeCode, home.path(), None).unwrap();
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(
            catalog.entries[0].symlink,
            Some(target.canonicalize().unwrap())
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_same_symlink_target_in_two_roots_remains_two_entries() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("memory/demo");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "# Shared\n").unwrap();
        let roots = [
            home.path().join(".agents/skills"),
            home.path().join(".codex/skills"),
        ];
        for root in &roots {
            std::fs::create_dir_all(root).unwrap();
            symlink(&target, root.join("demo")).unwrap();
            symlink(root, root.join("loop")).unwrap();
        }
        std::fs::write(
            home.path().join(".codex/config.toml"),
            format!(
                "[[skills.config]]\npath = {}\nenabled = false\n",
                serde_json::json!(roots[1].join("demo/SKILL.md"))
            ),
        )
        .unwrap();
        let catalog = skill_catalog(Runtime::Codex, home.path(), None).unwrap();
        assert_eq!(catalog.entries.len(), 2);
        for (index, root) in roots.iter().enumerate() {
            assert_eq!(catalog.entries[index].path, root.join("demo"));
            assert_eq!(
                catalog.entries[index].symlink,
                Some(target.canonicalize().unwrap())
            );
        }
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        assert_eq!(catalog.entries[1].global, GlobalState::Off);
    }

    #[test]
    fn frontmatter_document_handles_blocks_crlf_and_bad_input() {
        let doc = parse_skill_document("---\r\nname: \"demo\"\r\ndescription: >-\r\n  First line.\r\n  Second line.\r\n---\r\n# Body\r\n");
        assert_eq!(
            doc.frontmatter,
            [
                ("name".into(), "demo".into()),
                ("description".into(), "First line. Second line.".into())
            ]
        );
        assert_eq!(doc.body, "# Body\r\n");
        assert!(doc.problem.is_none());
        for text in [
            "---\nname: demo",
            "---\ninvalid\n---",
            "---\n: : broken\n---",
            "---\nname: 'unclosed\n---",
            "---\nuser-invocable: maybe\n---",
        ] {
            assert!(parse_skill_document(text).problem.is_some(), "{text}");
        }
        assert_eq!(parse_skill_document("# plain").body, "# plain");
        assert_eq!(skill_name(r"..\..\safe:name"), Some("safe_name".into()));
    }
}
