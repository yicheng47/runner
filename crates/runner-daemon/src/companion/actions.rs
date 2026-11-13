use super::core::{failed, DaemonCore, Failure, Result};
use crate::{ops, repo, AppCore};
use runner_core::{companion::*, protocol as p};
use serde_json::{json, Value};

fn launch_failed(error: crate::error::Error) -> Failure {
    match error {
        crate::error::Error::RuntimeVersionGate(gate) => Failure(
            ErrorCode::Failed,
            "The agent is too old for Runner.",
            Some(gate),
        ),
        error => failed(error),
    }
}

fn bad_request() -> Failure {
    Failure(
        ErrorCode::BadRequest,
        "The action has invalid fields.",
        None,
    )
}
fn role(core: &AppCore, id: &str) -> Result<p::Role> {
    repo::role::get(&*core.db.get().map_err(failed)?, id)
        .map_err(failed)?
        .ok_or(Failure(ErrorCode::NotFound, "No role has that id.", None))
}
fn crew(core: &AppCore, id: &str) -> Result<p::Crew> {
    repo::crew::get(&*core.db.get().map_err(failed)?, id)
        .map_err(failed)?
        .ok_or(Failure(ErrorCode::NotFound, "No crew has that id.", None))
}
fn slot(core: &AppCore, id: &str) -> Result<p::Slot> {
    repo::slot::get(&*core.db.get().map_err(failed)?, id)
        .map_err(failed)?
        .ok_or(Failure(ErrorCode::NotFound, "No slot has that id.", None))
}
fn name(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(bad_request())
    } else {
        Ok(())
    }
}
fn handle(value: &str) -> Result<()> {
    if value.is_empty() || runner_core::slot_handle::validate_slot_handle(value).is_some() {
        Err(bad_request())
    } else {
        Ok(())
    }
}
fn agent(value: &str) -> Result<p::Runtime> {
    ops::role::validate_agent_runtime(value).map_err(|_| bad_request())
}
fn speed(value: Option<String>) -> Result<Option<p::CodexSpeed>> {
    value
        .map(|value| match value.as_str() {
            "standard" => Ok(p::CodexSpeed::Standard),
            "fast" => Ok(p::CodexSpeed::Fast),
            _ => Err(bad_request()),
        })
        .transpose()
}
fn permission(value: Option<RolePermissionMode>) -> Result<Option<p::permissions::PermissionMode>> {
    use p::permissions::PermissionMode as Mode;
    value
        .map(|value| match value {
            RolePermissionMode::Default => Ok(Mode::Default),
            RolePermissionMode::AcceptEdits => Ok(Mode::AcceptEdits),
            RolePermissionMode::Auto => Ok(Mode::Auto),
            RolePermissionMode::Bypass => Ok(Mode::Bypass),
            RolePermissionMode::Unrecognized(_) => Err(bad_request()),
        })
        .transpose()
}
fn wire_speed(speed: Option<p::CodexSpeed>) -> Option<String> {
    speed.map(|speed| {
        match speed {
            p::CodexSpeed::Standard => "standard",
            p::CodexSpeed::Fast => "fast",
        }
        .into()
    })
}
fn role_record(role: p::Role) -> RoleRecord {
    use p::permissions::PermissionMode as Mode;
    let permission_mode = p::role::role_permission_mode(&role).map(|mode| match mode {
        Mode::Default => RolePermissionMode::Default,
        Mode::AcceptEdits => RolePermissionMode::AcceptEdits,
        Mode::Auto => RolePermissionMode::Auto,
        Mode::Bypass => RolePermissionMode::Bypass,
    });
    RoleRecord {
        id: role.id,
        handle: role.handle,
        name: role.display_name,
        runtime: role.runtime,
        command: role.command,
        args: role.args,
        working_dir: role.working_dir,
        system_prompt: role.system_prompt,
        model: role.model,
        effort: role.effort,
        speed: wire_speed(role.codex_speed),
        permission_mode,
    }
}
fn available_slot_handle(core: &AppCore, crew: &str, id: Option<&str>, value: &str) -> Result<()> {
    handle(value)?;
    if repo::slot::list_for_crew(&*core.db.get().map_err(failed)?, crew)
        .map_err(failed)?
        .iter()
        .any(|slot| Some(slot.id.as_str()) != id && slot.slot_handle == value)
    {
        return Err(Failure(
            ErrorCode::Failed,
            "That slot handle is already in use. Choose another handle in the crew.",
            None,
        ));
    }
    Ok(())
}
fn role_delete_allowed(core: &AppCore, id: &str) -> Result<()> {
    let conn = core.db.get().map_err(failed)?;
    if !repo::role::crew_names(&conn, id)
        .map_err(failed)?
        .is_empty()
    {
        return Err(Failure(ErrorCode::Failed, "This role is used by crews. Remove its slots from the crews in the catalog before deleting it.", None));
    }
    if !repo::role::unarchived_direct_session_ids(&conn, id)
        .map_err(failed)?
        .is_empty()
    {
        return Err(Failure(
            ErrorCode::Failed,
            "This role has unarchived chats. Archive them on the host before deleting it.",
            None,
        ));
    }
    if !repo::role::unarchived_mission_names(&conn, id)
        .map_err(failed)?
        .is_empty()
    {
        return Err(Failure(ErrorCode::Failed, "This role is used by unarchived missions. Archive them on the host before deleting it.", None));
    }
    Ok(())
}

