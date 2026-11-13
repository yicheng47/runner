use std::time::Duration;

use runner_core::protocol::runtime_metadata::antigravity::split_leveled_model_with_label;

use crate::ops::runtime::RuntimeCatalogOption;
use crate::runtime_status::models::{option, ModelCatalog, Query, Reason};
use crate::shell_path::LoginShellEnv;

const TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = crate::runtime_status::models::run(Query {
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
        if let Some((base, base_label, level)) = split_leveled_model_with_label(id, label) {
            if let Some(existing) = models.iter_mut().find(|model| model.value == base) {
                let efforts = existing.supported_efforts.get_or_insert_with(Vec::new);
                if !efforts.iter().any(|existing_level| existing_level == level) {
                    efforts.push(level.to_owned());
                }
            } else {
                let mut model = option(base.to_owned(), base_label.to_owned(), None);
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

#[cfg(test)]
mod tests {
    use super::*;

    const CAPTURED: &str = "Fetching available models...\n\
gemini-4-carbon-low\tGemini 4 Carbon (Low)\n\
gemini-4-carbon-medium\tGemini 4 Carbon (Medium)\n\
gemini-4-carbon-high\tGemini 4 Carbon (High)\n\
gemini-4-argon-low\tGemini 4 Argon (Low)\n\
gemini-4-argon-medium\tGemini 4 Argon (Medium)\n\
gemini-4-argon-high\tGemini 4 Argon (High)\n\
gemini-4-hydrogen-low\tGemini 4 Hydrogen (Low)\n\
gemini-4-hydrogen-medium\tGemini 4 Hydrogen (Medium)\n\
gemini-4-hydrogen-high\tGemini 4 Hydrogen (High)\n\
gemini-4-hydrogen-xhigh\tGemini 4 Hydrogen (XHigh)\n\
gemini-pro-orchestrator\tOrchestrator (Carbon)\n\
gemini-pro-orchestrator-argon\tOrchestrator (Argon)\n\
gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n\
gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\n\
gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n\
claude-opus-5-5-low\tClaude Opus 5.5 (Low)\n\
claude-opus-5-5-medium\tClaude Opus 5.5 (Medium)\n\
claude-opus-5-5-high\tClaude Opus 5.5 (High)\n\
claude-sonnet-5-5-low\tClaude Sonnet 5.5 (Low)\n\
claude-sonnet-5-5-medium\tClaude Sonnet 5.5 (Medium)\n\
claude-sonnet-5-5-high\tClaude Sonnet 5.5 (High)\n\
gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n\
gemini-5-future-max\tGemini 5 Future (Max)\n\
non-leveled-max\tCustom Model Max\n";

    #[test]
    fn groups_effort_suffixed_models_dynamically_and_preserves_unsuffixed_ids() {
        let catalog = parse(CAPTURED.as_bytes()).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| (model.value.as_str(), model.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("gemini-4-carbon", "Gemini 4 Carbon"),
                ("gemini-4-argon", "Gemini 4 Argon"),
                ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
                ("gemini-pro-orchestrator", "Orchestrator (Carbon)"),
                ("gemini-pro-orchestrator-argon", "Orchestrator (Argon)"),
                ("gemini-3.8-flash", "Gemini 3.8 Flash"),
                ("claude-opus-5-5", "Claude Opus 5.5"),
                ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
                ("gpt-oss-120b", "GPT-OSS 120B"),
                ("gemini-5-future", "Gemini 5 Future"),
                ("non-leveled-max", "Custom Model Max"),
            ]
        );
        assert_eq!(
            catalog.models[2].supported_efforts.as_deref(),
            Some(["low".into(), "medium".into(), "high".into(), "xhigh".into()].as_slice())
        );
        assert_eq!(
            catalog.models[3].supported_efforts.as_deref(),
            Some([].as_slice())
        );
        assert_eq!(
            catalog.models[6].supported_efforts.as_deref(),
            Some(["low".into(), "medium".into(), "high".into()].as_slice())
        );
        assert_eq!(
            catalog.models[8].supported_efforts.as_deref(),
            Some(["medium".into()].as_slice())
        );
        assert_eq!(
            catalog.models[9].supported_efforts.as_deref(),
            Some(["max".into()].as_slice())
        );
        assert_eq!(
            catalog.models[10].supported_efforts.as_deref(),
            Some([].as_slice())
        );
        let adapter = crate::runtimes::adapter(crate::model::Runtime::Antigravity);
        assert_eq!(
            adapter.model_effort_args(Some(&catalog.models[2].value), Some("xhigh")),
            ["--model", "gemini-4-hydrogen", "--effort", "xhigh"]
        );
        assert_eq!(
            adapter.model_effort_args(Some(&catalog.models[9].value), Some("max")),
            ["--model", "gemini-5-future", "--effort", "max"]
        );
        assert_eq!(
            adapter.model_effort_args(Some(&catalog.models[3].value), Some("high")),
            ["--model", "gemini-pro-orchestrator"]
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
