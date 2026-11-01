use super::*;

pub(super) fn mission_arg(command: &MissionCommand) -> Option<&Option<String>> {
    match command {
        MissionCommand::Show { mission }
        | MissionCommand::Stop { mission }
        | MissionCommand::Resume { mission }
        | MissionCommand::Archive { mission }
        | MissionCommand::Unarchive { mission }
        | MissionCommand::Pin { mission }
        | MissionCommand::Unpin { mission }
        | MissionCommand::Move { mission, .. }
        | MissionCommand::Feed { mission, .. } => Some(mission),
        _ => None,
    }
}

pub(super) async fn run_mission(
    client: &impl Caller,
    command: &MissionCommand,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    match command {
        MissionCommand::List { crew } => {
            let crew_id = match crew {
                Some(crew) => Some(resolve_named(client, "crew", "crew_list", crew).await?.id),
                None => None,
            };
            call(client, "mission_list_summary", json!({"crew_id": crew_id})).await
        }
        MissionCommand::Show { mission } => {
            let id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(client, "mission_status", json!({"id": id})).await
        }
        MissionCommand::Start {
            crew,
            goal,
            goal_file,
            title,
            project,
            cwd,
        } => {
            let crew = match crew {
                Some(crew) => Some(resolve_named(client, "crew", "crew_list", crew).await?),
                None => None,
            };
            let goal = read_text(goal.as_deref(), goal_file.as_deref(), "goal")?;
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
            call(
                client,
                "mission_start",
                json!({
                    "crew_id": crew.map(|crew| crew.id),
                    "project_id": project_id,
                    "title": title,
                    "goal_override": goal,
                    "cwd": cwd,
                }),
            )
            .await
        }
        MissionCommand::AddRole {
            mission,
            role,
            handle,
            runtime,
            model,
            effort,
            speed,
            task,
            task_file,
        } => {
            if matches!(context, BusContext::Mission(_)) {
                return Err(CliError::usage(
                    "runner mission add-role is for the person; ask the person to add a role",
                ));
            }
            let mission_id = resolve_mission(client, mission).await?;
            let role = resolve_role(client, role).await?;
            let task = read_text(task.as_deref(), task_file.as_deref(), "task")?;
            call(
                client,
                "mission_add_role",
                json!({"mission_id": mission_id, "role_id": role.id,
                "handle": handle, "runtime": runtime, "model": model, "effort": effort,
                "speed": speed.map(SpeedArg::value), "task": task}),
            )
            .await
        }
        MissionCommand::RemoveRole { mission, handle } => {
            if matches!(context, BusContext::Mission(_)) {
                return Err(CliError::usage(
                    "runner mission remove-role is for the person; ask the person to remove a role",
                ));
            }
            let mission_id = resolve_mission(client, mission).await?;
            call(
                client,
                "mission_remove_role",
                json!({"mission_id": mission_id, "handle": handle.trim_start_matches('@')}),
            )
            .await
        }
        MissionCommand::Stop { mission } => {
            mission_lifecycle(client, "mission_stop", mission.as_deref(), context).await
        }
        MissionCommand::Resume { mission } => {
            mission_lifecycle(client, "mission_resume", mission.as_deref(), context).await
        }
        MissionCommand::Archive { mission } => {
            mission_lifecycle(client, "mission_archive", mission.as_deref(), context).await
        }
        MissionCommand::Unarchive { mission } => {
            mission_lifecycle(client, "mission_unarchive", mission.as_deref(), context).await
        }
        MissionCommand::Rename { mission, title } => {
            let id = resolve_mission(client, mission).await?;
            call(client, "mission_rename", json!({"id": id, "title": title})).await
        }
        MissionCommand::Pin { mission } | MissionCommand::Unpin { mission } => {
            let id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(
                client,
                "mission_pin",
                json!({
                    "id": id,
                    "pinned": matches!(command, MissionCommand::Pin { .. }),
                }),
            )
            .await
        }
        MissionCommand::Move {
            mission,
            project,
            unfile: _,
        } => {
            let mission_id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            let project_id = match project {
                Some(project) => Some(
                    resolve_named(client, "project", "project_list", project)
                        .await?
                        .id,
                ),
                None => None,
            };
            call(
                client,
                "mission_set_project",
                json!({"mission_id": mission_id, "project_id": project_id}),
            )
            .await
        }
        MissionCommand::Feed {
            mission,
            follow: _,
            since,
            limit,
            oldest_first,
            types: _,
            from: _,
            all: _,
        } => {
            let mission_id = resolve_mission_arg(client, mission.as_deref(), context).await?;
            call(
                client,
                "mission_feed",
                json!({
                    "mission_id": mission_id,
                    "since_offset": since,
                    "limit": limit,
                    "order": if *oldest_first { "oldest_first" } else { "newest_first" },
                }),
            )
            .await
        }
        MissionCommand::Answer {
            mission,
            question_id,
            choice,
        } => {
            let mission_id = resolve_mission(client, mission).await?;
            call(
                client,
                "mission_signal",
                mission_signal_args(
                    &mission_id,
                    "human_response",
                    json!({"question_id": question_id, "choice": choice}),
                    None,
                ),
            )
            .await
        }
    }
}

pub(super) async fn mission_lifecycle(
    client: &impl Caller,
    tool: &str,
    mission: Option<&str>,
    context: &BusContext,
) -> Result<ToolResponse, CliError> {
    let id = resolve_mission_arg(client, mission, context).await?;
    let mut response = call(client, tool, json!({"id": id})).await?;
    if tool == "mission_resume" {
        let mission = call(client, "mission_get", json!({"id": id})).await?;
        response.value = mission.value;
    } else if tool == "mission_stop" {
        if let Some(mission) = mission_value_mut(&mut response.value).as_object_mut() {
            mission.insert("status".into(), Value::String("stopped".into()));
        }
    }
    let crew_id = mission_value(&response.value)
        .get("crew_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(crew_id) = crew_id {
        if let Some(mission) = mission_value_mut(&mut response.value).as_object_mut() {
            mission.insert("crew_name".into(), Value::Null);
        }
        if let Ok(crews) = list(client, "crew_list", json!({})).await {
            if let Some(name) = crews
                .iter()
                .find(|crew| field(crew, "id") == Some(&crew_id))
                .and_then(|crew| field(crew, "name"))
                .map(str::to_owned)
            {
                let mission = mission_value_mut(&mut response.value);
                if let Some(mission) = mission.as_object_mut() {
                    mission.insert("crew_name".into(), Value::String(name));
                }
            }
        }
    }
    Ok(response)
}

pub(super) fn mission_value(value: &Value) -> &Value {
    value.get("mission").unwrap_or(value)
}

pub(super) fn mission_value_mut(value: &mut Value) -> &mut Value {
    if value.get("mission").is_some() {
        value.get_mut("mission").expect("mission exists")
    } else {
        value
    }
}
