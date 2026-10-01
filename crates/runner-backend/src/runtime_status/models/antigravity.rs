use std::time::Duration;

use super::{option, ModelCatalog, Query, Reason};
use crate::ops::runtime::RuntimeCatalogOption;
use crate::runtimes::antigravity::ANTIGRAVITY_MODELS;
use crate::shell_path::LoginShellEnv;

const TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = super::run(Query {
        executable,
        args: &["models"],
        stdin: None,
        env,
        timeout: TIMEOUT,
    })?;
    parse(&output)
}

fn parse(bytes: &[u8]) -> Result<ModelCatalog, Reason> {
    let output = std::str::from_utf8(bytes).map_err(|_| Reason::InvalidOutput)?;
    let mut models: Vec<RuntimeCatalogOption> = Vec::new();
    for line in output.lines() {
        let Some((id, label)) = line.split_once('\t') else {
            continue;
        };
        let id = id.trim();
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            continue;
        }
        let label = label.trim();
        if label.is_empty() {
            continue;
        }
        let grouped = ["low", "medium", "high"].into_iter().find_map(|level| {
            let base = id.strip_suffix(&format!("-{level}"))?;
            ANTIGRAVITY_MODELS
                .iter()
                .any(|(model, levels)| *model == base && levels.contains(&level))
                .then_some((base, level))
        });
        if let Some((base, level)) = grouped {
            if let Some(existing) = models.iter_mut().find(|model| model.value == base) {
                existing
                    .supported_efforts
                    .as_mut()
                    .unwrap()
                    .push(level.to_owned());
            } else {
                let mut model = option(
                    base.to_owned(),
                    label
                        .trim_end_matches(&format!(" ({})", capitalize(level)))
                        .to_owned(),
                    None,
                );
                model.supported_efforts = Some(vec![level.to_owned()]);
                models.push(model);
            }
        } else if !models.iter().any(|model| model.value == id) {
            let mut model = option(id.to_owned(), label.to_owned(), None);
            model.supported_efforts = Some(Vec::new());
            models.push(model);
        }
    }
    if models.is_empty() {
        return Err(Reason::EmptyCatalog);
    }
    Ok(ModelCatalog {
        models,
        default_model: None,
    })
}

fn capitalize(level: &str) -> String {
    let mut chars = level.chars();
    chars.next().unwrap().to_ascii_uppercase().to_string() + chars.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPTURED: &str = "Fetching available models...\n\
gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n\
gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\n\
gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n\
gemini-3.1-pro-high\tGemini 3.1 Pro (High)\n\
gemini-3.1-pro-low\tGemini 3.1 Pro (Low)\n\
claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n\
gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n\
gemini-4-flash-high\tGemini 4 Flash (High)\n";

    #[test]
    fn groups_only_proven_aliases_and_keeps_other_ids() {
        let catalog = parse(CAPTURED.as_bytes()).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "gemini-3.8-flash",
                "gemini-3.1-pro",
                "claude-sonnet-4-6",
                "gpt-oss-120b-medium",
                "gemini-4-flash-high"
            ]
        );
        assert_eq!(
            catalog.models[0].supported_efforts.as_deref(),
            Some(["high".into(), "medium".into(), "low".into()].as_slice())
        );
        assert_eq!(
            catalog.models[1].supported_efforts.as_deref(),
            Some(["high".into(), "low".into()].as_slice())
        );
        assert_eq!(
            catalog.models[4].supported_efforts.as_deref(),
            Some([].as_slice())
        );
        assert_eq!(
            crate::runtimes::adapter(crate::model::Runtime::Antigravity).model_effort_args(
                Some(&catalog.models[0].value),
                catalog.models[0]
                    .supported_efforts
                    .as_ref()
                    .unwrap()
                    .first()
                    .map(String::as_str)
            ),
            ["--model", "gemini-3.8-flash", "--effort", "high"]
        );
        assert_eq!(
            crate::runtimes::adapter(crate::model::Runtime::Antigravity)
                .model_effort_args(Some(&catalog.models[4].value), None),
            ["--model", "gemini-4-flash-high"]
        );
    }

    #[test]
    fn rejects_no_models() {
        assert_eq!(
            parse(b"Fetching available models...\n"),
            Err(Reason::EmptyCatalog)
        );
    }
}
