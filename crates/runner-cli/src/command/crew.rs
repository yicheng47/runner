use super::*;

pub(super) async fn run_crew(
    client: &impl Caller,
    command: &CrewCommand,
) -> Result<ToolResponse, CliError> {
    match command {
        CrewCommand::List => call(client, "crew_list", json!({})).await,
        CrewCommand::Show { crew } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let detail = call(client, "crew_get", json!({"id": crew.id})).await?;
            let slots = call(client, "slot_list", json!({"crew_id": crew.id})).await?;
            response(json!({"crew": detail.value, "slots": slots.value}))
        }
        CrewCommand::Create { name, fields } => {
            let mut args = crew_create_fields(fields)?;
            args.as_object_mut()
                .unwrap()
                .insert("name".into(), json!(name));
            call(client, "crew_create", args).await
        }
        CrewCommand::Update { crew, fields } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            call(
                client,
                "crew_update",
                json!({"id": crew.id, "input": crew_fields(fields)?}),
            )
            .await
        }
        CrewCommand::Delete { crew } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            call(client, "crew_delete", json!({"id": crew.id})).await
        }
        CrewCommand::Add {
            crew,
            role,
            handle,
            runtime,
            model,
            effort,
            speed,
        } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let role = resolve_role(client, role).await?;
            let mut args = json!({
                "crew_id": crew.id,
                "role_id": role.id,
                "slot_handle": handle.as_deref().unwrap_or(&role.name),
            });
            insert_clearable(&mut args, "runtime_override", runtime.as_deref(), false);
            insert_clearable(&mut args, "model_override", model.as_deref(), false);
            let created = call(client, "slot_create", args).await?;
            if effort.is_some() || speed.is_some() {
                let slot_id = id_from(&created.value)?;
                let mut input = json!({});
                insert_clearable(&mut input, "effort_override", effort.as_deref(), true);
                insert_speed(&mut input, "codex_speed_override", *speed);
                call(
                    client,
                    "slot_update",
                    json!({
                        "slot_id": slot_id,
                        "input": input,
                    }),
                )
                .await
            } else {
                Ok(created)
            }
        }
        CrewCommand::Set {
            crew,
            handle,
            new_handle,
            runtime,
            model,
            effort,
            speed,
        } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slot = resolve_slot(client, &crew.id, handle).await?;
            let mut input = json!({});
            insert_string(&mut input, "slot_handle", new_handle.as_deref());
            insert_clearable(&mut input, "runtime_override", runtime.as_deref(), true);
            insert_clearable(&mut input, "model_override", model.as_deref(), true);
            insert_clearable(&mut input, "effort_override", effort.as_deref(), true);
            insert_speed(&mut input, "codex_speed_override", *speed);
            call(
                client,
                "slot_update",
                json!({"slot_id": slot.id, "input": input}),
            )
            .await
        }
        CrewCommand::Remove { crew, handle } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slot = resolve_slot(client, &crew.id, handle).await?;
            call(client, "slot_delete", json!({"slot_id": slot.id})).await
        }
        CrewCommand::Order { crew, handles } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let slots = list(client, "slot_list", json!({"crew_id": crew.id})).await?;
            let mut ids = Vec::new();
            for handle in handles {
                let matches = slots
                    .iter()
                    .filter(|slot| field(slot, "slot_handle") == Some(handle.as_str()))
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(CliError::usage(format!(
                        "crew slot handle not found: {handle}"
                    )));
                }
                ids.push(field(matches[0], "id").unwrap().to_owned());
            }
            if ids.len() != slots.len() {
                return Err(CliError::usage(
                    "runner crew order must name every slot exactly once",
                ));
            }
            if ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len() {
                return Err(CliError::usage(
                    "runner crew order must name every slot exactly once",
                ));
            }
            call(
                client,
                "slot_reorder",
                json!({"crew_id": crew.id, "ordered_slot_ids": ids}),
            )
            .await
        }
        CrewCommand::Export { crew, output } => {
            let crew = resolve_named(client, "crew", "crew_list", crew).await?;
            let exported = call(client, "template_export_crew", json!({"id": crew.id})).await?;
            if let Some(path) = output {
                let toml = field(&exported.value, "toml").ok_or_else(|| CliError {
                    code: 1,
                    message: "template_export_crew returned no toml payload".into(),
                })?;
                let written = write_file_atomic(path, toml)?;
                response(json!({"path": written}))
            } else {
                Ok(exported)
            }
        }
        CrewCommand::Import {
            file,
            name,
            map_runtime,
            dry_run,
        } => {
            let toml = read_template_file(file)?;
            let mappings = parse_runtime_mappings(map_runtime)?;
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
            let runner_core::template::Template::Crew(crew_tmpl) = &plan.template else {
                return Err(CliError::usage(
                    "expected a crew template, got a role template; use `runner role import`",
                ));
            };

            let mut runtime_overrides = BTreeMap::new();
            for slot in &plan.planned_slots {
                if let Some(&target) = mappings.get(&slot.effective_runtime) {
                    runtime_overrides.insert(slot.slot_handle.clone(), target);
                }
            }

            if *dry_run {
                let resolved =
                    runner_core::template::resolve_crew_import(crew_tmpl, &runtime_overrides)
                        .map_err(|err| CliError {
                            code: 1,
                            message: err.to_string(),
                        })?;
                let installed: HashSet<runner_core::Runtime> =
                    plan.installed_runtimes.iter().copied().collect();
                let mut uninstalled = Vec::new();
                for slot in &resolved.slots {
                    if !installed.contains(&slot.effective_runtime)
                        && !uninstalled.contains(&slot.effective_runtime)
                    {
                        uninstalled.push(slot.effective_runtime);
                    }
                }
                warn_uninstalled_runtimes(&uninstalled);
                let planned_by_tmpl: BTreeMap<&str, &str> = plan
                    .planned_roles
                    .iter()
                    .map(|r| (r.template_handle.as_str(), r.planned_handle.as_str()))
                    .collect();
                let crew_name = match name.as_deref() {
                    Some(raw) => {
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            return Err(CliError::usage("crew name must not be empty"));
                        }
                        trimmed.to_owned()
                    }
                    None => plan
                        .suggested_crew_name
                        .unwrap_or_else(|| crew_tmpl.crew.name.clone()),
                };
                let roles_json: Vec<Value> = resolved
                    .roles
                    .iter()
                    .map(|r| {
                        let planned = planned_by_tmpl
                            .get(r.handle.as_str())
                            .copied()
                            .unwrap_or(r.handle.as_str());
                        json!({
                            "template_handle": r.handle,
                            "handle": planned,
                            "display_name": r.display_name,
                            "runtime": r.runtime,
                            "model": r.model,
                            "effort": r.effort,
                            "codex_speed": r.codex_speed,
                        })
                    })
                    .collect();
                let slots_json: Vec<Value> = resolved
                    .slots
                    .iter()
                    .map(|s| {
                        let planned = planned_by_tmpl
                            .get(s.template_role_handle.as_str())
                            .copied()
                            .unwrap_or(s.template_role_handle.as_str());
                        json!({
                            "slot_handle": s.slot_handle,
                            "role_handle": planned,
                            "runtime": s.effective_runtime,
                            "runtime_override": s.runtime_override,
                            "model_override": s.model_override,
                            "effort_override": s.effort_override,
                            "codex_speed_override": s.codex_speed_override,
                        })
                    })
                    .collect();
                return response(json!({
                    "dry_run": true,
                    "kind": "crew",
                    "crew_name": crew_name,
                    "roles": roles_json,
                    "slots": slots_json,
                    "uninstalled_runtimes": uninstalled,
                }));
            }

            let import_resp = call(
                client,
                "template_import",
                json!({
                    "toml": toml,
                    "crew_name": name,
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

pub(super) fn crew_fields(fields: &CrewFields) -> Result<Value, CliError> {
    let mut value = json!({});
    insert_string(&mut value, "name", fields.name.as_deref());
    if let Some(path) = fields.conventions_file.as_deref() {
        let text = read_file(path, "conventions")?;
        insert_value(&mut value, "system_prompt_addendum", json!(text));
    }
    Ok(value)
}

pub(super) fn crew_create_fields(fields: &CrewCreateFields) -> Result<Value, CliError> {
    let mut value = json!({});
    if let Some(path) = fields.conventions_file.as_deref() {
        insert_value(
            &mut value,
            "system_prompt_addendum",
            json!(read_file(path, "conventions")?),
        );
    }
    Ok(value)
}
