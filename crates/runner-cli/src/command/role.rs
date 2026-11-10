use super::*;

pub(super) async fn run_role(
    client: &impl Caller,
    command: &RoleCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        RoleCommand::List => call(client, "role_list", json!({})).await,
        RoleCommand::Show { handle } => {
            let role = resolve_role(client, handle).await?;
            call(client, "role_get_by_handle", json!({"handle": role.name})).await
        }
        RoleCommand::Create { handle, fields } => {
            let runtime = fields
                .runtime
                .as_deref()
                .ok_or_else(|| CliError::usage("runner role create: --runtime is required"))?;
            let mut args = role_fields(fields, false)?;
            let object = args.as_object_mut().unwrap();
            object.insert("handle".into(), json!(handle));
            object.insert(
                "display_name".into(),
                json!(fields.name.as_deref().unwrap_or(handle)),
            );
            object.insert("runtime".into(), json!(runtime));
            object.insert("command".into(), json!(runtime_command(runtime)?));
            call(client, "role_create", args).await
        }
        RoleCommand::Update { handle, fields } => {
            let role = resolve_role(client, handle).await?;
            call(
                client,
                "role_update",
                json!({"id": role.id, "input": role_fields(fields, true)?}),
            )
            .await
        }
        RoleCommand::Delete { handle } => {
            let role = resolve_role(client, handle).await?;
            call(client, "role_delete", json!({"id": role.id})).await
        }
    }
}

pub(super) fn role_fields(fields: &RoleFields, update: bool) -> Result<Value, CliError> {
    let mut value = json!({});
    if update {
        insert_string(&mut value, "display_name", fields.name.as_deref());
        insert_string(&mut value, "runtime", fields.runtime.as_deref());
        if let Some(runtime) = fields.runtime.as_deref() {
            insert_string(&mut value, "command", Some(runtime_command(runtime)?));
        }
    }
    insert_string(&mut value, "model", fields.model.as_deref());
    insert_string(&mut value, "effort", fields.effort.as_deref());
    insert_speed(&mut value, "codex_speed", fields.speed);
    insert_string(&mut value, "permission_mode", fields.permission.as_deref());
    let prompt = read_text(
        fields.prompt.as_deref(),
        fields.prompt_file.as_deref(),
        "prompt",
    )?;
    if let Some(prompt) = prompt {
        insert_value(&mut value, "system_prompt", json!(prompt));
    }
    if !update {
        insert_value(&mut value, "args", json!([]));
    }
    let env = parse_env(&fields.env)?;
    if !env.is_empty() || !update {
        insert_value(&mut value, "env", json!(env));
    }
    insert_string(&mut value, "working_dir", fields.cwd.as_deref());
    Ok(value)
}
