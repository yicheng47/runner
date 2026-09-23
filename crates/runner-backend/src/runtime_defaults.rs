use crate::model::Runtime;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuntimeDefaults {
    pub model: Option<String>,
    pub effort: Option<String>,
}

const CODEX_CONFIG_RELATIVE_PATH: &str = ".codex/config.toml";
const CLAUDE_SETTINGS_RELATIVE_PATH: &str = ".claude/settings.json";
const COPILOT_SETTINGS_RELATIVE_PATH: &str = ".copilot/settings.json";
const PI_SETTINGS_RELATIVE_PATH: &str = ".pi/agent/settings.json";
const TRAE_CONFIG_RELATIVE_PATH: &str = ".trae/traecli.toml";

pub fn runtime_defaults(runtime: Runtime, home: &Path) -> RuntimeDefaults {
    match runtime {
        Runtime::Codex => toml_defaults(&codex_config_path(home)),
        Runtime::ClaudeCode => json_defaults(&claude_settings_path(home), false),
        Runtime::Copilot => json_defaults(&copilot_settings_path(home), true),
        Runtime::Pi => pi_defaults(&home.join(PI_SETTINGS_RELATIVE_PATH)),
        Runtime::Trae => toml_defaults(&trae_config_path(home)),
        // agy's settings.json stores `/model`'s pick as a display label such as
        // "Gemini 3.8 Flash (High)", not an id `--model` accepts (spec 644).
        Runtime::Antigravity | Runtime::Shell => RuntimeDefaults::default(),
    }
}

pub(crate) fn copilot_settings_path(home: &Path) -> PathBuf {
    home.join(COPILOT_SETTINGS_RELATIVE_PATH)
}

pub(crate) fn codex_config_path(home: &Path) -> PathBuf {
    home.join(CODEX_CONFIG_RELATIVE_PATH)
}

fn claude_settings_path(home: &Path) -> PathBuf {
    home.join(CLAUDE_SETTINGS_RELATIVE_PATH)
}

pub(crate) fn trae_config_path(home: &Path) -> PathBuf {
    home.join(TRAE_CONFIG_RELATIVE_PATH)
}

fn toml_defaults(path: &Path) -> RuntimeDefaults {
    let Some(document) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.parse::<toml_edit::DocumentMut>().ok())
    else {
        return RuntimeDefaults::default();
    };
    let profile = document
        .get("profile")
        .and_then(|profile| profile.as_str())
        .and_then(|profile| {
            document
                .get("profiles")
                .and_then(|profiles| profiles.as_table_like())
                .and_then(|profiles| profiles.get(profile))
                .and_then(|profile| profile.as_table_like())
        });

    RuntimeDefaults {
        model: toml_string(profile, &document, "model"),
        effort: toml_string(profile, &document, "model_reasoning_effort"),
    }
}

fn toml_string(
    profile: Option<&dyn toml_edit::TableLike>,
    document: &toml_edit::DocumentMut,
    key: &str,
) -> Option<String> {
    profile
        .and_then(|profile| profile.get(key))
        .or_else(|| document.get(key))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_owned())
}

fn json_defaults(path: &Path, comments: bool) -> RuntimeDefaults {
    let Some(document) = std::fs::read_to_string(path).ok().and_then(|raw| {
        if comments {
            jsonc_document(&raw).ok()
        } else {
            serde_json::from_str(&raw).ok()
        }
    }) else {
        return RuntimeDefaults::default();
    };
    RuntimeDefaults {
        model: json_string(&document, "model"),
        effort: json_string(&document, "effortLevel"),
    }
}

fn pi_defaults(path: &Path) -> RuntimeDefaults {
    let Some(document) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    else {
        return RuntimeDefaults::default();
    };
    let model = json_string(&document, "defaultModel");
    let provider = json_string(&document, "defaultProvider");
    RuntimeDefaults {
        model: model.map(|model| match provider {
            Some(provider) if !provider.is_empty() => format!("{provider}/{model}"),
            _ => model,
        }),
        effort: json_string(&document, "defaultThinkingLevel"),
    }
}

pub(crate) fn jsonc_document(raw: &str) -> serde_json::Result<serde_json::Value> {
    let mut bytes = raw.as_bytes().to_vec();
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if in_string => index += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                    bytes[index] = b' ';
                    index += 1;
                }
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    serde_json::from_slice(&bytes)
}

