//! Claude Code adapter: a control-only stream-JSON session that answers one
//! `list_models` request. No prompt is sent, no conversation is resumed, and
//! customizations (hooks, MCP servers, plugins) stay out of the query while
//! authentication and model selection behave normally.

use std::time::Duration;

use serde::Deserialize;

use super::{option, trimmed, ModelCatalog, Query, Reason};
use crate::ops::runtime::RuntimeCatalogOption;
use crate::shell_path::LoginShellEnv;

/// Longer than Codex's: the CLI initializes a session before answering, and
/// model fields keep showing the cached catalog meanwhile.
const TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_ID: &str = "runner-list-models";
const ARGS: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--include-partial-messages",
    "--verbose",
    "--safe-mode",
    "--no-session-persistence",
];
const REQUEST: &str = concat!(
    r#"{"type":"control_request","request_id":"runner-list-models","#,
    r#""request":{"subtype":"list_models"}}"#,
    "\n"
);

pub(super) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = super::run(Query {
        executable,
        args: ARGS,
        stdin: Some(REQUEST.as_bytes()),
        env,
        timeout: TIMEOUT,
    })?;
    parse(&output)
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    response: Option<Control>,
}

#[derive(Deserialize)]
struct Control {
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    response: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    models: Vec<Model>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Model {
    value: String,
    #[serde(default)]
    resolved_model: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    supports_effort: Option<bool>,
    supported_effort_levels: Option<Vec<String>>,
}

fn parse(bytes: &[u8]) -> Result<ModelCatalog, Reason> {
    let Ok(stream) = std::str::from_utf8(bytes) else {
        return Err(Reason::InvalidOutput);
    };
    let mut payload = None;
    for line in stream.lines() {
        let Ok(envelope) = serde_json::from_str::<Envelope>(line) else {
            continue;
        };
        if envelope.kind != "control_response" {
            continue;
        }
        let Some(control) = envelope.response else {
            continue;
        };
        if control.request_id != REQUEST_ID {
            continue;
        }
        if control.subtype != "success" {
            return Err(Reason::QueryFailed);
        }
        payload = control.response;
    }
    let Some(payload) = payload else {
        return Err(Reason::InvalidOutput);
    };
    let has_effort_metadata = payload
        .models
        .iter()
        .any(|model| model.supports_effort.is_some() || model.supported_effort_levels.is_some());
    let mut models: Vec<RuntimeCatalogOption> = Vec::new();
    let mut default_model = None;
    for model in payload.models {
        let Some(value) = trimmed(&model.value) else {
            continue;
        };
        // Runner's own Default row already means "the agent's own default";
        // the CLI's marker entry supplies what that resolves to.
        if value == "default" {
            default_model = trimmed(&model.resolved_model);
            continue;
        }
        if models.iter().any(|known| known.value == value) {
            continue;
        }
        let label = trimmed(&model.display_name).unwrap_or_else(|| value.clone());
        let mut entry = option(value, label, trimmed(&model.description));
        // Current Claude omits effort fields for non-configurable models;
        // older responses omit them throughout, leaving capabilities unknown.
        entry.supported_efforts = if model.supports_effort == Some(false)
            || (has_effort_metadata
                && model.supports_effort.is_none()
                && model.supported_effort_levels.is_none())
        {
            Some(Vec::new())
        } else {
            model.supported_effort_levels
        };
        models.push(entry);
    }
    if models.is_empty() {
        return Err(Reason::EmptyCatalog);
    }
    Ok(ModelCatalog {
        models,
        default_model,
    })
}

/// Shapes captured from Claude Code 2.1.270 on 2026-09-14, with an unrelated
/// hook line and a stream event left in place.
#[cfg(test)]
pub(super) const STREAM: &str = concat!(
    r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart","exit_code":0}"#,
    "\n",
    r#"{"type":"stream_event","event":{"type":"ping"}}"#,
    "\n",
    r#"{"type":"control_response","response":{"subtype":"success","request_id":"other","response":{"models":[{"value":"stale"}]}}}"#,
    "\n",
    r#"{"type":"control_response","response":{"subtype":"success","request_id":"runner-list-models","response":{"models":[{"value":"default","resolvedModel":"claude-opus-5[1m]","displayName":"Default (recommended)","description":"Opus 5 with 1M context","supportsEffort":true,"supportedEffortLevels":["low","max"]},{"value":"opus[1m]","resolvedModel":"claude-opus-5[1m]","displayName":"Opus (1M context)","description":"Opus 5 with 1M context","supportsEffort":true,"supportedEffortLevels":["low","max"]},{"value":"sonnet","resolvedModel":"claude-sonnet-5","displayName":"Sonnet","description":"Efficient for routine tasks","supportsEffort":true,"supportedEffortLevels":["low"]},{"value":"haiku","resolvedModel":"claude-haiku-4-5","displayName":"Haiku","description":"Fastest for quick answers"}]}}}"#,
    "\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_aliases_provider_ids_and_the_cli_default_marker() {
        let catalog = parse(STREAM.as_bytes()).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| (model.value.as_str(), model.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("opus[1m]", "Opus (1M context)"),
                ("sonnet", "Sonnet"),
                ("haiku", "Haiku"),
            ]
        );
        assert_eq!(catalog.default_model.as_deref(), Some("claude-opus-5[1m]"));
        assert_eq!(
            catalog.models[0].description.as_deref(),
            Some("Opus 5 with 1M context")
        );
        assert_eq!(
            catalog.models[0].supported_efforts,
            Some(vec!["low".into(), "max".into()])
        );
        assert_eq!(catalog.models[2].supported_efforts, Some(Vec::new()));
    }

    #[test]
    fn missing_effort_metadata_remains_unknown_for_older_responses() {
        let old = br#"{"type":"control_response","response":{"subtype":"success","request_id":"runner-list-models","response":{"models":[{"value":"custom"}]}}}"#;
        assert_eq!(parse(old).unwrap().models[0].supported_efforts, None);
        let unsupported = br#"{"type":"control_response","response":{"subtype":"success","request_id":"runner-list-models","response":{"models":[{"value":"custom","supportsEffort":false}]}}}"#;
        assert_eq!(
            parse(unsupported).unwrap().models[0].supported_efforts,
            Some(Vec::new())
        );
    }

    #[test]
    fn protocol_and_output_failures_are_classified() {
        assert_eq!(parse(b""), Err(Reason::InvalidOutput));
        assert_eq!(parse(b"not JSON\n{}\n"), Err(Reason::InvalidOutput));
        assert_eq!(
            parse(
                br#"{"type":"control_response","response":{"subtype":"error","request_id":"runner-list-models","error":"unknown subtype"}}"#
            ),
            Err(Reason::QueryFailed)
        );
        assert_eq!(
            parse(
                br#"{"type":"control_response","response":{"subtype":"success","request_id":"runner-list-models","response":{"models":[]}}}"#
            ),
            Err(Reason::EmptyCatalog)
        );
    }
}
