use crate::model::Runtime;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

use crate::skills::{skill_catalog, SkillCatalog, SkillEntry};
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
    crate::runtimes::catalogs()
        .iter()
        .filter_map(|runtime| skill_catalog(runtime.name, &home, codex_home.as_deref()))
        .collect()
}

pub fn set_global_enabled(
    _core: &AppCore,
    runtime: Runtime,
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

pub fn read_skill(_core: &AppCore, runtime: Runtime, path: &Path) -> Result<String> {
    read_skill_at(&home_dir()?, codex_home().as_deref(), runtime, path)
}

pub fn save_skill(
    _core: &AppCore,
    runtime: Runtime,
    path: &Path,
    text: &str,
) -> Result<SkillEntry> {
    save_skill_at(&home_dir()?, codex_home().as_deref(), runtime, path, text)
}

pub(crate) fn catalog_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
) -> Result<SkillCatalog> {
    skill_catalog(runtime, home, codex_home)
        .ok_or_else(|| Error::msg(format!("runtime {runtime} has no skills catalog")))
}

pub(crate) fn find_entry(catalog: &SkillCatalog, path: &Path) -> Result<SkillEntry> {
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
    runtime: Runtime,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    let toggle = crate::runtimes::adapter(runtime).skills().toggle.ok_or_else(|| Error::msg("global skill on/off is only supported for Claude Code, Codex and GitHub Copilot CLI"))?;
    toggle(home, codex_home, skill_path, enabled)
}