fn node_id(core: &AppCore, target: &ActionTarget) -> Result<String> {
    let conn = core.db.get().map_err(failed)?;
    let node = match target.kind {
        ActionKind::Tab => repo::node::get(&conn, &target.id)
            .map_err(failed)?
            .filter(|node| node.node_type == repo::node::NodeType::Tab),
        ActionKind::Mission => {
            repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &target.id)
                .map_err(failed)?
        }
        ActionKind::Project => {
            repo::node::find_by_ref(&conn, repo::node::NodeType::Project, &target.id)
                .map_err(failed)?
        }
        ActionKind::Session => return Err(bad_request()),
    };
    node.map(|node| node.id).ok_or(Failure(
        ErrorCode::NotFound,
        "No sidebar item has that id.",
        None,
    ))
}
fn project_node(core: &AppCore, project: Option<String>) -> Result<Option<String>> {
    project
        .map(|id| {
            node_id(
                core,
                &ActionTarget {
                    kind: ActionKind::Project,
                    id,
                },
            )
        })
        .transpose()
}
fn nodes(core: &AppCore, order: &[ActionTarget]) -> Result<Vec<String>> {
    order.iter().map(|target| node_id(core, target)).collect()
}
fn session_result(session: p::SpawnedSession) -> Value {
    json!({"session":session.id})
}
fn mission_changed(core: &AppCore, id: Option<&str>) {
    match id {
        Some(id) => core
            .events
            .emit("mission/changed", &json!({"mission_id":id})),
        None => core.events.emit("mission/changed", &()),
    }
}

fn pane_layout(pane: ActionPane, slots: &mut Vec<Option<String>>) -> Result<Value> {
    match pane {
        ActionPane::Leaf { id, session } => {
            slots.push(session.clone());
            Ok(json!({"kind":"leaf", "id":id, "session_id":session}))
        }
        ActionPane::Split {
            id,
            split,
            sizes,
            a,
            b,
        } => {
            if !matches!(split, SplitDirection::Row | SplitDirection::Column)
                || sizes.iter().any(|size| !size.is_finite() || *size <= 0.)
            {
                return Err(bad_request());
            }
            let scale = sizes[0].max(sizes[1]);
            let weights = [sizes[0] / scale, sizes[1] / scale];
            let total = weights[0] + weights[1];
            let sizes = [weights[0] / total * 100., weights[1] / total * 100.];
            Ok(
                json!({"kind":"split","id":id,"orientation":split,"sizes":sizes,
                "a":pane_layout(*a,slots)?,"b":pane_layout(*b,slots)?}),
            )
        }
    }
}
fn drawer_layout(core: &AppCore, node: Option<&str>, shells: Vec<String>) -> Result<Value> {
    let conn = core.db.get().map_err(failed)?;
    for session in &shells {
        if !repo::session::effective_runtime(&conn, session)
            .map_err(failed)?
            .as_deref()
            .and_then(p::Runtime::parse)
            .is_some_and(p::Runtime::is_shell)
        {
            return Err(bad_request());
        }
    }
    let layout = node
        .map(|id| repo::node::get(&conn, id))
        .transpose()
        .map_err(failed)?
        .flatten()
        .and_then(|node| node.layout);
    let mut drawer: Value = layout
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(failed)?
        .and_then(|mut layout| layout.get_mut("drawer").map(Value::take))
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let active = drawer["active"].as_u64().unwrap_or(0) as usize;
    let selected = drawer["shells"].get(active).and_then(Value::as_str);
    let next = selected
        .and_then(|id| shells.iter().position(|shell| shell == id))
        .unwrap_or_else(|| active.min(shells.len().saturating_sub(1)));
    drawer["shells"] = json!(shells);
    if next != 0 || drawer.get("active").is_some() {
        drawer["active"] = json!(next);
    }
    Ok(drawer)
}

fn usable_folder(path: &str) -> Result<()> {
    std::fs::read_dir(path).map(|_| ()).map_err(|_| {
        Failure(
            ErrorCode::BadRequest,
            "Choose an accessible folder on the host.",
            None,
        )
    })
}
fn start_cwd(
    core: &AppCore,
    project: Option<&str>,
    role: Option<&str>,
    cwd: Option<String>,
) -> Result<Option<String>> {
    let cwd = if project.is_none() && cwd.is_none() {
        let conn = core.db.get().map_err(failed)?;
        role.map(|id| {
            ops::role::get(&conn, id)
                .map_err(failed)
                .map(|role| role.working_dir)
        })
        .transpose()?
        .flatten()
    } else {
        cwd
    };
    let effective = if let Some(cwd) = cwd.as_deref() {
        cwd.to_owned()
    } else if let Some(project) = project {
        let conn = core.db.get().map_err(failed)?;
        ops::project::get(&conn, project).map_err(failed)?.cwd
    } else {
        runner_core::app_paths::home_dir()
            .ok_or_else(bad_request)?
            .to_string_lossy()
            .into_owned()
    };
    usable_folder(&effective)?;
    Ok(Some(effective))
}

