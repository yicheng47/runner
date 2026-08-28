use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuntimeDefaults {
    pub model: Option<String>,
    pub effort: Option<String>,
}

const CODEX_CONFIG_RELATIVE_PATH: &str = ".codex/config.toml";
const CLAUDE_SETTINGS_RELATIVE_PATH: &str = ".claude/settings.json";
const TRAE_CONFIG_RELATIVE_PATH: &str = ".trae/traecli.toml";

pub fn runtime_defaults(runtime: &str, home: &Path) -> RuntimeDefaults {
    match runtime {
        "codex" => toml_defaults(&codex_config_path(home)),
        "claude-code" => json_defaults(&claude_settings_path(home)),
        "trae" => toml_defaults(&trae_config_path(home)),
        _ => RuntimeDefaults::default(),
    }
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

fn json_defaults(path: &Path) -> RuntimeDefaults {
    let Some(document) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
    else {
        return RuntimeDefaults::default();
    };
    RuntimeDefaults {
        model: json_string(&document, "model"),
        effort: json_string(&document, "effortLevel"),
    }
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
            runtime_defaults("codex", home.path()),
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
            runtime_defaults("codex", home.path()),
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
            runtime_defaults("codex", home.path()),
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
            runtime_defaults("trae", home.path()),
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
            runtime_defaults("claude-code", home.path()),
            RuntimeDefaults {
                model: Some("claude-fable-5[1m]".into()),
                effort: Some("xhigh".into()),
            }
        );
    }

    #[test]
    fn missing_file_returns_unknown_defaults() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            runtime_defaults("codex", home.path()),
            RuntimeDefaults::default()
        );
    }

    #[test]
    fn malformed_toml_and_json_return_unknown_defaults() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), CODEX_CONFIG_RELATIVE_PATH, "model = [");
        write(home.path(), CLAUDE_SETTINGS_RELATIVE_PATH, "{");
        assert_eq!(
            runtime_defaults("codex", home.path()),
            RuntimeDefaults::default()
        );
        assert_eq!(
            runtime_defaults("claude-code", home.path()),
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
            runtime_defaults("codex", home.path()),
            RuntimeDefaults {
                model: None,
                effort: Some("high".into()),
            }
        );
    }

    #[test]
    fn unknown_runtime_returns_unknown_defaults() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            runtime_defaults("other", home.path()),
            RuntimeDefaults::default()
        );
    }
}
