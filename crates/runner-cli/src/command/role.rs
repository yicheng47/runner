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
        RoleCommand::Export { role, output } => {
            let role = resolve_role(client, role).await?;
            let exported = call(client, "template_export_role", json!({"id": role.id})).await?;
            if let Some(path) = output {
                let toml = field(&exported.value, "toml").ok_or_else(|| CliError {
                    code: 1,
                    message: "template_export_role returned no toml payload".into(),
                })?;
                let written = write_file_atomic(path, toml)?;
                response(json!({"path": written}))
            } else {
                Ok(exported)
            }
        }
        RoleCommand::Import {
            file,
            handle,
            runtime,
            dry_run,
        } => {
            let toml = read_template_file(file)?;
            let target_runtime = runtime.as_deref().map(parse_agent_runtime).transpose()?;
            let preview_resp = call(client, "template_preview", json!({"toml": toml})).await?;
            let preview: runner_core::protocol::TemplatePreview =
                serde_json::from_value(preview_resp.value).map_err(|error| CliError {
                    code: 1,
                    message: format!("failed to parse template preview: {error}"),
                })?;
            let plan = match preview {
                runner_core::protocol::TemplatePreview::Valid(plan) => *plan,
                runner_core::protocol::TemplatePreview::Invalid { error } => {
                    return Err(CliError {
                        code: 1,
                        message: error.to_string(),
                    });
                }
            };
            let runner_core::template::Template::Role(role_tmpl) = &plan.template else {
                return Err(CliError::usage(
                    "expected a role template, got a crew template; use `runner crew import`",
                ));
            };

            let mut runtime_overrides = BTreeMap::new();
            if let Some(rt) = target_runtime {
                runtime_overrides.insert(role_tmpl.role.handle.clone(), rt);
            }

            if *dry_run {
                let resolved =
                    runner_core::template::resolve_role_import(role_tmpl, &runtime_overrides)
                        .map_err(|err| CliError {
                            code: 1,
                            message: err.to_string(),
                        })?;
                let planned_handle = match handle.as_deref() {
                    Some(raw) => {
                        let trimmed = raw.trim().strip_prefix('@').unwrap_or(raw.trim());
                        if trimmed.is_empty() {
                            return Err(CliError {
                                code: 1,
                                message: "role handle must not be empty".to_owned(),
                            });
                        }
                        if let Some(msg) = runner_core::slot_handle::validate_slot_handle(trimmed) {
                            return Err(CliError {
                                code: 1,
                                message: msg.to_owned(),
                            });
                        }
                        let existing_roles = list(client, "role_list", json!({})).await?;
                        if existing_roles
                            .iter()
                            .any(|role| field(role, "handle") == Some(trimmed))
                        {
                            return Err(CliError {
                                code: 1,
                                message: format!("role handle @{trimmed} is already taken"),
                            });
                        }
                        trimmed.to_owned()
                    }
                    None => plan
                        .planned_roles
                        .first()
                        .map(|r| r.planned_handle.clone())
                        .unwrap_or_else(|| resolved.role.handle.clone()),
                };
                let installed: HashSet<runner_core::Runtime> =
                    plan.installed_runtimes.iter().copied().collect();
                let uninstalled = if installed.contains(&resolved.role.runtime) {
                    Vec::new()
                } else {
                    vec![resolved.role.runtime]
                };
                warn_uninstalled_runtimes(&uninstalled);
                return response(json!({
                    "dry_run": true,
                    "kind": "role",
                    "roles": [{
                        "template_handle": resolved.role.handle,
                        "handle": planned_handle,
                        "display_name": resolved.role.display_name,
                        "runtime": resolved.role.runtime,
                        "model": resolved.role.model,
                        "effort": resolved.role.effort,
                        "codex_speed": resolved.role.codex_speed,
                    }],
                    "uninstalled_runtimes": uninstalled,
                }));
            }

            let import_resp = call(
                client,
                "template_import",
                json!({
                    "toml": toml,
                    "role_handle": handle,
                    "runtime_overrides": runtime_overrides,
                }),
            )
            .await?;
            if let Ok(result) = serde_json::from_value::<runner_core::protocol::TemplateImportResult>(
                import_resp.value.clone(),
            ) {
                warn_uninstalled_runtimes(&result.uninstalled_runtimes);
            }
            Ok(import_resp)
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
