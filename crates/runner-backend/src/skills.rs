use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::router::runtime::runtime_definition;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalState {
    On,
    Off,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub marker: String,
    pub symlink: Option<PathBuf>,
    pub manual: bool,
    pub hidden: bool,
    pub problem: Option<String>,
    pub files: Vec<String>,
    pub global: GlobalState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillCatalog {
    pub runtime: String,
    pub roots: Vec<PathBuf>,
    pub root_exists: bool,
    pub entries: Vec<SkillEntry>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct SkillDocument {
    pub frontmatter: Vec<(String, String)>,
    pub body: String,
    pub problem: Option<String>,
}

pub fn parse_skill_document(text: &str) -> SkillDocument {
    let mut document = SkillDocument {
        body: text.to_owned(),
        ..SkillDocument::default()
    };
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next().filter(|line| line.trim_end() == "---") else {
        return document;
    };
    let mut offset = first.len();
    let mut closed = false;
    let mut block: Option<(String, bool, Vec<String>)> = None;
    for line in lines {
        offset += line.len();
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, _, values)) = block.as_mut() {
                values.push(line.trim().to_owned());
            }
            continue;
        }
        finish_block(&mut block, &mut document.frontmatter);
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line
            .split_once(':')
            .filter(|(key, _)| !key.trim().is_empty())
        else {
            document.problem = Some("malformed frontmatter".into());
            continue;
        };
        let key = key.trim();
        if !matches!(
            key,
            "name" | "description" | "disable-model-invocation" | "user-invocable"
        ) {
            continue;
        }
        let value = value.trim();
        if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            block = Some((key.into(), value.starts_with('>'), Vec::new()));
            continue;
        }
        let value = if value.starts_with('"') {
            serde_json::from_str::<String>(value).ok()
        } else if value.starts_with('\'') {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .map(|value| value.replace("''", "'"))
        } else if value.starts_with(['[', '{']) {
            None
        } else {
            Some(
                value
                    .split(" #")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            )
        };
        match value {
            Some(value)
                if !matches!(key, "disable-model-invocation" | "user-invocable")
                    || matches!(value.as_str(), "true" | "false") =>
            {
                document.frontmatter.push((key.into(), value));
            }
            _ => document.problem = Some(format!("malformed frontmatter: {key}")),
        }
    }
    finish_block(&mut block, &mut document.frontmatter);
    if closed {
        document.body = text[offset..].to_owned();
    } else {
        document.problem = Some("unclosed frontmatter".into());
    }
    document
}

fn finish_block(block: &mut Option<(String, bool, Vec<String>)>, rows: &mut Vec<(String, String)>) {
    if let Some((key, folded, values)) = block.take() {
        rows.push((key, values.join(if folded { " " } else { "\n" })));
    }
}

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

pub(crate) fn codex_config_tables(
    item: &toml_edit::Item,
) -> Option<Vec<&dyn toml_edit::TableLike>> {
    if let Some(tables) = item.as_array_of_tables() {
        Some(
            tables
                .iter()
                .map(|table| table as &dyn toml_edit::TableLike)
                .collect(),
        )
    } else {
        item.as_array()?
            .iter()
            .map(|value| {
                value
                    .as_inline_table()
                    .map(|table| table as &dyn toml_edit::TableLike)
            })
            .collect()
    }
}

pub(crate) fn codex_override_matches(table: &dyn toml_edit::TableLike, marker: &Path) -> bool {
    table
        .get("path")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|path| Path::new(path) == marker)
}

fn codex_global_state(document: &toml_edit::DocumentMut, entry: &SkillEntry) -> GlobalState {
    let marker = entry.path.join(&entry.marker);
    let enabled = document
        .get("skills")
        .and_then(|skills| skills.get("config"))
        .and_then(codex_config_tables)
        .and_then(|tables| {
            tables
                .into_iter()
                .rev()
                .find(|table| codex_override_matches(*table, &marker))
        })
        .and_then(|table| table.get("enabled"))
        .and_then(toml_edit::Item::as_bool);
    if enabled == Some(false) {
        GlobalState::Off
    } else {
        GlobalState::On
    }
}

