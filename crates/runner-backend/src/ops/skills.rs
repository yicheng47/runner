use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::router::runtime::runtime_definitions;
use crate::skills::{
    codex_config_tables, codex_override_matches, skill_catalog, SkillCatalog, SkillEntry,
};
use crate::AppCore;

fn home_dir() -> Result<PathBuf> {
    runner_core::app_paths::home_dir().ok_or_else(|| Error::msg("home directory is not available"))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn skill_catalogs(_core: &AppCore) -> Vec<SkillCatalog> {
    let Ok(home) = home_dir() else {
        return Vec::new();
    };
    let codex_home = codex_home();
    runtime_definitions()
        .iter()
        .filter_map(|runtime| skill_catalog(runtime.name, &home, codex_home.as_deref()))
        .collect()
}

pub fn set_global_enabled(
    _core: &AppCore,
    runtime: &str,
    path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    set_global_enabled_at(
        &home_dir()?,
        codex_home().as_deref(),
        runtime,
        path,
        enabled,
    )
}

pub fn read_skill(_core: &AppCore, runtime: &str, path: &Path) -> Result<String> {
    read_skill_at(&home_dir()?, codex_home().as_deref(), runtime, path)
}

pub fn save_skill(_core: &AppCore, runtime: &str, path: &Path, text: &str) -> Result<SkillEntry> {
    save_skill_at(&home_dir()?, codex_home().as_deref(), runtime, path, text)
}

fn catalog_at(home: &Path, codex_home: Option<&Path>, runtime: &str) -> Result<SkillCatalog> {
    skill_catalog(runtime, home, codex_home)
        .ok_or_else(|| Error::msg(format!("runtime {runtime} has no skills catalog")))
}

fn find_entry(catalog: &SkillCatalog, path: &Path) -> Result<SkillEntry> {
    catalog
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .cloned()
        .ok_or_else(|| Error::msg(format!("unknown skill path: {}", path.display())))
}

fn set_global_enabled_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: &str,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    if runtime == "codex" {
        return set_codex_enabled_at(home, codex_home, skill_path, enabled);
    }
    if runtime != "claude-code" {
        return Err(Error::msg(
            "global skill on/off is only supported for Claude Code and Codex",
        ));
    }
    let entry = find_entry(&catalog_at(home, None, runtime)?, skill_path)?;
    let path = home.join(".claude/settings.json");
    let mut settings: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| Error::msg(format!("parse {}: {error}", path.display())))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    let object = settings
        .as_object_mut()
        .ok_or_else(|| Error::msg("settings.json is not a JSON object"))?;
    let overrides = object
        .entry("skillOverrides")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::msg("skillOverrides is not a JSON object"))?;
    if enabled {
        overrides.shift_remove(&entry.name);
        if overrides.is_empty() {
            object.shift_remove("skillOverrides");
        }
    } else {
        overrides.insert(entry.name, serde_json::json!("off"));
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&settings)?);
    std::fs::create_dir_all(home.join(".claude"))?;
    std::fs::write(path, text)?;
    catalog_at(home, None, runtime)
}

