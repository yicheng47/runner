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

pub fn skill_catalogs(core: &AppCore, account_id: Option<&str>) -> Result<Vec<SkillCatalog>> {
    let Ok(home) = home_dir() else {
        return Ok(Vec::new());
    };
    let codex_home = codex_home();
    let env = core
        .runtime_shell_env
        .read()
        .map(|env| env.vars.clone())
        .unwrap_or_default();
    let env = crate::claude_accounts::sharing::account_config_env(&core.db, env, account_id)?;
    Ok(crate::runtimes::catalogs()
        .iter()
        .filter_map(|runtime| {
            crate::skills::skill_catalog_with_env(runtime.name, &home, codex_home.as_deref(), &env)
        })
        .collect())
}

pub fn set_global_enabled(
    core: &AppCore,
    runtime: Runtime,
    path: &Path,
    enabled: bool,
    account_id: Option<&str>,
) -> Result<SkillCatalog> {
    let env = crate::claude_accounts::sharing::account_config_env(
        &core.db,
        Default::default(),
        account_id,
    )?;
    if let Some(result) =
        crate::runtimes::adapter(runtime).toggle_skill_with_env(&home_dir()?, path, enabled, &env)
    {
        return result;
    }
    set_global_enabled_at(
        &home_dir()?,
        codex_home().as_deref(),
        runtime,
        path,
        enabled,
    )
}

pub fn read_skill(
    core: &AppCore,
    runtime: Runtime,
    path: &Path,
    account_id: Option<&str>,
) -> Result<String> {
    let env = core
        .runtime_shell_env
        .read()
        .map(|env| env.vars.clone())
        .unwrap_or_default();
    let env = crate::claude_accounts::sharing::account_config_env(&core.db, env, account_id)?;
    read_skill_at_with_env(&home_dir()?, codex_home().as_deref(), runtime, path, &env)
}

pub fn save_skill(
    core: &AppCore,
    runtime: Runtime,
    path: &Path,
    text: &str,
    account_id: Option<&str>,
) -> Result<SkillEntry> {
    let env = core
        .runtime_shell_env
        .read()
        .map(|env| env.vars.clone())
        .unwrap_or_default();
    let env = crate::claude_accounts::sharing::account_config_env(&core.db, env, account_id)?;
    save_skill_at_with_env(
        &home_dir()?,
        codex_home().as_deref(),
        runtime,
        path,
        text,
        &env,
    )
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

#[cfg(test)]
fn read_skill_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
    path: &Path,
) -> Result<String> {
    read_skill_at_with_env(home, codex_home, runtime, path, &Default::default())
}
fn read_skill_at_with_env(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
    path: &Path,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<String> {
    let catalog = crate::skills::skill_catalog_with_env(runtime, home, codex_home, env)
        .ok_or_else(|| Error::msg("runtime has no skills catalog"))?;
    let entry = find_entry(&catalog, path)?;
    if !entry.files.contains(&entry.marker) {
        return Err(Error::msg("skill has no marker file"));
    }
    Ok(std::fs::read_to_string(entry.path.join(entry.marker))?)
}

#[cfg(test)]
fn save_skill_at(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
    path: &Path,
    text: &str,
) -> Result<SkillEntry> {
    save_skill_at_with_env(home, codex_home, runtime, path, text, &Default::default())
}
fn save_skill_at_with_env(
    home: &Path,
    codex_home: Option<&Path>,
    runtime: Runtime,
    path: &Path,
    text: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<SkillEntry> {
    let catalog = || {
        crate::skills::skill_catalog_with_env(runtime, home, codex_home, env)
            .ok_or_else(|| Error::msg("runtime has no skills catalog"))
    };
    let entry = find_entry(&catalog()?, path)?;
    let marker = entry.path.join(&entry.marker);
    if !entry.files.contains(&entry.marker) || !marker.is_file() {
        return Err(Error::msg(format!(
            "no marker file at {}",
            marker.display()
        )));
    }
    std::fs::write(marker, text)?;
    catalog()?
        .entries
        .into_iter()
        .find(|updated| updated.path == entry.path)
        .ok_or_else(|| Error::msg("skill is no longer in the catalog after saving"))
}

#[cfg(test)]
mod tests;

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
