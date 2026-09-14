//! Codex adapter: `codex debug models` prints the CLI's own catalog as JSON,
//! refreshing it from the provider only when its five-minute cache is cold.

use std::time::Duration;

use serde::Deserialize;

use super::{option, trimmed, ModelCatalog, Query, Reason};
use crate::ops::runtime::RuntimeCatalogOption;
use crate::shell_path::LoginShellEnv;

const TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = super::run(Query {
        executable,
        args: &["debug", "models"],
        stdin: None,
        env,
        timeout: TIMEOUT,
    })?;
    parse(&output)
}

#[derive(Deserialize)]
struct Catalog {
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    slug: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    visibility: String,
    supported_reasoning_levels: Option<Vec<ReasoningLevel>>,
}

#[derive(Deserialize)]
struct ReasoningLevel {
    effort: String,
}

fn parse(bytes: &[u8]) -> Result<ModelCatalog, Reason> {
    let Ok(catalog) = serde_json::from_slice::<Catalog>(bytes) else {
        return Err(Reason::InvalidOutput);
    };
    let mut models: Vec<RuntimeCatalogOption> = Vec::new();
    for model in catalog.models {
        if model.visibility != "list" {
            continue;
        }
        let Some(value) = trimmed(&model.slug) else {
            continue;
        };
        if models.iter().any(|known| known.value == value) {
            continue;
        }
        let label = trimmed(&model.display_name).unwrap_or_else(|| value.clone());
        let mut entry = option(value, label, trimmed(&model.description));
        entry.supported_efforts = model
            .supported_reasoning_levels
            .map(|levels| levels.into_iter().map(|level| level.effort).collect());
        models.push(entry);
    }
    if models.is_empty() {
        return Err(Reason::EmptyCatalog);
    }
    Ok(ModelCatalog {
        models,
        default_model: None,
    })
}

/// Shapes captured from `codex debug models` (Codex 0.154.0) on 2026-09-14,
/// trimmed to the fields Runner reads.
#[cfg(test)]
pub(super) const CATALOG: &str = r#"{"models":[
    {"slug":"gpt-6-astra","display_name":"GPT-6-Astra","description":"Capable coding model","visibility":"list",
     "supported_reasoning_levels":[{"effort":"low","description":"Fast"},{"effort":"ultra","description":"Deepest"}]},
    {"slug":"internal","visibility":"hide"},
    {"slug":"gpt-6-astra","visibility":"list"},
    {"slug":" future-model ","visibility":"list"},
    {"slug":" ","visibility":"list"}
]}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_visible_models_in_order_without_duplicates() {
        let catalog = parse(CATALOG.as_bytes()).unwrap();
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.models[0].value, "gpt-6-astra");
        assert_eq!(catalog.models[0].label, "GPT-6-Astra");
        assert_eq!(
            catalog.models[0].description.as_deref(),
            Some("Capable coding model")
        );
        assert_eq!(catalog.models[1].value, "future-model");
        assert_eq!(catalog.models[1].label, "future-model");
        assert_eq!(catalog.models[1].description, None);
        assert_eq!(
            catalog.models[0].supported_efforts,
            Some(vec!["low".into(), "ultra".into()])
        );
        assert_eq!(catalog.models[1].supported_efforts, None);
        assert_eq!(catalog.default_model, None);
    }

    #[test]
    fn unusable_output_is_classified() {
        for invalid in ["not JSON", "", "{}"] {
            assert_eq!(parse(invalid.as_bytes()), Err(Reason::InvalidOutput));
        }
        for empty in [
            r#"{"models":[]}"#,
            r#"{"models":[{"slug":"x","visibility":"hide"}]}"#,
        ] {
            assert_eq!(parse(empty.as_bytes()), Err(Reason::EmptyCatalog));
        }
    }
}
