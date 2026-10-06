use super::super::SkillState;
use crate::error::{Error, Result};
use crate::model::Runtime;
use crate::ops::skills::{catalog_at, find_entry};
use crate::skills::{GlobalState, SkillCatalog, SkillEntry};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub(crate) fn load(home: &Path, codex_home: Option<&Path>) -> SkillState {
    let config_dir = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".codex"));
    let overrides = std::fs::read_to_string(config_dir.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok());
    Box::new(move |entry| {
        overrides.as_ref().map_or(GlobalState::On, |document| {
            codex_global_state(document, entry)
        })
    })
}

pub(crate) fn manual(path: &Path, _: &crate::skills::SkillDocument) -> bool {
    std::fs::read_to_string(path.join("agents/openai.yaml")).is_ok_and(|text| codex_manual(&text))
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

pub(crate) fn codex_manual(text: &str) -> bool {
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

pub(crate) fn set_enabled(
    home: &Path,
    codex_home: Option<&Path>,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    let catalog = catalog_at(home, codex_home, Runtime::Codex)?;
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
    catalog_at(home, codex_home, Runtime::Codex)
}