impl DaemonCore {
    pub(super) fn action(&self, request: ActionRequest) -> Result<Value> {
        let core = self.core()?;
        let core = core.as_ref();
        use ActionRequest::*;
        match request {
            Role { role: id, .. } => {
                serde_json::to_value(role_record(role(core, &id)?)).map_err(failed)
            }
            Crew { crew: id, .. } => {
                let crew = crew(core, &id)?;
                let slots = ops::slot::slot_list(core, &id)
                    .map_err(failed)?
                    .into_iter()
                    .map(|item| {
                        let slot = item.slot;
                        SlotRecord {
                            id: slot.id,
                            handle: slot.slot_handle,
                            role: slot.role_id,
                            lead: false,
                            runtime: slot.runtime_override,
                            model: slot.model_override,
                            effort: slot.effort_override,
                            speed: wire_speed(slot.codex_speed_override),
                        }
                    })
                    .collect();
                serde_json::to_value(CrewRecord {
                    id: crew.id,
                    name: crew.name,
                    conventions: crew.system_prompt_addendum,
                    slots,
                })
                .map_err(failed)
            }
            CreateRole {
                handle: value,
                name: display_name,
                runtime,
                command,
                args,
                working_dir,
                system_prompt,
                model,
                effort,
                speed: value_speed,
                permission_mode,
                ..
            } => {
                handle(&value)?;
                name(&display_name)?;
                let runtime = agent(&runtime)?;
                let codex_speed = speed(value_speed)?;
                let permission_mode =
                    permission(permission_mode)?.unwrap_or(p::permissions::PermissionMode::Default);
                if repo::role::get_by_handle(&*core.db.get().map_err(failed)?, &value)
                    .map_err(failed)?
                    .is_some()
                {
                    return Err(Failure(
                        ErrorCode::Failed,
                        "That role handle is already in use. Choose another handle in the catalog.",
                        None,
                    ));
                }
                let role = ops::role::role_create(
                    core,
                    p::role::CreateRoleInput {
                        handle: value,
                        display_name,
                        runtime,
                        command: command.unwrap_or_else(|| {
                            runtime
                                .command()
                                .expect("agent runtime has a command")
                                .into()
                        }),
                        args,
                        working_dir,
                        system_prompt,
                        env: Default::default(),
                        model,
                        effort,
                        codex_speed,
                        permission_mode,
                    },
                )
                .map_err(failed)?;
                core.events.emit("role/changed", &());
                Ok(json!({"role":role.id}))
            }
            UpdateRole {
                role: id,
                name: display_name,
                runtime,
                command,
                args,
                working_dir,
                system_prompt,
                model,
                effort,
                speed: value_speed,
                permission_mode,
                ..
            } => {
                role(core, &id)?;
                name(&display_name)?;
                let runtime = agent(&runtime)?;
                let codex_speed = speed(value_speed)?;
                let permission_mode = permission(permission_mode)?;
                ops::role::role_update(
                    core,
                    &id,
                    p::role::UpdateRoleInput {
                        display_name: Some(display_name),
                        runtime: Some(runtime),
                        command: Some(command.unwrap_or_else(|| {
                            runtime
                                .command()
                                .expect("agent runtime has a command")
                                .into()
                        })),
                        args: Some(args),
                        working_dir: Some(working_dir),
                        system_prompt: Some(system_prompt),
                        env: None,
                        model: Some(model),
                        effort: Some(effort),
                        codex_speed: Some(codex_speed),
                        permission_mode,
                    },
                )
                .map_err(failed)?;
                core.events.emit("role/changed", &());
                Ok(Value::Null)
            }
            DeleteRole { role: id, .. } => {
                role(core, &id)?;
                role_delete_allowed(core, &id)?;
                ops::role::role_delete(core, &id).map_err(failed)?;
                core.events.emit("role/changed", &());
                Ok(Value::Null)
            }
            CreateCrew {
                name: value,
                conventions,
                ..
            } => {
                name(&value)?;
                let crew = ops::crew::crew_create(
                    core,
                    p::crew::CreateCrewInput {
                        name: value,
                        system_prompt_addendum: conventions,
                    },
                )
                .map_err(failed)?;
                core.events.emit("crew/changed", &());
                Ok(json!({"crew":crew.id}))
            }
            UpdateCrew {
                crew: id,
                name: value,
                conventions,
                ..
            } => {
                crew(core, &id)?;
                name(&value)?;
                ops::crew::crew_update(
                    core,
                    &id,
                    p::crew::UpdateCrewInput {
                        name: Some(value),
                        system_prompt_addendum: Some(conventions),
                    },
                )
                .map_err(failed)?;
                core.events.emit("crew/changed", &());
                Ok(Value::Null)
            }
            DeleteCrew { crew: id, .. } => {
                crew(core, &id)?;
                ops::crew::crew_delete(core, &id).map_err(failed)?;
                core.events.emit("crew/changed", &());
                core.events.emit("slot/changed", &());
                Ok(Value::Null)
            }
            AddSlot {
                crew: crew_id,
                role: role_id,
                handle,
                runtime,
                model,
                ..
            } => {
                crew(core, &crew_id)?;
                role(core, &role_id)?;
                available_slot_handle(core, &crew_id, None, &handle)?;
                let runtime_override = runtime.as_deref().map(agent).transpose()?;
                let slot = ops::slot::slot_create(
                    core,
                    p::slot::CreateSlotInput {
                        crew_id,
                        role_id,
                        slot_handle: handle,
                        runtime_override,
                        model_override: model,
                    },
                )
                .map_err(failed)?;
                core.events.emit("slot/changed", &());
                Ok(json!({"slot":slot.slot.id}))
            }
            UpdateSlot {
                slot: id,
                handle,
                runtime,
                model,
                effort,
                speed: value_speed,
                ..
            } => {
                let slot = slot(core, &id)?;
                available_slot_handle(core, &slot.crew_id, Some(&id), &handle)?;
                let runtime_override = runtime.as_deref().map(agent).transpose()?;
                let codex_speed_override = speed(value_speed)?;
                ops::slot::slot_update(
                    core,
                    &id,
                    p::slot::UpdateSlotInput {
                        slot_handle: Some(handle),
                        runtime_override: Some(runtime_override),
                        model_override: Some(model),
                        effort_override: Some(effort),
                        codex_speed_override: Some(codex_speed_override),
                    },
                )
                .map_err(failed)?;
                core.events.emit("slot/changed", &());
                Ok(Value::Null)
            }
            RemoveSlot { slot: id, .. } => {
                slot(core, &id)?;
                ops::slot::slot_delete(core, &id).map_err(failed)?;
                core.events.emit("slot/changed", &());
                Ok(Value::Null)
            }
            ReorderSlots {
                crew: id, order, ..
            } => {
                crew(core, &id)?;
                let current = repo::slot::ids_for_crew(&*core.db.get().map_err(failed)?, &id)
                    .map_err(failed)?;
                let mut expected = current;
                expected.sort();
                let mut actual = order.clone();
                actual.sort();
                if actual != expected {
                    return Err(bad_request());
                }
                ops::slot::slot_reorder(core, &id, order).map_err(failed)?;
                core.events.emit("slot/changed", &());
                Ok(Value::Null)
            }
            RuntimeTaskStart { runtime, .. } => {
                let runtime = agent(&runtime)?;
                serde_json::to_value(
                    ops::runtime_tasks::start(core, runtime).map_err(launch_failed)?,
                )
                .map_err(failed)
            }
            RuntimeTask { runtime, .. } => {
                let runtime = agent(&runtime)?;
                serde_json::to_value(
                    ops::runtime_tasks::list(core)
                        .into_iter()
                        .find(|task| task.runtime == runtime),
                )
                .map_err(failed)
            }
            Catalog { .. } => serde_json::to_value(catalog(core)?).map_err(failed),
            Usage { popover, .. } => {
                core.usage
                    .request_refresh(core.clone(), usage_refresh_reason(popover));
                serde_json::to_value(usage(core)?).map_err(failed)
            }
            Folders { path, .. } => serde_json::to_value(folders(path)?).map_err(failed),
            StartChat {
                role,
                runtime,
                model,
                effort,
                speed,
                project,
                cwd,
                cols,
                rows,
                ..
            } => {
                let cwd = start_cwd(core, project.as_deref(), role.as_deref(), cwd)?;
                let speed = speed
                    .map(|value| {
                        serde_json::from_value::<p::CodexSpeed>(Value::String(value))
                            .map_err(|_| bad_request())
                    })
                    .transpose()?;
                let scope = p::ProjectScope::or_root(project);
                let session = match (role, runtime) {
                    (Some(role), runtime) => ops::session::session_start_direct_with_speed(
                        core,
                        role,
                        runtime,
                        model,
                        effort,
                        speed,
                        scope,
                        cwd,
                        Some(cols),
                        Some(rows),
                    ),
                    (None, Some(runtime)) => ops::session::session_start_runtime_with_speed(
                        core,
                        &runtime,
                        scope,
                        cwd,
                        Some(cols),
                        Some(rows),
                        model,
                        effort,
                        speed,
                    )
                    .map(|out| out.session),
                    (None, None) => return Err(bad_request()),
                }
                .map_err(launch_failed)?;
                Ok(session_result(session))
            }
            StartShell {
                project,
                cwd,
                cols,
                rows,
                ..
            } => {
                let cwd = start_cwd(core, project.as_deref(), None, cwd)?;
                ops::session::session_start_shell_in(
                    core,
                    p::ProjectScope::or_root(project),
                    cwd,
                    Some(cols),
                    Some(rows),
                )
                .map(session_result)
                .map_err(launch_failed)
            }
            Stop { session, .. } => ops::session::session_stop(core, &session)
                .map(|_| Value::Null)
                .map_err(launch_failed),
            MarkViewed { session, .. } => {
                ops::node::mark_direct_sessions_viewed(core, &[session]).map_err(launch_failed)?;
                Ok(Value::Null)
            }
            Close { session, .. } => ops::session::session_close(core, &session)
                .map(|_| Value::Null)
                .map_err(launch_failed),
            Resume {
                session,
                cols,
                rows,
                ..
            } => ops::session::session_resume(core, &session, Some(cols), Some(rows))
                .map(session_result)
                .map_err(launch_failed),
            Restart {
                session,
                cols,
                rows,
                ..
            } => ops::session::session_restart(core, &session, Some(cols), Some(rows))
                .map(session_result)
                .map_err(launch_failed),
            Fork {
                session,
                title,
                cols,
                rows,
                ..
            } => ops::session::session_fork(core, &session, title, Some(cols), Some(rows))
                .map(session_result)
                .map_err(launch_failed),
            Rename { target, name, .. } => {
                match target.kind {
                    ActionKind::Session => {
                        ops::session::session_rename(core, &target.id, Some(name))
                            .map_err(failed)?
                    }
                    ActionKind::Tab => {
                        ops::node::node_rename(core, node_id(core, &target)?, name)
                            .map_err(failed)?;
                    }
                    ActionKind::Mission => {
                        let mission = crate::daemon::block_on(ops::mission::mission_rename_impl(
                            core, target.id, name,
                        ))
                        .map_err(failed)?;
                        mission_changed(core, Some(&mission.id));
                    }
                    ActionKind::Project => {
                        ops::project::project_rename(core, target.id, name).map_err(failed)?;
                    }
                }
                Ok(Value::Null)
            }
            Archive {
                target,
                with_missions,
                ..
            } => {
                match target.kind {
                    ActionKind::Session => {
                        let archived = ops::session::session_archive_direct_with_missions(
                            core,
                            &target.id,
                            with_missions,
                        )
                        .map_err(failed)?;
                        if with_missions {
                            return Ok(json!({"archivedMissions": archived}));
                        }
                    }
                    ActionKind::Tab => ops::node::node_tab_delete(core, &node_id(core, &target)?)
                        .map_err(failed)?,
                    ActionKind::Mission => {
                        let mission = crate::daemon::block_on(ops::mission::mission_archive_impl(
                            core, target.id,
                        ))
                        .map_err(failed)?;
                        mission_changed(core, Some(&mission.id));
                    }
                    ActionKind::Project => return Err(bad_request()),
                }
                Ok(Value::Null)
            }
            Layout {
                target,
                project,
                name,
                panes,
                shells,
                ..
            } => {
                match target.kind {
                    ActionKind::Tab => {
                        if !shells.is_empty() {
                            return Err(Failure(
                                ErrorCode::BadRequest,
                                "Chat tabs have no terminal drawer; add a shell pane instead.",
                                None,
                            ));
                        }
                        let mut slots = vec![];
                        let tree = pane_layout(panes, &mut slots)?;
                        let layout = json!({"tree":tree,"slots":slots}).to_string();
                        ops::node::node_tab_upsert(
                            core,
                            p::NodeTabUpsertInput {
                                id: target.id.clone(),
                                parent_id: project_node(core, project)?,
                                name,
                                layout,
                            },
                        )
                        .map_err(failed)?;
                    }
                    ActionKind::Mission => {
                        let node = node_id(core, &target)?;
                        let drawer = drawer_layout(core, Some(&node), shells)?;
                        ops::node::node_mission_layout_set(
                            core,
                            &node,
                            json!({"drawer":drawer}).to_string(),
                        )
                        .map_err(failed)?;
                    }
                    _ => return Err(bad_request()),
                }
                Ok(json!({"target":target}))
            }
            Pin { target, pinned, .. } => {
                match target.kind {
                    ActionKind::Tab => {
                        ops::node::node_set_pinned(core, node_id(core, &target)?, pinned)
                            .map_err(failed)?;
                    }
                    ActionKind::Mission => {
                        let mission = crate::daemon::block_on(ops::mission::mission_pin_impl(
                            core, target.id, pinned,
                        ))
                        .map_err(failed)?;
                        mission_changed(core, Some(&mission.id));
                    }
                    _ => return Err(bad_request()),
                }
                Ok(Value::Null)
            }
            Move {
                target,
                project,
                order,
                ..
            } => {
                ops::node::node_move(
                    core,
                    node_id(core, &target)?,
                    project_node(core, project)?,
                    nodes(core, &order)?,
                )
                .map_err(failed)?;
                Ok(Value::Null)
            }
            ReorderPinned { order, .. } => {
                ops::node::node_reorder_pinned(core, nodes(core, &order)?).map_err(failed)?;
                Ok(Value::Null)
            }
            CreateProject { name, path, .. } => {
                let row = ops::project::project_create_checked(core, name, path).map_err(failed)?;
                Ok(json!({"project":row.id}))
            }
            DeleteProject { project, force, .. } => {
                let result = crate::daemon::block_on(ops::project::project_delete_checked(
                    core, project, force,
                ))
                .map_err(|error| {
                    if error.to_string() == "project has running members" {
                        Failure(
                            ErrorCode::Failed,
                            "The project has running chats or missions.",
                            None,
                        )
                    } else {
                        failed(error)
                    }
                })?;
                Ok(
                    json!({"archivedMissions":result.archived_mission_ids,"archivedSessions":result.archived_session_ids}),
                )
            }
            StartMission {
                crew,
                project,
                title,
                goal,
                cwd,
                cols,
                rows,
                ..
            } => {
                let cwd = start_cwd(core, project.as_deref(), None, cwd)?;
                let output = crate::daemon::block_on(ops::mission::mission_start_impl_with_size(
                    core,
                    p::MissionStart {
                        crew_id: Some(crew),
                        scope: p::ProjectScope::or_root(project),
                        title,
                        goal_override: goal,
                        cwd,
                        started_by_session_id: None,
                        no_report: false,
                    },
                    Some((cols, rows)),
                ))
                .map_err(launch_failed)?;
                mission_changed(core, None);
                Ok(json!({"mission":output.mission.id}))
            }
            StopMission { mission, .. } => {
                crate::daemon::block_on(ops::mission::mission_stop_impl(core, mission))
                    .map_err(launch_failed)?;
                mission_changed(core, None);
                Ok(Value::Null)
            }
            ResumeMission {
                mission,
                cols,
                rows,
                ..
            } => {
                let output =
                    ops::mission::mission_resume_with_size(core, &mission, Some((cols, rows)))
                        .map_err(launch_failed)?;
                mission_changed(core, None);
                Ok(json!({"mission":output.mission_id,"sessions":output.resumed_session_ids}))
            }
            LinkMission {
                mission, session, ..
            } => {
                let mission = ops::mission::link::mission_link(
                    core,
                    p::MissionLinkInput {
                        mission_id: mission,
                        session_id: session,
                        no_report: false,
                        caller_session_id: None,
                    },
                )
                .map_err(failed)?;
                mission_changed(core, Some(&mission.id));
                Ok(Value::Null)
            }
            UnlinkMission { mission, .. } => {
                let mission = ops::mission::link::mission_unlink(core, &mission).map_err(failed)?;
                mission_changed(core, Some(&mission.id));
                Ok(Value::Null)
            }
        }
    }
}