fn set_codex_enabled_at(
    home: &Path,
    codex_home: Option<&Path>,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    let catalog = catalog_at(home, codex_home, "codex")?;
    let entry = find_entry(&catalog, skill_path)?;
    if !entry.files.contains(&entry.marker) {
        return Err(Error::msg("skill has no marker file"));
    }
    let marker = entry.path.join(&entry.marker);
    let marker_text = marker
        .to_str()
        .ok_or_else(|| Error::msg("skill marker path is not valid UTF-8"))?;
    let config_dir = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex"));
    let path = config_dir.join("config.toml");
    let mut document = match std::fs::read_to_string(&path) {
        Ok(text) => text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| Error::msg(format!("parse {}: {error}", path.display())))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(error) => return Err(error.into()),
    };
    let mut matching = Vec::new();
    let preserve_skills = document
        .get("skills")
        .is_some_and(|skills| skills.as_table().is_none_or(|table| !table.is_implicit()));
    if let Some(skills) = document.get("skills") {
        let skills = skills
            .as_table_like()
            .ok_or_else(|| Error::msg("skills is not a table"))?;
        if let Some(config) = skills.get("config") {
            let tables = codex_config_tables(config)
                .ok_or_else(|| Error::msg("skills.config is not an array of tables"))?;
            matching = tables
                .iter()
                .enumerate()
                .filter(|(_, table)| codex_override_matches(**table, &marker))
                .map(|(index, _)| index)
                .collect();
        }
    }
    if enabled && matching.is_empty() {
        return Ok(catalog);
    }
    let mut new_skills = toml_edit::Table::new();
    new_skills.set_implicit(true);
    let skills = document
        .entry("skills")
        .or_insert(toml_edit::Item::Table(new_skills));
    let inline = skills.is_inline_table();
    let skills = skills
        .as_table_like_mut()
        .ok_or_else(|| Error::msg("skills is not a table"))?;
    let config = skills.entry("config").or_insert(if inline {
        toml_edit::value(toml_edit::Array::new())
    } else {
        toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new())
    });
    let keep = if enabled {
        None
    } else {
        matching.first().copied()
    };
    for index in matching
        .into_iter()
        .rev()
        .filter(|index| Some(*index) != keep)
    {
        if let Some(tables) = config.as_array_of_tables_mut() {
            tables.remove(index);
        } else if let Some(array) = config.as_array_mut() {
            array.remove(index);
        }
    }
    if let Some(index) = keep {
        let table = config
            .get_mut(index)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| Error::msg("invalid skills.config entry"))?;
        let mut value = toml_edit::Value::from(false);
        if let Some(previous) = table.get("enabled").and_then(toml_edit::Item::as_value) {
            *value.decor_mut() = previous.decor().clone();
        }
        if let Some(previous) = table.get_mut("enabled") {
            *previous = toml_edit::Item::Value(value);
        } else {
            table.insert("enabled", toml_edit::Item::Value(value));
        }
    } else if !enabled {
        if let Some(tables) = config.as_array_of_tables_mut() {
            let mut table = toml_edit::Table::new();
            table["path"] = toml_edit::value(marker_text);
            table["enabled"] = toml_edit::value(false);
            tables.push(table);
        } else if let Some(array) = config.as_array_mut() {
            let mut table = toml_edit::InlineTable::new();
            table.insert("path", marker_text.into());
            table.insert("enabled", false.into());
            array.push(table);
        }
    }
    if enabled && codex_config_tables(config).is_some_and(|tables| tables.is_empty()) {
        skills.remove("config");
        if skills.is_empty() && !preserve_skills {
            document.remove("skills");
        }
    }
    std::fs::create_dir_all(&config_dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(&path)?
        .write_all(document.to_string().as_bytes())?;
    catalog_at(home, codex_home, "codex")
}

fn read_skill_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: &str,
    path: &Path,
) -> Result<String> {
    let entry = find_entry(&catalog_at(home, codex_home, runtime)?, path)?;
    if !entry.files.contains(&entry.marker) {
        return Err(Error::msg("skill has no marker file"));
    }
    Ok(std::fs::read_to_string(entry.path.join(entry.marker))?)
}

