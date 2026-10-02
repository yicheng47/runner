use super::*;
use crate::runtimes::test_support::*;

#[test]
fn trae_fresh_returns_empty_plan() {
    let plan = resume_plan(Runtime::Trae.key(), None);
    assert!(plan.args.is_empty());
    assert!(plan.assigned_key.is_none());
    assert!(!plan.resuming);
}

#[test]
fn trae_resume_uses_subcommand_prefix() {
    let prior = "019fa1b9-a133-7841-b4dd-730d376ab1d1".to_string();
    let plan = resume_plan(Runtime::Trae.key(), Some(&prior));
    assert!(plan.resuming);
    assert!(plan.prepend, "trae resume is a subcommand, must prepend");
    assert_eq!(plan.args, vec!["resume", &prior]);
    assert_eq!(plan.assigned_key.as_deref(), Some(prior.as_str()));
}

#[test]
fn trae_emits_model_and_reasoning_effort_override() {
    let args = model_effort_args(Runtime::Trae.key(), Some("trae-model"), Some("High"));
    assert_eq!(
        args,
        vec![
            "--model".to_string(),
            "trae-model".to_string(),
            "-c".to_string(),
            "model_reasoning_effort=high".to_string(),
        ],
    );
}

#[test]
fn apply_permission_mode_trae_auto_strips_the_legacy_invalid_flag() {
    let user = vec![
        "--debug".to_string(),
        "--permission-mode".to_string(),
        "auto".to_string(),
    ];
    let auto = apply_permission_mode(Runtime::Trae.key(), &user, PermissionMode::Auto);
    assert_eq!(auto, vec!["--debug".to_string()]);
    assert_eq!(
        infer_permission_mode(Runtime::Trae.key(), &auto),
        PermissionMode::Default
    );
    assert_eq!(
        apply_permission_mode(Runtime::Trae.key(), &auto, PermissionMode::Default),
        auto,
    );
}
