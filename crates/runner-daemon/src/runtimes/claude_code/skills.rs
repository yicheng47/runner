use super::super::SkillState;
use crate::error::{Error, Result};
use crate::model::Runtime;
use crate::ops::skills::{catalog_at, find_entry};
use crate::skills::{GlobalState, SkillCatalog};
use std::path::Path;

pub(crate) fn load(home: &Path, _: Option<&Path>) -> SkillState {
    let overrides = std::fs::read_to_string(home.join(".claude/settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("skillOverrides").cloned());
    Box::new(
        move |entry| match overrides.as_ref().and_then(|value| value.get(&entry.name)) {
            None => GlobalState::On,
            Some(value) => match value.as_str() {
                Some("on") => GlobalState::On,
                Some("off") => GlobalState::Off,
                Some(other) => GlobalState::Other(other.into()),
                None => GlobalState::Other(value.to_string()),
            },
        },
    )
}

pub(crate) fn set_enabled(
    home: &Path,
    _codex_home: Option<&Path>,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    let runtime = Runtime::ClaudeCode;
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