fn save_skill_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: &str,
    path: &Path,
    text: &str,
) -> Result<SkillEntry> {
    let entry = find_entry(&catalog_at(home, codex_home, runtime)?, path)?;
    let marker = entry.path.join(&entry.marker);
    if !entry.files.contains(&entry.marker) || !marker.is_file() {
        return Err(Error::msg(format!(
            "no marker file at {}",
            marker.display()
        )));
    }
    std::fs::write(marker, text)?;
    catalog_at(home, codex_home, runtime)?
        .entries
        .into_iter()
        .find(|updated| updated.path == entry.path)
        .ok_or_else(|| Error::msg("skill is no longer in the catalog after saving"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::GlobalState;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/skills/demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "# Original\r\n").unwrap();
        (home, path)
    }

    fn codex_fixture() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".codex/skills/demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "# Original\n").unwrap();
        (home, path)
    }

    #[test]
    fn codex_toggle_creates_private_config_and_leaves_effective_noops_untouched() {
        let (home, marker_dir) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            true,
        )
        .unwrap();
        assert!(!config.exists());
        assert!(set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/unknown"),
            false
        )
        .is_err());
        assert!(!config.exists());
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::Off);
        let original = std::fs::read_to_string(&config).unwrap();
        assert!(!original.lines().any(|line| line.trim() == "[skills]"));
        let document = original.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document.len(), 1);
        let tables = document["skills"]["config"].as_array_of_tables().unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables.get(0).unwrap().len(), 2);
        assert_eq!(
            tables.get(0).unwrap()["path"].as_str(),
            marker_dir.join("SKILL.md").to_str()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        let restored = std::fs::read_to_string(&config)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert!(restored.is_empty());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
        assert_eq!(
            std::fs::read_to_string(marker_dir.join("SKILL.md")).unwrap(),
            "# Original\n"
        );
        assert!(!home.path().join(".claude").exists());
    }

    #[test]
    fn codex_toggle_preserves_toml_comments_siblings_and_inline_arrays() {
        let (home, skill_path) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        for template in [
            "# Personal config\nmodel = 'default'\n\n[skills]\nmax_context_tokens = 2000 # keep\n\n[[skills.config]]\nname = 'other'\nenabled = false # other\n\n[[skills.config]] # selected\npath = 'skills/demo/SKILL.md'\nenabled   = true # selected\nextra = 'untouched'\n\n[mcp_servers.runner]\ncommand = 'runner-mcp'\n",
            "# Personal config\n[skills]\nconfig = [{ name = 'other', enabled = false }, { path = 'skills/demo/SKILL.md', enabled   = true, extra = 'untouched' }] # keep\n\n[unrelated]\nvalue = 42\n",
            "skills = { config = [{ path = 'skills/demo/SKILL.md', enabled   = true }], extra = 'keep' } # keep\nmodel = 'default'\n",
        ] {
            let original = template.replace("'skills/demo/SKILL.md'", &json!(skill_path.join("SKILL.md")).to_string());
            std::fs::write(&config, &original).unwrap();
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
            }
            let catalog = set_global_enabled_at(home.path(), None, "codex", &home.path().join(".codex/skills/demo"), false).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::Off);
            assert_eq!(std::fs::read_to_string(&config).unwrap(), original.replace("enabled   = true", "enabled   = false"));
            let catalog = set_global_enabled_at(home.path(), None, "codex", &home.path().join(".codex/skills/demo"), true).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::On);
            let removed = std::fs::read_to_string(&config).unwrap();
            let document = removed.parse::<toml_edit::DocumentMut>().unwrap();
            if template.starts_with("skills =") {
                assert!(document["skills"].get("config").is_none());
                assert_eq!(document["skills"]["extra"].as_str(), Some("keep"));
            } else {
                let tables = codex_config_tables(&document["skills"]["config"]).unwrap();
                assert_eq!(tables.len(), 1);
                assert_eq!(tables[0].get("name").and_then(toml_edit::Item::as_str), Some("other"));
                assert!(tables.iter().all(|table| !codex_override_matches(*table, &skill_path.join("SKILL.md"))));
            }
            if template.contains("[unrelated]") { assert!(removed.ends_with("[unrelated]\nvalue = 42\n")); }
            if template.contains("mcp_servers") { assert!(removed.ends_with("[mcp_servers.runner]\ncommand = 'runner-mcp'\n")); }
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o640);
            }
        }
    }

    #[test]
    fn codex_off_on_round_trip_restores_original_config_bytes() {
        let (home, skill_path) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        for original in [
            "# Personal config\nmodel = 'default'\n\n[projects.'/repo']\ntrust_level = 'trusted' # keep\n\n[mcp_servers.runner]\ncommand = 'runner-mcp'\n\n[tui]\nnotifications = true\n",
            "# Personal config\nmodel = 'default'\n\n[skills] # user policy\nmax_context_tokens = 2000 # keep\n\n[mcp_servers.runner]\ncommand = 'runner-mcp'\n",
            "model = 'default'\n\n[skills]\nmax_context_tokens = 2000\n",
            "model = 'default'\n\n[skills.options]\ncustom = true\n",
            "skills = { extra = 'keep' } # inline policy\nmodel = 'default'\n",
            "# Keep this note\n[skills] # user policy\n",
            "",
        ] {
            std::fs::write(&config, original).unwrap();
            let off = set_global_enabled_at(home.path(), None, "codex", &skill_path, false).unwrap();
            assert_eq!(off.entries[0].global, GlobalState::Off);
            let on = set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(on.entries[0].global, GlobalState::On);
            assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes(), "{original}");
        }
    }

    #[test]
    fn codex_enabling_removes_empty_config_but_preserves_explicit_skills() {
        let (home, skill_path) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        let marker = json!(skill_path.join("SKILL.md")).to_string();
        for (template, expected) in [
            ("model = 'default'\n[[skills.config]]\npath = MARKER\nenabled = false\n", "model = 'default'\n"),
            ("model = 'default'\n[skills]\n[[skills.config]]\npath = MARKER\nenabled = false\n", "model = 'default'\n[skills]\n"),
            ("# Keep this note\n[skills] # user policy\n\n[[skills.config]]\npath = MARKER\nenabled = false\n", "# Keep this note\n[skills] # user policy\n"),
            ("model = 'default'\n[skills]\nconfig = [{ path = MARKER, enabled = false }]\n", "model = 'default'\n[skills]\n"),
            ("skills = { config = [{ path = MARKER, enabled = false }] }\nmodel = 'default'\n", "skills = {}\nmodel = 'default'\n"),
            ("model = 'default'\n[skills]\nconfig = [{ path = MARKER, enabled = false }]\nextra = 'keep'\n", "model = 'default'\n[skills]\nextra = 'keep'\n"),
        ] {
            std::fs::write(&config, template.replace("MARKER", &marker)).unwrap();
            set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(std::fs::read_to_string(&config).unwrap(), expected);
        }
    }

    #[test]
    fn codex_toggle_deduplicates_matching_paths_and_preserves_unrelated_entries() {
        let (home, skill_path) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        let named = "[[skills.config]]\nname = 'other'\nenabled = false # user name rule\n";
        let path = format!(
            "[[skills.config]]\npath = {}\nenabled = true # selected\n",
            json!(skill_path.join("SKILL.md"))
        );
        let unrelated = "[[skills.config]]\npath = '/other/SKILL.md' # missing enabled\n";
        let siblings = "[model]\nname = 'default' # model comment\n\n[projects.'/repo']\ntrust_level = 'trusted'\n\n[mcp_servers.runner]\ncommand = 'runner-mcp'\n\n[tui]\nnotifications = true # keep\n";
        for original in [
            format!("{named}{path}{unrelated}{path}{siblings}"),
            format!("{path}{path}{named}{unrelated}{siblings}"),
        ] {
            std::fs::write(&config, &original).unwrap();
            let catalog =
                set_global_enabled_at(home.path(), None, "codex", &skill_path, false).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::Off);
            let written = std::fs::read_to_string(&config).unwrap();
            assert_eq!(
                written
                    .matches(&json!(skill_path.join("SKILL.md")).to_string())
                    .count(),
                1
            );
            assert!(written.contains(named));
            assert!(written.contains(unrelated));
            assert!(written.ends_with(siblings));
            let catalog =
                set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::On);
            assert_eq!(
                std::fs::read_to_string(&config).unwrap(),
                format!("{named}{unrelated}{siblings}")
            );
            std::fs::write(&config, &original).unwrap();
            set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(
                std::fs::read_to_string(&config).unwrap(),
                format!("{named}{unrelated}{siblings}")
            );
        }
    }

    #[test]
    fn codex_toggle_repairs_matching_missing_enabled_and_keeps_other_incomplete_tables() {
        let (home, skill_path) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        for enabled in ["", "enabled = 'unexpected'\n"] {
            let other = "[[skills.config]]\npath = '/music/SKILL.md'\n";
            let original = format!(
                "{other}[[skills.config]]\npath = {}\n{enabled}",
                json!(skill_path.join("SKILL.md"))
            );
            std::fs::write(&config, &original).unwrap();
            assert_eq!(
                catalog_at(home.path(), None, "codex").unwrap().entries[0].global,
                GlobalState::On
            );
            assert_eq!(
                set_global_enabled_at(home.path(), None, "codex", &skill_path, false)
                    .unwrap()
                    .entries[0]
                    .global,
                GlobalState::Off
            );
            assert!(std::fs::read_to_string(&config).unwrap().starts_with(other));
            set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(std::fs::read_to_string(&config).unwrap(), other);
            std::fs::write(&config, &original).unwrap();
            set_global_enabled_at(home.path(), None, "codex", &skill_path, true).unwrap();
            assert_eq!(std::fs::read_to_string(&config).unwrap(), other);
        }
    }

    #[test]
    fn codex_toggle_uses_codex_home_and_rejects_invalid_configs_without_writes() {
        let (home, _) = codex_fixture();
        let config = home.path().join(".codex/config.toml");
        for original in [
            "invalid = [",
            "skills = false",
            "skills.config = false",
            "skills.config = [3]",
        ] {
            std::fs::write(&config, original).unwrap();
            for enabled in [false, true] {
                assert!(set_global_enabled_at(
                    home.path(),
                    None,
                    "codex",
                    &home.path().join(".codex/skills/demo"),
                    enabled
                )
                .is_err());
                assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
            }
            assert!(skill_catalog("codex", home.path(), None).is_some());
        }
        let custom = home.path().join("custom-codex");
        std::fs::create_dir_all(custom.join("skills/demo")).unwrap();
        std::fs::write(custom.join("skills/demo/SKILL.md"), "# Custom\n").unwrap();
        let before = std::fs::read_to_string(&config).unwrap();
        let catalog = set_codex_enabled_at(
            home.path(),
            Some(&custom),
            &custom.join("skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(
            catalog.roots,
            [home.path().join(".agents/skills"), custom.join("skills")]
        );
        assert_eq!(catalog.entries[0].global, GlobalState::Off);
        assert!(custom.join("config.toml").is_file());
        assert_eq!(std::fs::read_to_string(config).unwrap(), before);
        assert!(!home.path().join(".claude").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_skill_enablement_is_independent_between_runtimes() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("memory/demo");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "# Shared content\n").unwrap();
        for runtime in [".claude", ".codex"] {
            std::fs::create_dir_all(home.path().join(runtime).join("skills")).unwrap();
            symlink(&target, home.path().join(runtime).join("skills/demo")).unwrap();
        }
        let claude_config = home.path().join(".claude/settings.json");
        let codex_config = home.path().join(".codex/config.toml");
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            false,
        )
        .unwrap();
        let claude_off = std::fs::read_to_string(&claude_config).unwrap();
        assert_eq!(
            catalog_at(home.path(), None, "codex").unwrap().entries[0].global,
            GlobalState::On
        );
        let alias = home.path().join(".codex/skills/demo/SKILL.md");
        let original = format!(
            "[[skills.config]]\npath = {}\nenabled = false\n",
            json!(alias)
        );
        std::fs::write(&codex_config, &original).unwrap();
        assert_eq!(
            catalog_at(home.path(), None, "codex").unwrap().entries[0].global,
            GlobalState::Off
        );
        set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            true,
        )
        .unwrap();
        assert!(!std::fs::read_to_string(&codex_config)
            .unwrap()
            .contains("[[skills.config]]"));
        assert_eq!(std::fs::read_to_string(&claude_config).unwrap(), claude_off);
        set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            false,
        )
        .unwrap();
        let codex_off = std::fs::read_to_string(&codex_config).unwrap();
        let document = codex_off.parse::<toml_edit::DocumentMut>().unwrap();
        let written_path = document["skills"]["config"][0]["path"].as_str().unwrap();
        assert_eq!(Path::new(written_path), alias);
        assert_ne!(Path::new(written_path), target.join("SKILL.md"));
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&codex_config).unwrap(), codex_off);
        assert_eq!(
            catalog_at(home.path(), None, "codex").unwrap().entries[0].global,
            GlobalState::Off
        );
        assert_eq!(
            catalog_at(home.path(), None, "claude-code")
                .unwrap()
                .entries[0]
                .global,
            GlobalState::On
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "# Shared content\n"
        );
        assert!(home.path().join(".claude/skills/demo").is_symlink());
        assert!(home.path().join(".codex/skills/demo").is_symlink());
    }

    #[test]
    fn codex_duplicate_names_read_save_and_toggle_only_the_selected_root() {
        let home = tempfile::tempdir().unwrap();
        let agents = home.path().join(".agents/skills/demo");
        let legacy = home.path().join(".codex/skills/demo");
        for (path, text) in [(&agents, "# Agents\r\n"), (&legacy, "# Legacy\n")] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join("SKILL.md"), text).unwrap();
        }
        assert_eq!(
            read_skill_at(home.path(), None, "codex", &agents).unwrap(),
            "# Agents\r\n"
        );
        assert_eq!(
            read_skill_at(home.path(), None, "codex", &legacy).unwrap(),
            "# Legacy\n"
        );
        let replacement = "---\nname: renamed\n---\n# Changed\r\n";
        let changed = save_skill_at(home.path(), None, "codex", &agents, replacement).unwrap();
        assert_eq!(changed.path, agents);
        assert_eq!(changed.name, "renamed");
        assert_eq!(
            read_skill_at(home.path(), None, "codex", &agents).unwrap(),
            replacement
        );
        assert_eq!(
            read_skill_at(home.path(), None, "codex", &legacy).unwrap(),
            "# Legacy\n"
        );
        let catalog = set_global_enabled_at(home.path(), None, "codex", &legacy, false).unwrap();
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| (&entry.path, &entry.global))
                .collect::<Vec<_>>(),
            [(&legacy, &GlobalState::Off), (&agents, &GlobalState::On)]
        );
        let catalog = set_global_enabled_at(home.path(), None, "codex", &agents, false).unwrap();
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::Off));
        let catalog = set_global_enabled_at(home.path(), None, "codex", &legacy, true).unwrap();
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| (&entry.path, &entry.global))
                .collect::<Vec<_>>(),
            [(&legacy, &GlobalState::On), (&agents, &GlobalState::Off)]
        );
        let config = home.path().join(".codex/config.toml");
        let before = std::fs::read_to_string(&config).unwrap();
        for invalid in [
            PathBuf::from("demo"),
            home.path().join("outside"),
            agents.join("../demo"),
        ] {
            assert!(read_skill_at(home.path(), None, "codex", &invalid).is_err());
            assert!(save_skill_at(home.path(), None, "codex", &invalid, "bad").is_err());
            assert!(set_global_enabled_at(home.path(), None, "codex", &invalid, false).is_err());
        }
        assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(agents.join("SKILL.md")).unwrap(),
            replacement
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("SKILL.md")).unwrap(),
            "# Legacy\n"
        );
    }

    #[test]
    fn codex_agents_only_root_creates_config_in_default_or_custom_home() {
        for custom in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let path = home.path().join(".agents/skills/demo");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("SKILL.md"), "# Agents\n").unwrap();
            let config_dir = home.path().join(if custom { "custom" } else { ".codex" });
            let codex_home = custom.then_some(config_dir.as_path());
            set_global_enabled_at(home.path(), codex_home, "codex", &path, true).unwrap();
            assert!(!config_dir.exists());
            let catalog =
                set_global_enabled_at(home.path(), codex_home, "codex", &path, false).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::Off);
            let text = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
            let document = text.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(document.len(), 1);
            assert_eq!(
                document["skills"]["config"][0]["path"].as_str(),
                path.join("SKILL.md").to_str()
            );
            assert!(!config_dir.join("skills").exists());
            if custom {
                assert!(!home.path().join(".codex").exists());
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_non_utf8_marker_path_errors_without_writing_config() {
        use std::os::unix::ffi::OsStringExt;
        let home = tempfile::tempdir().unwrap();
        let path = home
            .path()
            .join(".agents/skills")
            .join(std::ffi::OsString::from_vec(vec![0xff]));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "# Skill").unwrap();
        assert!(set_global_enabled_at(home.path(), None, "codex", &path, false).is_err());
        assert!(!home.path().join(".codex").exists());
        assert_eq!(
            std::fs::read_to_string(path.join("SKILL.md")).unwrap(),
            "# Skill"
        );
    }

    #[test]
    fn toggles_preserve_siblings_and_remove_only_the_selected_override() {
        let (home, _) = fixture();
        let path = home.path().join(".claude/settings.json");
        let original = json!({"theme":"dark", "permissions":{"allow":["Read"]}, "skillOverrides":{"demo":"on","other":"user-invocable-only"}});
        std::fs::write(&path, original.to_string()).unwrap();
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::Off);
        let mut expected = original.clone();
        expected["skillOverrides"]["demo"] = json!("off");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
                .unwrap(),
            expected
        );
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(catalog.entries[0].global, GlobalState::On);
        expected["skillOverrides"]
            .as_object_mut()
            .unwrap()
            .remove("demo");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(path).unwrap())
                .unwrap(),
            expected
        );
    }

    #[test]
    fn toggles_preserve_json_key_order_without_the_app_feature_graph() {
        let (home, _) = fixture();
        let path = home.path().join(".claude/settings.json");
        let original = "{\n  \"permissions\": {\n    \"deny\": [],\n    \"allow\": []\n  },\n  \"model\": \"default\",\n  \"theme\": \"dark\",\n  \"skillOverrides\": {\n    \"zebra\": \"name-only\",\n    \"demo\": \"on\",\n    \"alpha\": \"off\",\n    \"beta\": \"on\"\n  }\n}\n";
        std::fs::write(&path, original).unwrap();
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("\"demo\": \"on\"", "\"demo\": \"off\"")
        );
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("    \"demo\": \"on\",\n", "")
        );
    }

    #[test]
    fn claude_off_on_round_trip_removes_empty_overrides_and_preserves_other_entries() {
        let (home, skill_path) = fixture();
        let path = home.path().join(".claude/settings.json");
        for original in [
            "{\n  \"permissions\": {\n    \"deny\": [],\n    \"allow\": [\n      \"Read\"\n    ]\n  },\n  \"theme\": \"dark\"\n}\n",
            "{\n  \"permissions\": {\n    \"deny\": [],\n    \"allow\": []\n  },\n  \"skillOverrides\": {\n    \"zebra\": \"name-only\",\n    \"alpha\": \"off\",\n    \"beta\": \"on\"\n  },\n  \"theme\": \"dark\"\n}\n",
            "{}",
        ] {
            std::fs::write(&path, original).unwrap();
            let off = set_global_enabled_at(home.path(), None, "claude-code", &skill_path, false).unwrap();
            assert_eq!(off.entries[0].global, GlobalState::Off);
            let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(written["skillOverrides"]["demo"], "off");
            let on = set_global_enabled_at(home.path(), None, "claude-code", &skill_path, true).unwrap();
            assert_eq!(on.entries[0].global, GlobalState::On);
            let restored = std::fs::read_to_string(&path).unwrap();
            assert_eq!(restored, format!("{}\n", original.strip_suffix('\n').unwrap_or(original)));
            let restored: serde_json::Value = serde_json::from_str(&restored).unwrap();
            let original: serde_json::Value = serde_json::from_str(original).unwrap();
            assert_eq!(restored.get("skillOverrides"), original.get("skillOverrides"));
        }
    }

    #[test]
    fn toggle_creates_missing_settings_and_rejects_other_runtimes() {
        let (home, _) = fixture();
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap()
            )
            .unwrap(),
            json!({"skillOverrides":{"demo":"off"}})
        );
        set_global_enabled_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap(),
            "{}\n"
        );
        assert!(set_global_enabled_at(
            home.path(),
            None,
            "codex",
            &home.path().join(".codex/skills/demo"),
            false
        )
        .is_err());
        assert!(!home.path().join(".codex").exists());
        for runtime in ["qoder", "trae", "unknown"] {
            assert!(set_global_enabled_at(
                home.path(),
                None,
                runtime,
                &home.path().join(".claude/skills/demo"),
                false
            )
            .is_err());
        }
    }

    #[test]
    fn malformed_settings_are_never_overwritten() {
        let (home, _) = fixture();
        let path = home.path().join(".claude/settings.json");
        for text in [
            "{broken",
            "",
            "[]",
            r#"{"skillOverrides":false,"theme":"dark"}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(set_global_enabled_at(
                home.path(),
                None,
                "claude-code",
                &home.path().join(".claude/skills/demo"),
                false
            )
            .is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn read_is_fresh_and_save_is_exact_and_reparsed() {
        let (home, path) = fixture();
        std::fs::write(path.join("other.txt"), "leave alone").unwrap();
        assert_eq!(
            read_skill_at(
                home.path(),
                None,
                "claude-code",
                &home.path().join(".claude/skills/demo")
            )
            .unwrap(),
            "# Original\r\n"
        );
        let text = "---\nname: renamed\ndescription: New description\ndisable-model-invocation: true\n---\n# Edited\n\n";
        let entry = save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            text,
        )
        .unwrap();
        assert_eq!(entry.name, "renamed");
        assert_eq!(entry.description, "New description");
        assert!(entry.manual);
        assert_eq!(
            std::fs::read_to_string(path.join("SKILL.md")).unwrap(),
            text
        );
        assert_eq!(
            read_skill_at(
                home.path(),
                None,
                "claude-code",
                &home.path().join(".claude/skills/demo")
            )
            .unwrap(),
            text
        );
        assert_eq!(
            std::fs::read_to_string(path.join("other.txt")).unwrap(),
            "leave alone"
        );
        assert!(!home.path().join(".claude/settings.json").exists());
    }

    #[test]
    fn legacy_save_unknown_names_and_missing_markers() {
        let (home, path) = fixture();
        std::fs::rename(path.join("SKILL.md"), path.join("skill.md")).unwrap();
        let entry = save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            "legacy edit",
        )
        .unwrap();
        assert_eq!(entry.marker, "skill.md");
        assert_eq!(
            std::fs::read_to_string(path.join("skill.md")).unwrap(),
            "legacy edit"
        );
        assert!(!std::fs::read_dir(&path)
            .unwrap()
            .any(|entry| entry.unwrap().file_name() == "SKILL.md"));
        for name in ["unknown", "../demo", "../../settings.json"] {
            assert!(save_skill_at(
                home.path(),
                None,
                "claude-code",
                &home.path().join(".claude/skills").join(name),
                "bad"
            )
            .is_err());
        }
        let missing = home.path().join(".claude/skills/missing");
        std::fs::create_dir_all(&missing).unwrap();
        assert!(save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/missing"),
            "bad"
        )
        .is_err());
        assert!(!missing.join("SKILL.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_follows_folder_symlink_and_unwritable_file_returns_error() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (home, path) = fixture();
        let target = home.path().join("target");
        std::fs::rename(&path, &target).unwrap();
        symlink(&target, &path).unwrap();
        let entry = save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            "through the link",
        )
        .unwrap();
        assert_eq!(entry.symlink, Some(target.canonicalize().unwrap()));
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        let marker = target.join("SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "through the link"
        );
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o444)).unwrap();
        let result = save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            "forbidden",
        );
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "through the link");
    }

    #[test]
    fn wrong_marker_casing_cannot_be_read_or_written() {
        let (home, path) = fixture();
        std::fs::rename(path.join("SKILL.md"), path.join("Skill.md")).unwrap();
        assert!(read_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo")
        )
        .is_err());
        assert!(save_skill_at(
            home.path(),
            None,
            "claude-code",
            &home.path().join(".claude/skills/demo"),
            "bad"
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(path.join("Skill.md")).unwrap(),
            "# Original\r\n"
        );
    }

    #[test]
    fn duplicate_names_are_selected_by_path_without_writing_the_other_skill() {
        let (home, path) = fixture();
        let other = path.with_file_name("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
        save_skill_at(home.path(), None, "claude-code", &other, "chosen").unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("SKILL.md")).unwrap(),
            "# Original\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(other.join("SKILL.md")).unwrap(),
            "chosen"
        );
        assert!(save_skill_at(home.path(), None, "claude-code", Path::new("demo"), "bad").is_err());
    }
}
