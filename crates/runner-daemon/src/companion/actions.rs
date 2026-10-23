use super::core::{failed, DaemonCore, Failure, Result};
use crate::{ops, repo, AppCore};
use runner_core::{companion::*, protocol as p};
use serde_json::{json, Value};

fn bad_request() -> Failure {
    Failure(ErrorCode::BadRequest, "The action has invalid fields.")
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
    node.map(|node| node.id)
        .ok_or(Failure(ErrorCode::NotFound, "No sidebar item has that id."))
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
            Catalog { .. } => serde_json::to_value(catalog(core)?).map_err(failed),
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
                .map_err(failed)?;
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
                .map_err(failed)
            }
            Stop { session, .. } => ops::session::session_stop(core, &session)
                .map(|_| Value::Null)
                .map_err(failed),
            Close { session, .. } => ops::session::session_close(core, &session)
                .map(|_| Value::Null)
                .map_err(failed),
            Resume {
                session,
                cols,
                rows,
                ..
            } => ops::session::session_resume(core, &session, Some(cols), Some(rows))
                .map(session_result)
                .map_err(failed),
            Restart {
                session,
                cols,
                rows,
                ..
            } => ops::session::session_restart(core, &session, Some(cols), Some(rows))
                .map(session_result)
                .map_err(failed),
            Fork {
                session,
                title,
                cols,
                rows,
                ..
            } => ops::session::session_fork(core, &session, title, Some(cols), Some(rows))
                .map(session_result)
                .map_err(failed),
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
            Archive { target, .. } => {
                match target.kind {
                    ActionKind::Session => {
                        ops::session::session_archive_direct(core, &target.id).map_err(failed)?
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
                        let drawer = drawer_layout(core, Some(&target.id), shells)?;
                        let mut slots = vec![];
                        let tree = pane_layout(panes, &mut slots)?;
                        let layout = json!({"tree":tree,"slots":slots,"drawer":drawer}).to_string();
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
                    },
                    Some((cols, rows)),
                ))
                .map_err(failed)?;
                mission_changed(core, None);
                Ok(json!({"mission":output.mission.id}))
            }
            StopMission { mission, .. } => {
                crate::daemon::block_on(ops::mission::mission_stop_impl(core, mission))
                    .map_err(failed)?;
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
                        .map_err(failed)?;
                mission_changed(core, None);
                Ok(json!({"mission":output.mission_id,"sessions":output.resumed_session_ids}))
            }
        }
    }
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
        )
    })?;
    let entries = std::fs::read_dir(&path).map_err(|_| {
        Failure(
            ErrorCode::BadRequest,
            "Choose an accessible folder on the host.",
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
                    lead: slot.slot.lead,
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