fn read_skill_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
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
    runtime: Runtime,
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
    use crate::runtimes::codex::skills::{
        codex_config_tables, codex_override_matches, set_enabled as set_codex_enabled_at,
    };
    use crate::skills::GlobalState;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/skills/demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), "# Original\r\n").unwrap();
        (home, path)
    }

    fn copilot_fixture() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".copilot/skills/demo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: demo\n---\n# Original\n",
        )
        .unwrap();
        (home, path)
    }

    fn copilot_settings(home: &Path) -> String {
        std::fs::read_to_string(home.join(".copilot/settings.json")).unwrap()
    }

    #[test]
    fn copilot_toggle_writes_disabled_skills_by_frontmatter_name_and_preserves_siblings() {
        let (home, skill_path) = copilot_fixture();
        let original =
            "{\n  \"model\": \"gpt-5.4\",\n  \"footer\": {\n    \"showQuota\": true\n  }\n}\n";
        std::fs::write(home.path().join(".copilot/settings.json"), original).unwrap();
        let off =
            set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, false).unwrap();
        assert_eq!(off.entries[0].name, "demo-skill");
        assert_eq!(off.entries[0].global, GlobalState::Off);
        assert_eq!(
            copilot_settings(home.path()),
            "{\n  \"model\": \"gpt-5.4\",\n  \"footer\": {\n    \"showQuota\": true\n  },\n  \"disabledSkills\": [\n    \"demo-skill\"\n  ]\n}\n"
        );
        let on =
            set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, true).unwrap();
        assert_eq!(on.entries[0].global, GlobalState::On);
        assert_eq!(copilot_settings(home.path()), original);
    }

    #[test]
    fn copilot_toggle_keeps_unrelated_names_and_never_duplicates() {
        let (home, skill_path) = copilot_fixture();
        std::fs::write(
            home.path().join(".copilot/settings.json"),
            "{\"disabledSkills\":[\"other\",\"demo-skill\",\"demo-skill\"]}",
        )
        .unwrap();
        set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, false).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&copilot_settings(home.path())).unwrap(),
            json!({"disabledSkills": ["other", "demo-skill", "demo-skill"]})
        );
        set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, true).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&copilot_settings(home.path())).unwrap(),
            json!({"disabledSkills": ["other"]})
        );
    }

    #[test]
    fn copilot_toggle_creates_missing_settings_and_rejects_bad_shapes_without_writes() {
        let (home, skill_path) = copilot_fixture();
        set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, false).unwrap();
        assert_eq!(
            copilot_settings(home.path()),
            "{\n  \"disabledSkills\": [\n    \"demo-skill\"\n  ]\n}\n"
        );
        set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, true).unwrap();
        assert_eq!(copilot_settings(home.path()), "{}\n");
        for raw in ["{\"disabledSkills\": 3}", "[]", "{\"model\": "] {
            std::fs::write(home.path().join(".copilot/settings.json"), raw).unwrap();
            assert!(
                set_global_enabled_at(home.path(), None, Runtime::Copilot, &skill_path, false)
                    .is_err()
            );
            assert_eq!(copilot_settings(home.path()), raw);
        }
        assert!(set_global_enabled_at(
            home.path(),
            None,
            Runtime::Copilot,
            &home.path().join(".copilot/skills/missing"),
            false
        )
        .is_err());
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
            Runtime::Codex,
            &home.path().join(".codex/skills/demo"),
            true,
        )
        .unwrap();
        assert!(!config.exists());
        assert!(set_global_enabled_at(
            home.path(),
            None,
            Runtime::Codex,
            &home.path().join(".codex/skills/unknown"),
            false
        )
        .is_err());
        assert!(!config.exists());
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            Runtime::Codex,
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
            tables.get(0).unwrap()["path"].as_str().map(Path::new),
            Some(marker_dir.join("SKILL.md").as_path())
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
            Runtime::Codex,
            &home.path().join(".codex/skills/demo"),
            false,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        let catalog = set_global_enabled_at(
            home.path(),
            None,
            Runtime::Codex,
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
            let catalog = set_global_enabled_at(home.path(), None, Runtime::Codex, &home.path().join(".codex/skills/demo"), false).unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::Off);
            assert_eq!(std::fs::read_to_string(&config).unwrap(), original.replace("enabled   = true", "enabled   = false"));
            let catalog = set_global_enabled_at(home.path(), None, Runtime::Codex, &home.path().join(".codex/skills/demo"), true).unwrap();
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
            let off = set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, false).unwrap();
            assert_eq!(off.entries[0].global, GlobalState::Off);
            let on = set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true).unwrap();
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
            set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true).unwrap();
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
                set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, false)
                    .unwrap();
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
                set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true)
                    .unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::On);
            assert_eq!(
                std::fs::read_to_string(&config).unwrap(),
                format!("{named}{unrelated}{siblings}")
            );
            std::fs::write(&config, &original).unwrap();
            set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true).unwrap();
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
                catalog_at(home.path(), None, Runtime::Codex)
                    .unwrap()
                    .entries[0]
                    .global,
                GlobalState::On
            );
            assert_eq!(
                set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, false)
                    .unwrap()
                    .entries[0]
                    .global,
                GlobalState::Off
            );
            assert!(std::fs::read_to_string(&config).unwrap().starts_with(other));
            set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true).unwrap();
            assert_eq!(std::fs::read_to_string(&config).unwrap(), other);
            std::fs::write(&config, &original).unwrap();
            set_global_enabled_at(home.path(), None, Runtime::Codex, &skill_path, true).unwrap();
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
                    Runtime::Codex,
                    &home.path().join(".codex/skills/demo"),
                    enabled
                )
                .is_err());
                assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
            }
            assert!(skill_catalog(Runtime::Codex, home.path(), None).is_some());
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
            Runtime::ClaudeCode,
            &home.path().join(".claude/skills/demo"),
            false,
        )
        .unwrap();
        let claude_off = std::fs::read_to_string(&claude_config).unwrap();
        assert_eq!(
            catalog_at(home.path(), None, Runtime::Codex)
                .unwrap()
                .entries[0]
                .global,
            GlobalState::On
        );
        let alias = home.path().join(".codex/skills/demo/SKILL.md");
        let original = format!(
            "[[skills.config]]\npath = {}\nenabled = false\n",
            json!(alias)
        );
        std::fs::write(&codex_config, &original).unwrap();
        assert_eq!(
            catalog_at(home.path(), None, Runtime::Codex)
                .unwrap()
                .entries[0]
                .global,
            GlobalState::Off
        );
        set_global_enabled_at(
            home.path(),
            None,
            Runtime::Codex,
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
            Runtime::Codex,
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
            Runtime::ClaudeCode,
            &home.path().join(".claude/skills/demo"),
            true,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&codex_config).unwrap(), codex_off);
        assert_eq!(
            catalog_at(home.path(), None, Runtime::Codex)
                .unwrap()
                .entries[0]
                .global,
            GlobalState::Off
        );
        assert_eq!(
            catalog_at(home.path(), None, Runtime::ClaudeCode)
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
            read_skill_at(home.path(), None, Runtime::Codex, &agents).unwrap(),
            "# Agents\r\n"
        );
        assert_eq!(
            read_skill_at(home.path(), None, Runtime::Codex, &legacy).unwrap(),
            "# Legacy\n"
        );
        let replacement = "---\nname: renamed\n---\n# Changed\r\n";
        let changed =
            save_skill_at(home.path(), None, Runtime::Codex, &agents, replacement).unwrap();
        assert_eq!(changed.path, agents);
        assert_eq!(changed.name, "renamed");
        assert_eq!(
            read_skill_at(home.path(), None, Runtime::Codex, &agents).unwrap(),
            replacement
        );
        assert_eq!(
            read_skill_at(home.path(), None, Runtime::Codex, &legacy).unwrap(),
            "# Legacy\n"
        );
        let catalog =
            set_global_enabled_at(home.path(), None, Runtime::Codex, &legacy, false).unwrap();
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| (&entry.path, &entry.global))
                .collect::<Vec<_>>(),
            [(&legacy, &GlobalState::Off), (&agents, &GlobalState::On)]
        );
        let catalog =
            set_global_enabled_at(home.path(), None, Runtime::Codex, &agents, false).unwrap();
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.global == GlobalState::Off));
        let catalog =
            set_global_enabled_at(home.path(), None, Runtime::Codex, &legacy, true).unwrap();
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
            assert!(read_skill_at(home.path(), None, Runtime::Codex, &invalid).is_err());
            assert!(save_skill_at(home.path(), None, Runtime::Codex, &invalid, "bad").is_err());
            assert!(
                set_global_enabled_at(home.path(), None, Runtime::Codex, &invalid, false).is_err()
            );
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
            set_global_enabled_at(home.path(), codex_home, Runtime::Codex, &path, true).unwrap();
            assert!(!config_dir.exists());
            let catalog =
                set_global_enabled_at(home.path(), codex_home, Runtime::Codex, &path, false)
                    .unwrap();
            assert_eq!(catalog.entries[0].global, GlobalState::Off);
            let text = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
            let document = text.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(document.len(), 1);
            assert_eq!(
                document["skills"]["config"][0]["path"]
                    .as_str()
                    .map(Path::new),
                Some(path.join("SKILL.md").as_path())
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
        assert!(set_global_enabled_at(home.path(), None, Runtime::Codex, &path, false).is_err());
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            let off = set_global_enabled_at(home.path(), None, Runtime::ClaudeCode, &skill_path, false).unwrap();
            assert_eq!(off.entries[0].global, GlobalState::Off);
            let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(written["skillOverrides"]["demo"], "off");
            let on = set_global_enabled_at(home.path(), None, Runtime::ClaudeCode, &skill_path, true).unwrap();
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::Codex,
            &home.path().join(".codex/skills/demo"),
            false
        )
        .is_err());
        assert!(!home.path().join(".codex").exists());
        for runtime in [
            Runtime::Trae,
            Runtime::Pi,
            Runtime::Antigravity,
            Runtime::Shell,
        ] {
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
                Runtime::ClaudeCode,
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
                Runtime::ClaudeCode,
                &home.path().join(".claude/skills/demo")
            )
            .unwrap(),
            "# Original\r\n"
        );
        let text = "---\nname: renamed\ndescription: New description\ndisable-model-invocation: true\n---\n# Edited\n\n";
        let entry = save_skill_at(
            home.path(),
            None,
            Runtime::ClaudeCode,
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
                Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
                Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
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
            Runtime::ClaudeCode,
            &home.path().join(".claude/skills/demo")
        )
        .is_err());
        assert!(save_skill_at(
            home.path(),
            None,
            Runtime::ClaudeCode,
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
        save_skill_at(home.path(), None, Runtime::ClaudeCode, &other, "chosen").unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("SKILL.md")).unwrap(),
            "# Original\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(other.join("SKILL.md")).unwrap(),
            "chosen"
        );
        assert!(save_skill_at(
            home.path(),
            None,
            Runtime::ClaudeCode,
            Path::new("demo"),
            "bad"
        )
        .is_err());
    }
}

