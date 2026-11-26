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

fn role_failed(error: crate::error::Error) -> Failure {
    use crate::error::RoleRejection;
    match error {
        crate::error::Error::RoleRejected { kind, .. } => {
            let code = match kind {
                RoleRejection::NotRunning => ErrorCode::NotRunning,
                RoleRejection::NotInMission
                | RoleRejection::AlreadyRemoved
                | RoleRejection::NoSession => ErrorCode::NotFound,
                RoleRejection::SpawnPending
                | RoleRejection::HandleInUse
                | RoleRejection::HandleRemoved
                | RoleRejection::FirstTurnTooLarge
                | RoleRejection::Resuming => ErrorCode::Failed,
            };
            Failure(code, kind.companion_message(), None)
        }
        crate::error::Error::RuntimeVersionGate(gate) => {
            launch_failed(crate::error::Error::RuntimeVersionGate(gate))
        }
        error => failed(error),
    }
}

fn mission_exists(core: &AppCore, id: &str) -> Result<()> {
    repo::mission::get(&*core.db.get().map_err(failed)?, id)
        .map_err(failed)?
        .ok_or(Failure(
            ErrorCode::NotFound,
            "No mission has that id.",
            None,
        ))
        .map(drop)
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
                if role.is_none() && runtime.is_none() {
                    return Err(bad_request());
                }
                let session = ops::session::session_start_direct(
                    core,
                    ops::session::DirectLaunch {
                        role_id: role,
                        runtime,
                        model,
                        effort,
                        speed,
                        scope: p::ProjectScope::or_root(project),
                        cwd,
                        cols: Some(cols),
                        rows: Some(rows),
                        ..Default::default()
                    },
                )
                .map(|out| out.session)
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
                    None,
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
            } => ops::session::session_resume(core, &session, Some(cols), Some(rows), false)
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
                        worktree: None,
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
            AddMissionRole {
                mission: mission_id,
                role: role_id,
                handle: slot_handle,
                runtime,
                model,
                effort,
                speed: value_speed,
                task,
                cols,
                rows,
                ..
            } => {
                let runtime = runtime.as_deref().map(agent).transpose()?;
                let speed = speed(value_speed)?;
                if let Some(value) = slot_handle.as_deref() {
                    handle(value)?;
                }
                mission_exists(core, &mission_id)?;
                role(core, &role_id)?;
                let size = (cols.clamp(20, 512), rows.clamp(8, 256));
                let output = ops::mission::roles::add_role_with_size(
                    core,
                    p::MissionAddRoleInput {
                        mission_id,
                        role_id,
                        handle: slot_handle,
                        runtime,
                        model,
                        effort,
                        speed,
                        task,
                    },
                    Some(size),
                )
                .map_err(role_failed)?;
                mission_changed(core, None);
                core.events.emit(
                    "session/updated",
                    &json!({"mission_id": output.mission_id, "session_id": output.session_id}),
                );
                Ok(json!({
                    "mission": output.mission_id,
                    "handle": output.handle,
                    "session": output.session_id,
                }))
            }
            RemoveMissionRole {
                mission: mission_id,
                handle: slot_handle,
                ..
            } => {
                if slot_handle.trim().is_empty() {
                    return Err(bad_request());
                }
                mission_exists(core, &mission_id)?;
                let output = ops::mission::roles::remove_role(core, &mission_id, &slot_handle)
                    .map_err(role_failed)?;
                mission_changed(core, None);
                core.events.emit(
                    "session/updated",
                    &json!({"mission_id": output.mission_id, "session_id": output.session_id}),
                );
                Ok(json!({
                    "mission": output.mission_id,
                    "handle": output.handle,
                    "session": output.session_id,
                }))
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
    let statuses = crate::ops::runtime::runtime_status_list(core).map_err(failed)?;
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
        crate::test_support::isolate_runtime_path(&core, None);
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
        crate::test_support::isolate_runtime_path(&core, None);
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

    struct MissionRoleFixture {
        core: std::sync::Arc<AppCore>,
        _dir: tempfile::TempDir,
    }

    impl MissionRoleFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let core = std::sync::Arc::new(crate::test_support::test_core_in(dir.path().into()));
            crate::test_support::isolate_runtime_path(&core, None);
            let executable = std::env::current_exe().unwrap();
            crate::db::set_runtime_override(&core.db, "codex", executable.to_str()).unwrap();
            crate::db::set_runtime_override(&core.db, "claude-code", executable.to_str()).unwrap();
            {
                let conn = core.db.get().unwrap();
                conn.execute_batch(
                    "INSERT INTO crews (id,name,created_at,updated_at) VALUES ('c','Team','2026-10-01T00:00:00Z','2026-10-01T00:00:00Z');
                     INSERT INTO missions (id,crew_id,title,status,started_at) VALUES ('m','c','secret-title-canary','running','2026-10-01T00:00:00Z');",
                )
                .unwrap();
                crate::test_support::insert_test_role(&conn, "r-coder", "coder", "codex", "codex");
                crate::test_support::insert_test_role(
                    &conn,
                    "r-reviewer",
                    "reviewer",
                    "codex",
                    "codex",
                );
                crate::test_support::insert_test_slot(
                    &conn, "s-coder", "c", "r-coder", "coder", 0, true,
                );
                let mut row = crate::test_support::test_session_row(
                    "session-coder",
                    crate::model::SessionStatus::Stopped,
                );
                row.mission_id = Some("m".into());
                row.slot_id = Some("s-coder".into());
                row.role_id = Some("r-coder".into());
                crate::test_support::insert_session_row(&conn, &row).unwrap();
                repo::node::upsert(
                    &conn,
                    &repo::node::NodeRow {
                        id: "node-m".into(),
                        parent_id: None,
                        position: 0,
                        node_type: repo::node::NodeType::Mission,
                        name: None,
                        ref_id: Some("m".into()),
                        layout: None,
                        pinned_position: None,
                        last_completed_at: None,
                        last_viewed_at: None,
                        created_at: chrono::Utc::now().to_rfc3339(),
                    },
                )
                .unwrap();
                ops::mission::roster::load(
                    &conn,
                    &core.app_data_dir,
                    &ops::mission::get(&conn, "m").unwrap(),
                )
                .unwrap();
            }
            crate::daemon::block_on(ops::mission::ensure_mission_router_mounted(&core, "m"))
                .unwrap();
            Self { core, _dir: dir }
        }

        fn daemon(&self) -> DaemonCore {
            DaemonCore(std::sync::Arc::downgrade(&self.core))
        }

        fn settle(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while self.core.sessions.has_pending_mission_spawns("m")
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!self.core.sessions.has_pending_mission_spawns("m"));
        }
    }

    impl Drop for MissionRoleFixture {
        fn drop(&mut self) {
            self.settle();
            self.core.buses.unmount("m");
            self.core.routers.unregister("m");
        }
    }

    #[test]
    fn add_and_remove_mission_role_happy_paths_emit_events_and_populate_overview_removed_handles() {
        use crate::companion::core::Core as _;

        assert!(crate::companion::FEATURES.contains(&"addMissionRole"));
        assert!(crate::companion::FEATURES.contains(&"removeMissionRole"));

        let fixture = MissionRoleFixture::new();
        let daemon = fixture.daemon();
        let mut events = fixture.core.events.subscribe();

        // 1. Add with defaults (suggested handle, no overrides, no task).
        let added = daemon
            .action(ActionRequest::AddMissionRole {
                id: 1,
                mission: "m".into(),
                role: "r-reviewer".into(),
                handle: None,
                runtime: None,
                model: None,
                effort: None,
                speed: None,
                task: None,
                cols: 140,
                rows: 42,
            })
            .unwrap();
        assert_eq!(added["mission"], "m");
        assert_eq!(added["handle"], "reviewer");
        let first_session = added["session"].as_str().unwrap().to_owned();
        fixture.settle();

        let mut saw_mission_changed = false;
        let mut saw_session_updated = false;
        while let Ok(event) = events.try_recv() {
            if event.name == "mission/changed" {
                saw_mission_changed = true;
            }
            if event.name == "session/updated"
                && event.payload["mission_id"] == "m"
                && event.payload["session_id"] == first_session
            {
                saw_session_updated = true;
            }
        }
        assert!(
            saw_mission_changed,
            "addMissionRole must emit mission/changed"
        );
        assert!(
            saw_session_updated,
            "addMissionRole must emit session/updated"
        );
        {
            let conn = fixture.core.db.get().unwrap();
            let row = repo::session::get_row(&conn, &first_session)
                .unwrap()
                .unwrap();
            assert_eq!(row.last_cols, Some(140));
            assert_eq!(row.last_rows, Some(42));
        }

        // 2. Add a claude-code role overridden to codex with explicit handle, model/effort/speed, task, and clamped size (20x256).
        {
            let conn = fixture.core.db.get().unwrap();
            crate::test_support::insert_test_role(
                &conn,
                "r-claude",
                "claude-reviewer",
                "claude-code",
                "claude",
            );
        }
        let custom = daemon
            .action(ActionRequest::AddMissionRole {
                id: 2,
                mission: "m".into(),
                role: "r-claude".into(),
                handle: Some("reviewer-custom".into()),
                runtime: Some("codex".into()),
                model: Some("gpt-5.4".into()),
                effort: Some("high".into()),
                speed: Some("fast".into()),
                task: Some("Audit this change".into()),
                cols: 5,
                rows: 999,
            })
            .unwrap();
        assert_eq!(custom["mission"], "m");
        assert_eq!(custom["handle"], "reviewer-custom");
        let custom_session = custom["session"].as_str().unwrap().to_owned();
        fixture.settle();
        {
            let conn = fixture.core.db.get().unwrap();
            let row = repo::session::get_row(&conn, &custom_session)
                .unwrap()
                .unwrap();
            assert_eq!(row.agent_runtime.as_deref(), Some("codex"));
            assert_eq!(row.last_cols, Some(20));
            assert_eq!(row.last_rows, Some(256));
            let def = row.launch_definition.unwrap();
            assert_eq!(def.model.as_deref(), Some("gpt-5.4"));
            assert_eq!(def.effort.as_deref(), Some("high"));
            assert_eq!(def.codex_speed, Some(p::CodexSpeed::Fast));
            let log_events =
                ops::mission::read_events(&fixture.core.app_data_dir, &conn, "m").unwrap();
            assert!(log_events.iter().any(|event| {
                event.from == "human"
                    && event.payload["text"] == "Audit this change"
                    && event.recipients().first().map(String::as_str) == Some("reviewer-custom")
            }));
        }

        // 3. Remove the custom role and verify events + Overview.removedHandles.
        while events.try_recv().is_ok() {}
        let removed = daemon
            .action(ActionRequest::RemoveMissionRole {
                id: 3,
                mission: "m".into(),
                handle: "reviewer-custom".into(),
            })
            .unwrap();
        assert_eq!(
            removed,
            json!({"mission": "m", "handle": "reviewer-custom", "session": custom_session})
        );
        let mut saw_remove_mission_changed = false;
        let mut saw_remove_session_updated = false;
        while let Ok(event) = events.try_recv() {
            if event.name == "mission/changed" {
                saw_remove_mission_changed = true;
            }
            if event.name == "session/updated"
                && event.payload["mission_id"] == "m"
                && event.payload["session_id"] == custom_session
            {
                saw_remove_session_updated = true;
            }
        }
        assert!(
            saw_remove_mission_changed,
            "removeMissionRole must emit mission/changed"
        );
        assert!(
            saw_remove_session_updated,
            "removeMissionRole must emit session/updated"
        );

        let overview = daemon.overview().unwrap();
        assert_eq!(overview.missions.len(), 1);
        assert_eq!(
            overview.missions[0].removed_handles,
            vec!["reviewer-custom"]
        );
        assert!(!overview.missions[0]
            .members
            .iter()
            .any(|member| member.handle == "reviewer-custom"));
    }

    #[test]
    fn add_and_remove_mission_role_map_every_rejection_and_preserve_privacy() {
        let fixture = MissionRoleFixture::new();
        let daemon = fixture.daemon();

        let add = |mission: &str,
                   role: &str,
                   handle: Option<&str>,
                   runtime: Option<&str>,
                   speed: Option<&str>| {
            ActionRequest::AddMissionRole {
                id: 1,
                mission: mission.into(),
                role: role.into(),
                handle: handle.map(str::to_owned),
                runtime: runtime.map(str::to_owned),
                model: None,
                effort: None,
                speed: speed.map(str::to_owned),
                task: None,
                cols: 120,
                rows: 40,
            }
        };
        let remove = |mission: &str, handle: &str| ActionRequest::RemoveMissionRole {
            id: 2,
            mission: mission.into(),
            handle: handle.into(),
        };

        // Pre-validation errors.
        for req in [
            add("m", "r-reviewer", None, Some("shell"), None),
            add("m", "r-reviewer", None, Some("unknown"), None),
            add("m", "r-reviewer", None, None, Some("turbo")),
            add("m", "r-reviewer", Some(""), None, None),
            add("m", "r-reviewer", Some("Bad Handle"), None, None),
            remove("m", ""),
            remove("m", "   "),
        ] {
            let err = daemon.action(req).unwrap_err();
            assert_eq!(err.0, ErrorCode::BadRequest);
        }

        // Missing mission and missing role.
        let err = daemon
            .action(add("missing-mission", "r-reviewer", None, None, None))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "No mission has that id.");

        let err = daemon
            .action(add("m", "missing-role", None, None, None))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "No role has that id.");

        let err = daemon
            .action(remove("missing-mission", "coder"))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "No mission has that id.");

        // HandleInUse.
        let err = daemon
            .action(add("m", "r-reviewer", Some("coder"), None, None))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert_eq!(err.1, "That handle is already in use in this mission.");

        // NotInMission.
        let err = daemon.action(remove("m", "ghost")).unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "That role is not in this mission.");

        // Add then remove a role to exercise HandleRemoved & AlreadyRemoved.
        daemon
            .action(add("m", "r-reviewer", Some("qa"), None, None))
            .unwrap();
        fixture.settle();
        daemon.action(remove("m", "qa")).unwrap();

        let err = daemon
            .action(add("m", "r-reviewer", Some("qa"), None, None))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert_eq!(
            err.1,
            "That handle was already removed from this mission; choose another handle."
        );

        let err = daemon.action(remove("m", "qa")).unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "That role was already removed from this mission.");

        // SpawnPending (both add and remove).
        let cancel = fixture.core.sessions.register_pending_mission_cancel("m");
        for req in [
            add("m", "r-reviewer", Some("qa-2"), None, None),
            remove("m", "coder"),
        ] {
            let err = daemon.action(req).unwrap_err();
            assert_eq!(err.0, ErrorCode::Failed);
            assert_eq!(
                err.1,
                "A session in this mission is still starting; try again once it is up."
            );
            assert!(!err.1.contains("secret-title-canary"));
        }
        fixture
            .core
            .sessions
            .drop_pending_mission_cancel("m", &cancel);

        // Resuming.
        let claim = fixture
            .core
            .sessions
            .try_claim_resume("session-coder")
            .unwrap();
        let err = daemon.action(remove("m", "coder")).unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert_eq!(
            err.1,
            "That role is being resumed or restarted; try again once it is up."
        );
        drop(claim);

        // NoSession (archive the session row while leaving the slot in roster).
        fixture
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE sessions SET archived_at='2026-10-02T00:00:00Z' WHERE id='session-coder'",
                [],
            )
            .unwrap();
        let err = daemon.action(remove("m", "coder")).unwrap_err();
        assert_eq!(err.0, ErrorCode::NotFound);
        assert_eq!(err.1, "That role has no session in this mission.");

        // FirstTurnTooLarge.
        let huge = "x".repeat(crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES + 1024);
        fixture
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE roles SET system_prompt=?1 WHERE id='r-reviewer'",
                rusqlite::params![huge],
            )
            .unwrap();
        let err = daemon
            .action(add("m", "r-reviewer", Some("qa-2"), None, None))
            .unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert_eq!(err.1, "The mission brief is too large to launch that role.");

        // NotRunning (both add and remove; never leaks mission title).
        fixture
            .core
            .db
            .get()
            .unwrap()
            .execute("UPDATE missions SET status='completed' WHERE id='m'", [])
            .unwrap();
        for req in [
            add("m", "r-reviewer", Some("qa-3"), None, None),
            remove("m", "coder"),
        ] {
            let err = daemon.action(req).unwrap_err();
            assert_eq!(err.0, ErrorCode::NotRunning);
            assert_eq!(err.1, "The mission is not running.");
            assert!(!err.1.contains("secret-title-canary"));
        }

        // RuntimeVersionGate & generic fallback through role_failed.
        let gate = p::RuntimeVersionGate {
            runtime: p::Runtime::Codex,
            installed_version: "0.140.0".into(),
            min_version: "0.164.0".into(),
        };
        let mapped = role_failed(crate::error::Error::RuntimeVersionGate(gate.clone()));
        assert_eq!(mapped.0, ErrorCode::Failed);
        assert_eq!(mapped.1, "The agent is too old for Runner.");
        assert_eq!(mapped.2, Some(gate));

        let generic = role_failed(crate::error::Error::msg("internal /secret/path failure"));
        assert_eq!(generic.0, ErrorCode::Failed);
        assert_eq!(generic.1, "The host could not complete the request.");
        assert!(!generic.1.contains("/secret/path"));
    }

    #[test]
    fn remove_mission_role_succeeds_and_notifies_when_leave_notice_append_fails() {
        use crate::companion::core::Core as _;

        let fixture = MissionRoleFixture::new();
        let daemon = fixture.daemon();
        let mut events = fixture.core.events.subscribe();

        ops::mission::roles::tests::inject_leave_notice_failure_for_test();
        let removed = daemon
            .action(ActionRequest::RemoveMissionRole {
                id: 9,
                mission: "m".into(),
                handle: "coder".into(),
            })
            .unwrap();
        assert_eq!(
            removed,
            json!({"mission": "m", "handle": "coder", "session": "session-coder"})
        );

        let mut saw_mission_changed = false;
        let mut saw_session_updated = false;
        while let Ok(event) = events.try_recv() {
            if event.name == "mission/changed" {
                saw_mission_changed = true;
            }
            if event.name == "session/updated"
                && event.payload["mission_id"] == "m"
                && event.payload["session_id"] == "session-coder"
            {
                saw_session_updated = true;
            }
        }
        assert!(saw_mission_changed, "must still emit mission/changed");
        assert!(saw_session_updated, "must still emit session/updated");

        let overview = daemon.overview().unwrap();
        assert!(overview.missions[0].members.is_empty());
        assert_eq!(overview.missions[0].removed_handles, vec!["coder"]);

        let conn = fixture.core.db.get().unwrap();
        let log_events = ops::mission::read_events(&fixture.core.app_data_dir, &conn, "m").unwrap();
        assert!(log_events.iter().any(|event| {
            event.from == "human"
                && event
                    .signal_type
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == "role_removed")
                && event.payload["handle"] == "coder"
        }));
        assert!(!log_events.iter().any(|event| {
            event.from == "runner"
                && event.payload["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("@coder left the mission."))
        }));
    }
}
