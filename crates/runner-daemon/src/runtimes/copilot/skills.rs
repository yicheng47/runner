use super::super::SkillState;
use crate::error::{Error, Result};
use crate::model::Runtime;
use crate::ops::skills::{catalog_at, find_entry};
use crate::skills::{GlobalState, SkillCatalog};
use std::path::Path;
pub(crate) fn copilot_disabled_skills(home: &Path) -> Option<Vec<String>> {
    let text =
        std::fs::read_to_string(crate::runtime_defaults::copilot_settings_path(home)).ok()?;
    let settings = crate::runtime_defaults::jsonc_document(&text).ok()?;
    Some(
        settings
            .get("disabledSkills")?
            .as_array()?
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect(),
    )
}

pub(crate) fn load(home: &Path, _: Option<&Path>) -> SkillState {
    let disabled = copilot_disabled_skills(home);
    Box::new(move |entry| {
        if disabled
            .as_ref()
            .is_some_and(|disabled| disabled.iter().any(|name| name == &entry.name))
        {
            GlobalState::Off
        } else {
            GlobalState::On
        }
    })
}

pub(crate) fn set_enabled(
    home: &Path,
    _codex_home: Option<&Path>,
    skill_path: &Path,
    enabled: bool,
) -> Result<SkillCatalog> {
    let entry = find_entry(&catalog_at(home, None, Runtime::Copilot)?, skill_path)?;
    let path = crate::runtime_defaults::copilot_settings_path(home);
    let mut settings: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| Error::msg(format!("parse {}: {error}", path.display())))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    let object = settings
        .as_object_mut()
        .ok_or_else(|| Error::msg("settings.json is not a JSON object"))?;
    let disabled = object
        .entry("disabledSkills")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| Error::msg("disabledSkills is not a JSON array"))?;
    if enabled {
        disabled.retain(|value| value.as_str() != Some(entry.name.as_str()));
        if disabled.is_empty() {
            object.shift_remove("disabledSkills");
        }
    } else if !disabled
        .iter()
        .any(|value| value.as_str() == Some(entry.name.as_str()))
    {
        disabled.push(serde_json::json!(entry.name));
    }
    let text = format!("{}\n", serde_json::to_string_pretty(&settings)?);
    std::fs::create_dir_all(home.join(".copilot"))?;
    std::fs::write(path, text)?;
    catalog_at(home, None, Runtime::Copilot)
}