fn usage_refresh_reason(popover: bool) -> p::usage::RefreshReason {
    if popover {
        p::usage::RefreshReason::Open
    } else {
        p::usage::RefreshReason::Schedule
    }
}

fn usage(core: &AppCore) -> Result<runner_core::companion::Usage> {
    let snapshot = core.usage.snapshot();
    let enabled = core.usage.enabled();
    let statuses = crate::runtime_status::status_list(
        &core.db,
        &core.runtime_shell_env,
        &core.runtime_discovery,
    )
    .map_err(failed)?;
    let agents: Vec<_> = statuses
        .runtimes
        .into_iter()
        .filter(|status| {
            enabled.contains(&status.name)
                && crate::runtimes::adapter(status.name).usage().is_some()
                && matches!(
                    status.effective_source,
                    Some(
                        crate::runtime_status::RuntimeCommandSource::Detected
                            | crate::runtime_status::RuntimeCommandSource::Override
                    )
                )
        })
        .map(|status| UsageAgent {
            name: status.name.key().into(),
            value: snapshot.get(status.name).cloned().map(Into::into),
            error: snapshot.error(status.name).map(Into::into),
        })
        .collect();
    let active_claude_account = agents
        .iter()
        .any(|agent| agent.name == "claude-code")
        .then(|| {
            let accounts = crate::claude_accounts::load(&core.db).ok()?;
            if accounts.accounts.len() <= 1 {
                return None;
            }
            accounts
                .accounts
                .into_iter()
                .find(|account| account.id == accounts.active_id)
                .map(|account| account.name)
        })
        .flatten();
    Ok(runner_core::companion::Usage {
        agents,
        refreshing: snapshot.refreshing,
        active_claude_account,
    })
}