#[cfg(unix)]
#[test]
fn skills_catalog_golden() {
    let mut rows = Vec::new();
    for runtime in Runtime::ALL {
        let home = tempfile::tempdir().unwrap();
        let Some(catalog) = skill_catalog(runtime, home.path(), None) else {
            rows.push(serde_json::json!({"runtime":runtime,"roots":null}));
            continue;
        };
        for root in &catalog.roots {
            let skill = root.join("golden-skill");
            std::fs::create_dir_all(skill.join("agents")).unwrap();
            std::fs::write(skill.join("SKILL.md"), "---\nname: golden-skill\ndescription: Golden description.\ndisable-model-invocation: true\nuser-invocable: false\n---\n# Golden\n").unwrap();
            std::fs::write(
                skill.join("agents/openai.yaml"),
                "policy:\n  allow_implicit_invocation: false\n",
            )
            .unwrap();
        }
        let skill = catalog.roots[0].join("golden-skill");
        let mut writes = Vec::new();
        for enabled in [false, false, true, true] {
            let result = set_global_enabled_at(home.path(), None, runtime, &skill, enabled);
            let config = [
                ".claude/settings.json",
                ".codex/config.toml",
                ".copilot/settings.json",
            ]
            .into_iter()
            .filter_map(|relative| {
                std::fs::read_to_string(home.path().join(relative))
                    .ok()
                    .map(|text| (relative, text))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
            writes.push(serde_json::json!({"enabled":enabled,"error":result.err().map(|error| error.to_string()),"files":config}));
        }
        let catalog = skill_catalog(runtime, home.path(), None).unwrap();
        rows.push(serde_json::json!({"runtime":runtime,"roots":catalog.roots,"entries":catalog.entries.iter().map(|entry| serde_json::json!({"name":entry.name,"manual":entry.manual,"hidden":entry.hidden,"global":format!("{:?}",entry.global),"marker":entry.marker,"problem":entry.problem})).collect::<Vec<_>>(),"writes":writes}));
    }
    // Each row has its own home; normalize during construction below as well.
    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut row| {
            if let Some(roots) = row["roots"].as_array() {
                if let Some(root) = roots.first().and_then(|root| root.as_str()) {
                    let relative =
                        crate::model::Runtime::parse(row["runtime"].as_str().unwrap()).unwrap();
                    let root_suffix = crate::runtimes::adapter(relative)
                        .catalog()
                        .unwrap()
                        .skills_dirs[0];
                    let home = std::path::PathBuf::from(root)
                        .ancestors()
                        .nth(std::path::Path::new(root_suffix).components().count())
                        .unwrap()
                        .to_path_buf();
                    row = crate::golden::normalize(row, &home);
                }
            }
            row
        })
        .collect();
    crate::golden::assert_golden("catalog-skills", serde_json::json!(rows));
}