pub fn skill_catalog(
    runtime: &str,
    home: &Path,
    codex_home: Option<&Path>,
) -> Option<SkillCatalog> {
    let relatives = runtime_definition(runtime)?.skills_dirs;
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
    let config_dir = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex"));
    let codex_overrides = (runtime == "codex")
        .then(|| {
            std::fs::read_to_string(config_dir.join("config.toml"))
                .ok()
                .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        })
        .flatten();
    let overrides = if runtime == "claude-code" {
        std::fs::read_to_string(home.join(".claude/settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value.get("skillOverrides").cloned())
    } else {
        None
    };
    let mut catalog = SkillCatalog {
        runtime: runtime.into(),
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
                entry.global = match overrides.as_ref().and_then(|value| value.get(&entry.name)) {
                    None => GlobalState::On,
                    Some(value) => match value.as_str() {
                        Some("on") => GlobalState::On,
                        Some("off") => GlobalState::Off,
                        Some(other) => GlobalState::Other(other.into()),
                        None => GlobalState::Other(value.to_string()),
                    },
                };
                if let Some(document) = &codex_overrides {
                    entry.global = codex_global_state(document, &entry);
                }
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

fn codex_manual(text: &str) -> bool {
    let mut in_policy = false;
    let mut child_indent = None;
    let mut manual = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent == 0 {
            in_policy = trimmed
                .split_once(':')
                .is_some_and(|(key, value)| key.trim() == "policy" && value.trim().is_empty());
            child_indent = None;
        } else if in_policy && indent == *child_indent.get_or_insert(indent) {
            if let Some((key, value)) = trimmed.split_once(':') {
                if key.trim() == "allow_implicit_invocation" {
                    manual = value.trim() == "false";
                }
            }
        }
    }
    manual
}

fn read_entry(path: &Path, marker: &str, runtime: &str) -> SkillEntry {
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
        manual: if runtime == "codex" {
            std::fs::read_to_string(path.join("agents/openai.yaml"))
                .is_ok_and(|text| codex_manual(&text))
        } else {
            value("disable-model-invocation") == Some("true")
        },
        hidden: runtime == "claude-code" && value("user-invocable") == Some("false"),
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
        let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let catalog = skill_catalog("codex", home.path(), None).unwrap();
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
            let catalog = skill_catalog("codex", home.path(), None).unwrap();
            assert_eq!(
                catalog.entries[0].manual,
                policy.contains("allow_implicit_invocation: false")
            );
            assert!(!catalog.entries[0].hidden);
            assert_eq!(catalog.entries[0].global, GlobalState::On);
            assert!(catalog.entries[0].problem.is_none());
            let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
        assert!(!catalog.entries[0].manual && !catalog.entries[0].hidden);
        std::fs::write(codex.join("agents/openai.yaml"), [0xff]).unwrap();
        assert!(!skill_catalog("codex", home.path(), None).unwrap().entries[0].manual);
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
        let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let entries = skill_catalog("claude-code", home.path(), None)
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
        assert!(skill_catalog("claude-code", home.path(), None)
            .unwrap()
            .entries
            .iter()
            .all(|e| e.global == GlobalState::On));
    }

    #[test]
    fn roots_and_unsupported_runtimes() {
        let home = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let empty = skill_catalog("codex", home.path(), None).unwrap();
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
        let catalog = skill_catalog("codex", home.path(), Some(custom.path())).unwrap();
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
        let catalog = skill_catalog("codex", home.path(), Some(custom.path())).unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::Off);
        std::fs::write(custom.path().join("config.toml"), "invalid = [").unwrap();
        let catalog = skill_catalog("codex", home.path(), Some(custom.path())).unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        for runtime in ["qoder", "trae", "unknown"] {
            assert!(skill_catalog(runtime, home.path(), None).is_none());
        }
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
        let catalog = skill_catalog("codex", home.path(), None).unwrap();
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
        let claude = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let catalog = skill_catalog("codex", home.path(), Some(&custom)).unwrap();
        assert_eq!(catalog.roots, [agents.clone(), custom.join("skills")]);
        assert!(catalog.root_exists);
        assert_eq!(catalog.entries.len(), 2);
        std::fs::rename(&agents, home.path().join("moved-agents")).unwrap();
        let catalog = skill_catalog("codex", home.path(), None).unwrap();
        assert!(catalog.root_exists);
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["apple", "demo"]
        );
        let catalog = skill_catalog("codex", home.path(), Some(&custom)).unwrap();
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
                skill_catalog("codex", home.path(), None).unwrap().entries[0].global,
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
        let catalog = skill_catalog("claude-code", home.path(), None).unwrap();
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
        let catalog = skill_catalog("codex", home.path(), None).unwrap();
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
