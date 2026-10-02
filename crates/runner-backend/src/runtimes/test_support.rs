use super::*;
use std::path::Path;

fn selected(runtime: &str) -> &'static dyn RuntimeAdapter {
    for_key(runtime)
}
pub(crate) fn model_effort_args(
    runtime: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Vec<String> {
    selected(runtime).model_effort_args(model, effort)
}
pub(crate) fn permission_mode_args(runtime: &str, mode: PermissionMode) -> Vec<String> {
    selected(runtime).permissions().mode_args(mode)
}
pub(crate) fn strip_permission_flags(runtime: &str, args: &[String]) -> Vec<String> {
    selected(runtime).permissions().strip(args)
}
pub(crate) fn apply_permission_mode(
    runtime: &str,
    args: &[String],
    mode: PermissionMode,
) -> Vec<String> {
    selected(runtime).permissions().apply(args, mode)
}
pub(crate) fn mission_permission_mode_args(
    runtime: &str,
    mode: MissionPermissionMode,
) -> Option<Vec<String>> {
    selected(runtime).permissions().mission_args(mode)
}
pub(crate) fn apply_mission_permission_mode(
    runtime: &str,
    args: &[String],
    mode: MissionPermissionMode,
) -> Vec<String> {
    selected(runtime).permissions().apply_mission(args, mode)
}
pub(crate) fn infer_permission_mode(runtime: &str, args: &[String]) -> PermissionMode {
    selected(runtime).permissions().infer(args)
}
pub(crate) fn system_prompt_args(runtime: &str, system_prompt: Option<&str>) -> Vec<String> {
    selected(runtime)
        .prompt_channels()
        .system_prompt_args(system_prompt)
}
pub(crate) fn first_turn_argv(runtime: &str, body: Option<&str>) -> Vec<String> {
    selected(runtime).first_turn_argv(body)
}
pub(crate) fn mission_bus_sandbox_args(runtime: &str, mission_dir: Option<&Path>) -> Vec<String> {
    selected(runtime).mission_dir_args(mission_dir)
}
pub(crate) fn resume_plan(runtime: &str, prior_key: Option<&str>) -> ResumePlan {
    selected(runtime).resume_plan(prior_key)
}
pub(crate) fn fork_plan(runtime: &str, source_key: &str, source_label: &str) -> Option<ForkPlan> {
    selected(runtime).fork_plan(source_key, source_label)
}
pub(crate) fn supports_native_fork(runtime: &str) -> bool {
    selected(runtime)
        .catalog()
        .is_some_and(|catalog| catalog.native_fork)
}
pub(crate) fn runtime_definitions() -> Vec<RuntimeCatalog> {
    catalogs()
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn trailing_runtime_args(
    runtime: &str,
    role_args: &[String],
    app_data_dir: &Path,
    runner_session_id: &str,
    plan_resuming: bool,
    model: Option<&str>,
    effort: Option<&str>,
    codex_speed: Option<crate::model::CodexSpeed>,
    system_prompt: Option<&str>,
    first_turn: Option<&str>,
) -> Vec<String> {
    selected(runtime).launch_args(&LaunchContext {
        role_args,
        app_data_dir,
        session_id: runner_session_id,
        resuming: plan_resuming,
        mission: false,
        model,
        effort,
        codex_speed,
        system_prompt,
        first_turn,
    })
}
