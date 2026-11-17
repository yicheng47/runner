use super::*;

pub(super) async fn run_chat(
    client: &impl Caller,
    command: &ChatCommand,
) -> Result<ToolResponse, CliError> {
    let ChatCommand::Start {
        role,
        runtime,
        model,
        effort,
        speed,
        project,
        cwd,
        worktree,
        base,
    } = command;
    let role_id = match role {
        Some(role) => Some(resolve_role(client, role).await?.id),
        None => None,
    };
    let project_id = match project {
        Some(project) => Some(
            resolve_named(client, "project", "project_list", project)
                .await?
                .id,
        ),
        None => None,
    };
    let cwd = if project_id.is_none() {
        Some(absolute_path(cwd.as_deref().unwrap_or(Path::new(".")))?)
    } else {
        None
    };
    let mut args = json!({
        "role_id": role_id,
        "runtime": runtime,
        "model": model,
        "effort": effort,
        "project_id": project_id,
        "cwd": cwd,
    });
    if let Some(branch) = worktree {
        args["worktree"] = json!({
            "branch": branch,
            "base": base,
        });
    }
    insert_speed(&mut args, "speed", *speed);
    call(client, "session_start_direct", args).await
}
