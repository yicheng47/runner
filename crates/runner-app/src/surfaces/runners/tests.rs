use super::logic::resolve_slot_runtime_layers;
use super::logic::runtime_default_effort_label;
use super::logic::runtime_model_placeholder;
use super::logic::validate_runner_handle;
use super::*;
use runner_backend::model::Runtime;

#[test]
fn legacy_slot_pins_reach_validation_as_raw_names() {
    for name in ["qoder", "Runtime-Needle"] {
        let layers = resolve_slot_runtime_layers("codex", Some(name), None, None);
        assert!(layers.runtime_pinned);
        assert_eq!(layers.runtime, name);
        let raw_override = layers.runtime_pinned.then_some(layers.runtime.as_str());
        let error = runner_backend::ops::slot::validate_runtime_override(raw_override).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("unknown runtime '{name}' — valid runtimes: codex, claude-code, copilot, trae")
        );
    }
}

fn runtime_with_defaults(
    default_model: Option<&str>,
    default_effort: Option<&str>,
) -> RuntimeCatalogEntry {
    RuntimeCatalogEntry {
        name: Runtime::Codex,
        display_name: "Codex".into(),
        command: "codex".into(),
        native_fork: true,
        description: "OpenAI Codex CLI".into(),
        default_enabled: true,
        available: true,
        default_model: default_model.map(str::to_owned),
        default_effort: default_effort.map(str::to_owned),
        models: Vec::new(),
        efforts: Vec::new(),
    }
}

#[test]
fn runner_handle_validation_matches_the_shipped_contract() {
    for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
        assert_eq!(validate_runner_handle(valid), None, "{valid}");
    }
    for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
        assert!(validate_runner_handle(invalid).is_some(), "{invalid}");
    }
}

#[test]
fn runtime_default_labels_include_known_values() {
    let runtimes = [runtime_with_defaults(Some("gpt-5.6-sol"), Some("xhigh"))];
    assert_eq!(
        runtime_model_placeholder(&runtimes, "codex", None),
        "default (gpt-5.6-sol)"
    );
    assert_eq!(
        runtime_default_effort_label(&runtimes, "codex"),
        "Runtime default (xhigh)"
    );

    let runtimes = [runtime_with_defaults(None, None)];
    assert_eq!(
        runtime_model_placeholder(&runtimes, "codex", None),
        "default"
    );
    assert_eq!(
        runtime_default_effort_label(&runtimes, "codex"),
        "Runtime default"
    );
}

#[test]
fn slot_runtime_layers_leave_blank_overrides_to_inherit_runner_defaults() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", None, None, None),
        RuntimeLayerResolution {
            runtime: "codex".into(),
            runtime_pinned: false,
            model: None,
            effort: None,
        }
    );
}

#[test]
fn same_runtime_pin_keeps_blank_model_and_effort_overrides() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("codex"), None, None),
        RuntimeLayerResolution {
            runtime: "codex".into(),
            runtime_pinned: true,
            model: None,
            effort: None,
        }
    );
}

#[test]
fn different_runtime_uses_runtime_defaults_unless_overridden() {
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("claude-code"), None, None),
        RuntimeLayerResolution {
            runtime: "claude-code".into(),
            runtime_pinned: true,
            model: None,
            effort: None,
        }
    );
    assert_eq!(
        resolve_slot_runtime_layers("codex", Some("claude-code"), Some("opus"), Some("max"),),
        RuntimeLayerResolution {
            runtime: "claude-code".into(),
            runtime_pinned: true,
            model: Some("opus".into()),
            effort: Some("max".into()),
        }
    );
}

#[test]
fn trae_does_not_offer_a_mode_it_cannot_write() {
    use super::logic::{permission_mode_description, permission_modes};
    use runner_backend::router::runtime::PermissionMode;

    // TRAE CLI has no auto-approve middle ground, so Auto would write
    // nothing and read back as Default (#599).
    assert_eq!(
        permission_modes("trae"),
        &[PermissionMode::Default, PermissionMode::Bypass]
    );
    assert!(!permission_modes("trae").contains(&PermissionMode::Auto));
    assert!(permission_mode_description("trae", PermissionMode::Auto).is_empty());

    // Codex keeps its own Auto — it maps to a real flag pair.
    assert!(permission_modes("codex").contains(&PermissionMode::Auto));
    assert!(permission_modes("claude-code").contains(&PermissionMode::Auto));

    // Every offered mode describes itself.
    for runtime in ["claude-code", "codex", "trae", "copilot"] {
        for mode in permission_modes(runtime) {
            assert!(
                !permission_mode_description(runtime, *mode).is_empty(),
                "{runtime} {mode:?} has no description"
            );
        }
    }
}

#[test]
fn copilot_offers_only_the_three_supported_permission_modes_with_the_approved_copy() {
    use super::logic::{permission_mode_description, permission_modes};
    use runner_backend::router::runtime::PermissionMode;
    assert_eq!(
        permission_modes("copilot"),
        [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Bypass
        ]
    );
    assert!(permission_mode_description("copilot", PermissionMode::Auto).is_empty());
    assert_eq!(permission_mode_description("copilot", PermissionMode::Default), "Copilot's own manual mode: read-only tools run, writes and shell commands ask. Governed by defaultPermissionMode in ~/.copilot/settings.json.");
    assert_eq!(permission_mode_description("copilot", PermissionMode::AcceptEdits), "File creates and edits run without asking; shell commands, URLs and paths outside the cwd still prompt.");
    assert_eq!(permission_mode_description("copilot", PermissionMode::Bypass), "Every tool, path and URL is allowed. Same flag for the app-wide mission permission mode; chats never carry it (#596).");
}