fn json_string(document: &serde_json::Value, key: &str) -> Option<String> {
    document
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(home: &Path, relative: &str, contents: &str) {
        let path = home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn reads_codex_top_level_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            CODEX_CONFIG_RELATIVE_PATH,
            "model = \" gpt-5.6-sol \"\nmodel_reasoning_effort = \"xhigh\"\n",
        );
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults {
                model: Some("gpt-5.6-sol".into()),
                effort: Some("xhigh".into()),
            }
        );
    }

    #[test]
    fn codex_profile_values_take_precedence_per_key() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            CODEX_CONFIG_RELATIVE_PATH,
            "model = \"gpt-5.6-terra\"\nmodel_reasoning_effort = \"high\"\nprofile = \"work\"\n\n[profiles.work]\nmodel = \"gpt-5.6-sol\"\n",
        );
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults {
                model: Some("gpt-5.6-sol".into()),
                effort: Some("high".into()),
            }
        );

        write(
            home.path(),
            CODEX_CONFIG_RELATIVE_PATH,
            "model = \"gpt-5.6-terra\"\nmodel_reasoning_effort = \"high\"\nprofile = \"work\"\nprofiles = { work = { model = \"gpt-5.6-luna\" } }\n",
        );
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults {
                model: Some("gpt-5.6-luna".into()),
                effort: Some("high".into()),
            }
        );
    }

    #[test]
    fn reads_trae_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            TRAE_CONFIG_RELATIVE_PATH,
            "model = \"claude-fable-5[1m]\"\nmodel_reasoning_effort = \"max\"\n",
        );
        assert_eq!(
            runtime_defaults(Runtime::Trae, home.path()),
            RuntimeDefaults {
                model: Some("claude-fable-5[1m]".into()),
                effort: Some("max".into()),
            }
        );
    }

    #[test]
    fn reads_claude_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            CLAUDE_SETTINGS_RELATIVE_PATH,
            r#"{"model":"claude-fable-5[1m]","effortLevel":" xhigh "}"#,
        );
        assert_eq!(
            runtime_defaults(Runtime::ClaudeCode, home.path()),
            RuntimeDefaults {
                model: Some("claude-fable-5[1m]".into()),
                effort: Some("xhigh".into()),
            }
        );
    }

    #[test]
    fn reads_copilot_defaults_with_line_comments_and_literal_slashes() {
        let home = tempfile::tempdir().unwrap();
        for raw in [
            r#"{"model":"gpt-5.4","effortLevel":" high "}"#,
            "// Copilot settings\n{\n  \"model\": \"gpt-5.4\", // pinned\n  \"effortLevel\": \" high \",\n  \"url\": \"https://example.com\",\n  \"escaped\": \"a\\\"//b\"\n}\n",
        ] {
            write(home.path(), COPILOT_SETTINGS_RELATIVE_PATH, raw);
            assert_eq!(runtime_defaults(Runtime::Copilot, home.path()), RuntimeDefaults { model: Some("gpt-5.4".into()), effort: Some("high".into()) });
        }
    }

    #[test]
    fn reads_pi_provider_model_and_thinking_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            PI_SETTINGS_RELATIVE_PATH,
            r#"{"defaultProvider":"deepseek","defaultModel":"deepseek-v4-pro","defaultThinkingLevel":" high "}"#,
        );
        assert_eq!(
            runtime_defaults(Runtime::Pi, home.path()),
            RuntimeDefaults {
                model: Some("deepseek/deepseek-v4-pro".into()),
                effort: Some("high".into()),
            }
        );

        write(
            home.path(),
            PI_SETTINGS_RELATIVE_PATH,
            r#"{"defaultModel":"gpt-5.5"}"#,
        );
        assert_eq!(
            runtime_defaults(Runtime::Pi, home.path()),
            RuntimeDefaults {
                model: Some("gpt-5.5".into()),
                effort: None,
            }
        );
    }

    #[test]
    fn missing_file_returns_unknown_defaults() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults::default()
        );
    }

    #[test]
    fn malformed_toml_and_json_return_unknown_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), CODEX_CONFIG_RELATIVE_PATH, "model = [");
        write(home.path(), CLAUDE_SETTINGS_RELATIVE_PATH, "{");
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults::default()
        );
        assert_eq!(
            runtime_defaults(Runtime::ClaudeCode, home.path()),
            RuntimeDefaults::default()
        );
    }

    #[test]
    fn non_string_model_returns_unknown_model() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            CODEX_CONFIG_RELATIVE_PATH,
            "model = 5\nmodel_reasoning_effort = \"high\"\n",
        );
        assert_eq!(
            runtime_defaults(Runtime::Codex, home.path()),
            RuntimeDefaults {
                model: None,
                effort: Some("high".into()),
            }
        );
    }

    #[test]
    fn shell_runtime_has_no_agent_defaults() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            runtime_defaults(Runtime::Shell, home.path()),
            RuntimeDefaults::default()
        );
    }

    #[test]
    fn antigravity_display_label_is_not_read_as_a_model() {
        let home = tempfile::tempdir().unwrap();
        let settings = home.path().join(".gemini/antigravity-cli/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            "{\n  \"model\": \"Gemini 3.8 Flash (High)\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            runtime_defaults(Runtime::Antigravity, home.path()),
            RuntimeDefaults::default()
        );
    }
}
