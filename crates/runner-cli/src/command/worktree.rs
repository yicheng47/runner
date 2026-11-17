use super::*;

pub(super) async fn run_worktree(
    client: &impl Caller,
    command: &WorktreeCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        WorktreeCommand::Create {
            branch,
            project,
            cwd,
            base,
        } => {
            let dir = resolve_worktree_dir(client, project.as_deref(), cwd.as_deref()).await?;
            call(
                client,
                "worktree_create",
                json!({
                    "dir": dir,
                    "branch": branch,
                    "base": base,
                }),
            )
            .await
        }
        WorktreeCommand::List { project, cwd } => {
            let dir = resolve_worktree_dir(client, project.as_deref(), cwd.as_deref()).await?;
            call(client, "worktree_list", json!({ "dir": dir })).await
        }
        WorktreeCommand::Remove {
            target,
            project,
            cwd,
        } => {
            let (dir, resolved_target) =
                resolve_remove_target(client, project.as_deref(), cwd.as_deref(), target).await?;
            call(
                client,
                "worktree_remove",
                json!({
                    "dir": dir,
                    "target": resolved_target,
                }),
            )
            .await
        }
    }
}

async fn resolve_worktree_dir(
    client: &impl Caller,
    project: Option<&str>,
    cwd: Option<&Path>,
) -> Result<String, CliError> {
    if let Some(project) = project {
        let project = resolve_named(client, "project", "project_list", project).await?;
        let row = call(client, "project_get", json!({ "id": project.id })).await?;
        return row
            .value
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| CliError::usage(format!("project {} has no path", project.name)));
    }
    absolute_path(cwd.unwrap_or(Path::new(".")))
}

async fn resolve_remove_target(
    client: &impl Caller,
    project: Option<&str>,
    cwd: Option<&Path>,
    target: &str,
) -> Result<(String, String), CliError> {
    if project.is_some() || cwd.is_some() {
        let dir = resolve_worktree_dir(client, project, cwd).await?;
        return Ok((dir, target.to_owned()));
    }
    let target_path = Path::new(target);
    if target_path.is_absolute() {
        if let Some(dir) = target_path.ancestors().find(|p| p.is_dir()) {
            return Ok((absolute_path(dir)?, target.to_owned()));
        }
    }
    let dir = absolute_path(Path::new("."))?;
    Ok((dir, target.to_owned()))
}