fn folders(path: Option<String>) -> Result<FolderPage> {
    let path = path
        .map(std::path::PathBuf::from)
        .or_else(runner_core::app_paths::home_dir)
        .ok_or_else(bad_request)?;
    let path = path.canonicalize().map_err(|_| {
        Failure(
            ErrorCode::BadRequest,
            "Choose an accessible folder on the host.",
            None,
        )
    })?;
    let entries = std::fs::read_dir(&path).map_err(|_| {
        Failure(
            ErrorCode::BadRequest,
            "Choose an accessible folder on the host.",
            None,
        )
    })?;
    let mut folders = vec![];
    for entry in entries {
        let entry = entry.map_err(failed)?;
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                folders.push(name.to_owned());
            }
        }
    }
    folders.sort();
    Ok(FolderPage {
        path: path.to_string_lossy().into_owned(),
        parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
        folders,
    })
}
fn option(value: p::RuntimeCatalogOption) -> CatalogOption {
    CatalogOption {
        value: value.value,
        label: value.label,
        supported_efforts: value.supported_efforts,
    }
}
fn catalog(core: &AppCore) -> Result<Catalog> {
    let roles = ops::role::role_list(core)
        .map_err(failed)?
        .into_iter()
        .map(|role| CatalogRole {
            id: role.id,
            name: role.display_name,
            handle: role.handle,
            runtime: role.runtime,
            model: role.model,
            effort: role.effort,
            speed: role.codex_speed.map(|s| {
                match s {
                    p::CodexSpeed::Fast => "fast",
                    p::CodexSpeed::Standard => "standard",
                }
                .into()
            }),
        })
        .collect();
    let runtimes = ops::runtime::runtime_catalog(core)
        .map_err(failed)?
        .into_iter()
        .map(|runtime| CatalogRuntime {
            installed_version: runtime.installed_version,
            available_version: runtime.available_version,
            min_version: runtime.min_version,
            below_minimum: runtime.below_minimum,
            name: runtime.name.key().into(),
            label: runtime.display_name,
            available: runtime.available,
            native_fork: runtime.native_fork,
            default_model: runtime.default_model,
            default_effort: runtime.default_effort,
            models: runtime.models.into_iter().map(option).collect(),
            efforts: runtime.efforts.into_iter().map(option).collect(),
            speeds: if runtime.capabilities.codex_speed {
                vec!["standard".into(), "fast".into()]
            } else {
                vec![]
            },
        })
        .collect();
    let accounts = crate::claude_accounts::load(&core.db).map_err(failed)?;
    let crews = {
        let conn = core.db.get().map_err(failed)?;
        ops::crew::list(&conn).map_err(failed)?
    };
    let crews = crews
        .into_iter()
        .map(|item| {
            let slots = ops::slot::slot_list(core, &item.crew.id)
                .map_err(failed)?
                .into_iter()
                .map(|slot| CatalogSlot {
                    handle: slot.slot.slot_handle,
                    role: slot.slot.role_id,
                    lead: false,
                })
                .collect();
            Ok(CatalogCrew {
                id: item.crew.id,
                name: item.crew.name,
                slots,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let projects = ops::project::project_list(core)
        .map_err(failed)?
        .into_iter()
        .map(|p| CatalogProject {
            id: p.id,
            name: p.name,
            path: p.cwd,
        })
        .collect();
    let mut recent_folders = vec![];
    for session in ops::session::session_list_recent_direct(core).map_err(failed)? {
        if let Some(cwd) = session.cwd {
            if !recent_folders.contains(&cwd) {
                recent_folders.push(cwd);
            }
        }
    }
    Ok(Catalog {
        roles,
        runtimes,
        accounts: accounts
            .accounts
            .into_iter()
            .map(|a| CatalogName {
                id: a.id,
                name: a.name,
            })
            .collect(),
        active_account: accounts.active_id,
        crews,
        projects,
        recent_folders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_projects_only_enabled_installed_agents_and_the_active_account_name() {
        use crate::model::Runtime;
        use crate::usage::{AgentUsage, UsageWindow};
        let mut core = crate::test_support::test_core();
        core.usage = std::sync::Arc::new(crate::usage::UsageService::without_fetch());
        let core = std::sync::Arc::new(core);
        let executable = std::env::current_exe().unwrap();
        crate::db::set_runtime_override(&core.db, "codex", executable.to_str()).unwrap();
        crate::db::set_runtime_override(&core.db, "claude-code", executable.to_str()).unwrap();
        core.runtime_shell_env.write().unwrap().path = Some(String::new());
        core.usage.set_enabled(vec![Runtime::Codex, Runtime::Pi]);
        let value = AgentUsage {
            credits: Some(p::UsageCredits {
                balance: Some("1250.75".into()),
                has_credits: true,
                unlimited: false,
            }),
            windows: vec![UsageWindow {
                name: "Week".into(),
                used_percent: 41.,
                resets_at: None,
            }],
            updated_at: chrono::Utc::now(),
        };
        crate::db::set_usage_lkg(
            &core.db,
            &crate::db::UsageLkg {
                runtimes: [
                    (Runtime::Codex, value.clone()),
                    (Runtime::ClaudeCode, value.clone()),
                ]
                .into(),
                ..Default::default()
            },
        )
        .unwrap();
        core.usage.restore(&core.db);
        core.usage.set_polling_enabled(false);
        let daemon = DaemonCore(std::sync::Arc::downgrade(&core));
        let request = || ActionRequest::Usage {
            id: 1,
            popover: false,
        };
        let result: runner_core::companion::Usage =
            serde_json::from_value(daemon.action(request()).unwrap()).unwrap();
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].name, "codex");
        assert_eq!(result.agents[0].value, Some(value.into()));
        assert_eq!(result.active_claude_account, None);
        crate::db::app_state_set(&core.db.get().unwrap(), "claude_accounts", &json!({"accounts":[{"id":"native","name":"Personal","folder":"private-folder-canary","native":true},{"id":"work","name":"Work","folder":"private-folder-canary","native":false}],"main_id":"native","active_id":"work"}).to_string()).unwrap();
        assert_eq!(
            daemon.action(request()).unwrap()["activeClaudeAccount"],
            Value::Null
        );
        core.usage
            .set_enabled(vec![Runtime::Codex, Runtime::ClaudeCode]);
        let result = daemon.action(request()).unwrap();
        assert_eq!(result["activeClaudeAccount"], "Work");
        let encoded = result.to_string();
        for private in [
            "folder",
            "private-folder-canary",
            "command",
            "path",
            "main_id",
            "work",
        ] {
            assert!(!encoded.contains(private), "{private}");
        }
        crate::db::app_state_set(
            &core.db.get().unwrap(),
            "claude_accounts",
            "unreadable-json",
        )
        .unwrap();
        let result = daemon.action(request()).unwrap();
        assert_eq!(result["activeClaudeAccount"], Value::Null);
        assert_eq!(
            result["agents"][0]["value"]["windows"][0]["usedPercent"],
            41.
        );
        assert_eq!(
            usage_refresh_reason(false),
            p::usage::RefreshReason::Schedule
        );
        assert_eq!(usage_refresh_reason(true), p::usage::RefreshReason::Open);
    }

    #[cfg(unix)]
    #[test]
    fn remote_updates_join_one_daemon_task_and_only_its_unlisted_terminal_is_attachable() {
        use crate::companion::core::Core;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        std::fs::write(&executable, "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'codex-cli 0.166.0'; exit; fi\nread answer\necho done\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut core = crate::test_support::test_core_in(dir.path().into());
        core.sessions = crate::session::SessionManager::new(
            core.runtime_shell_env.clone(),
            core.runtime_discovery.clone(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        crate::db::set_runtime_override(&core.db, "codex", executable.to_str()).unwrap();
        let core = Arc::new(core);
        let daemon = Arc::new(DaemonCore(Arc::downgrade(&core)));
        let start = || ActionRequest::RuntimeTaskStart {
            id: 1,
            runtime: "codex".into(),
        };
        let first = daemon.action(start()).unwrap();
        let joined = daemon.action(start()).unwrap();
        assert_eq!(first["id"], joined["id"]);
        let id = first["id"].as_str().unwrap();
        let query = ActionRequest::RuntimeTask {
            id: 2,
            runtime: "codex".into(),
        };
        assert_eq!(daemon.action(query).unwrap()["id"], id);
        assert!(repo::session::get_row(&core.db.get().unwrap(), id)
            .unwrap()
            .is_none());
        assert!(daemon.clone().attach(id, 100, 1).is_ok());
        let events: Arc<dyn crate::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal("other-unlisted", (80, 24), &core.db, &events)
            .unwrap();
        assert!(daemon.clone().attach("other-unlisted", 100, 1).is_err());
        core.sessions.discard_unlisted_terminal("other-unlisted");
        core.sessions.desktop_input(id, b"done\r").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let task = ops::runtime_tasks::list(&core).pop().unwrap();
            if task.state == p::RuntimeUpdateState::Succeeded {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "update never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            daemon.clone().attach(id, 100, 2).is_ok(),
            "retained output after exit"
        );
        ops::runtime_tasks::dismiss(&core, id).unwrap();
        assert!(daemon.clone().attach(id, 100, 3).is_err());
        core.sessions.drain_terminal_workers();
    }

    #[test]
    fn chat_drawer_layout_requests_fail_without_mutation() {
        let core = std::sync::Arc::new(crate::test_support::test_core());
        let daemon = DaemonCore(std::sync::Arc::downgrade(&core));
        let request = serde_json::from_value(json!({"type":"layout","id":1,"target":{"kind":"tab","id":"missing"},"name":"","panes":{"id":"p1","session":null},"shells":["shell"]})).unwrap();
        let error = daemon.action(request).unwrap_err();
        assert_eq!(error.0, ErrorCode::BadRequest);
        assert!(error.1.contains("no terminal drawer"));
        assert!(repo::node::list(&core.db.get().unwrap())
            .unwrap()
            .is_empty());
    }
}
