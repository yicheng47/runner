use super::*;

impl RemoteClient {
    pub(crate) fn sync_remote_state(&mut self, cx: &mut Context<NativeRoot>) -> bool {
        self.sync_computer_pair(cx);
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.workspace.as_mut() else {
            cx.notify();
            return false;
        };
        if let Some(computer) = clients
            .read(cx)
            .computers
            .iter()
            .find(|computer| computer.id == remote.computer.id)
        {
            remote.computer = computer.clone();
        }
        let pair_epoch = clients
            .read(cx)
            .pair_epochs
            .get(&remote.computer.id)
            .copied()
            .unwrap_or_default();
        if remote.pair_epoch != pair_epoch {
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.errors.clear();
            remote.pair_epoch = pair_epoch;
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let terminal_epoch = clients.read(cx).terminal_epoch;
        if remote.terminal_epoch != terminal_epoch {
            for (id, pane) in &remote.terminals {
                if let Some(size) = pane.attachment.terminal_interaction.read(cx).pane_size {
                    remote
                        .sizes
                        .insert(id.clone(), (size.cols.max(20), size.rows.max(8)));
                }
            }
            remote.recovery.extend(remote.terminals.keys().cloned());
            remote.recovery.extend(remote.pending.keys().cloned());
            remote.terminals.clear();
            remote.pending.clear();
            remote.errors.clear();
            remote.terminal_epoch = terminal_epoch;
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let connection = clients.read(cx).connection(&remote.computer.id);
        let connection = if !remote.computer.paired && connection == Connection::Connected {
            Connection::Revoked
        } else {
            connection
        };
        let trusted = clients.read(cx).computers.iter().any(|computer| {
            computer.id == remote.computer.id
                && computer.fingerprint == remote.computer.fingerprint
                && computer.paired
        }) && !matches!(connection, Connection::Revoked | Connection::Forgotten);
        if !trusted {
            remote._follow = None;
        } else if remote._follow.is_none() {
            remote._follow = Some(clients.read(cx).follow(remote.computer.id.clone()));
        }

        if connection == Connection::Forgotten && remote.connection != connection {
            remote.terminals.clear();
            remote.pending.clear();
            remote.recovery.clear();
            remote.errors.clear();
            remote.attachment_generation = remote.attachment_generation.wrapping_add(1);
        }
        let revision = clients
            .read(cx)
            .overview_revisions
            .get(&remote.computer.id)
            .copied()
            .unwrap_or_default();
        let pushed = revision != remote.overview_revision;
        remote.overview_revision = revision;
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            if pushed {
                remote.layout_hints.retain(|id, _| {
                    !remote.layouts.contains_key(id)
                        || overview.tabs.iter().any(|tab| &tab.id == id)
                });
            }
            remote
                .layouts
                .retain(|id, _| overview.tabs.iter().any(|tab| &tab.id == id));
            for tab in &overview.tabs {
                let layout = crate::surfaces::front_page::model::reconcile_tab(
                    tab,
                    remote.layouts.get(&tab.id),
                    remote.layout_hints.get(&tab.id),
                );
                if remote.layout_hints.get(&tab.id).is_some_and(|hint| {
                    crate::surfaces::front_page::model::matches_layout(hint, tab)
                        || (pushed
                            && remote.layouts.get(&tab.id).is_some_and(|old| {
                                !crate::surfaces::front_page::model::matches_layout(old, tab)
                            }))
                }) {
                    remote.layout_hints.remove(&tab.id);
                }
                remote.layouts.insert(tab.id.clone(), layout);
            }
            remote.closing_layouts.retain(|id, layout| {
                overview
                    .tabs
                    .iter()
                    .find(|tab| &tab.id == id)
                    .is_some_and(|tab| {
                        !crate::surfaces::front_page::model::same_topology(&layout.root, &tab.panes)
                    })
            });
        }
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            let live = remote
                .reopen_sessions
                .iter()
                .filter(|id| {
                    overview.sessions.get(*id).is_some_and(|session| {
                        !matches!(
                            session.status.status.effective(),
                            Status::Stopped | Status::Error
                        )
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            for id in live {
                remote.reopen_sessions.remove(&id);
                remote.terminals.remove(&id);
                remote.pending.remove(&id);
                remote.recovery.remove(&id);
            }
        }
        if let Some(session) = &remote.activate_session {
            let destination = if let Some((tab, pane)) = &remote.activate_placement {
                remote
                    .layouts
                    .get(tab)
                    .filter(|layout| match pane {
                        Some(pane) => layout.root.leaves().iter().any(|leaf| {
                            &leaf.id == pane && leaf.session_id.as_ref() == Some(session)
                        }),
                        None => layout.drawer_shells().contains(session),
                    })
                    .map(|_| tab.clone())
            } else {
                remote
                    .layouts
                    .iter()
                    .find(|(_, layout)| layout.all_session_ids().contains(session))
                    .map(|(id, _)| id.clone())
            };
            if let Some(id) = destination {
                let layout = remote.layouts.get_mut(&id).unwrap();
                if let Some((_, pane)) = &remote.activate_placement {
                    if let Some(pane) = pane {
                        layout.focus_pane(pane);
                    } else {
                        layout.activate_drawer_shell(session);
                        layout.set_drawer_open(true);
                    }
                }
                remote.active = Some(Item::Tab(id));
                remote.activate_session = None;
                remote.activate_placement = None;
            }
        }
        if let Some(tab) = &remote.activate_tab {
            if remote.layouts.contains_key(tab) {
                remote.active = Some(Item::Tab(tab.clone()));
                remote.activate_tab = None;
            }
        }
        if let Some(mission) = &remote.activate_mission {
            if clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .is_some_and(|overview| overview.missions.iter().any(|item| &item.id == mission))
            {
                remote.active = Some(Item::Mission(mission.clone()));
                remote.activate_mission = None;
            }
        }
        if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
            let valid = match &remote.active {
                Some(Item::Tab(id)) => remote.layouts.contains_key(id),
                Some(Item::Mission(id)) => {
                    overview.missions.iter().any(|mission| &mission.id == id)
                }
                Some(Item::Launcher | Item::Roles | Item::Crews) | None => true,
            };
            if !valid {
                remote.active = None;
            }
        }
        if remote.active.is_none() {
            if let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) {
                let viewed = remote
                    .viewed_tabs
                    .iter()
                    .rev()
                    .find(|tab| remote.layouts.contains_key(*tab))
                    .map(|tab| Item::Tab(tab.clone()));
                let left_a_tab = !remote.viewed_tabs.is_empty();
                remote.active = viewed
                    .or_else(|| overview.tabs.first().map(|tab| Item::Tab(tab.id.clone())))
                    .or_else(|| {
                        overview
                            .missions
                            .first()
                            .map(|mission| Item::Mission(mission.id.clone()))
                    });
                if left_a_tab && matches!(remote.active, Some(Item::Tab(_))) {
                    remote.landed_at = Some(std::time::Instant::now());
                }
            }
        }
        if let Some(Item::Tab(id)) = &remote.active {
            if remote.viewed_tabs.last() != Some(id) {
                remote
                    .viewed_tabs
                    .retain(|tab| tab != id && remote.layouts.contains_key(tab));
                remote.viewed_tabs.push(id.clone());
            }
        }
        remote
            .connected
            .store(connection == Connection::Connected, Ordering::Release);
        if connection != Connection::Connected {
            remote.catalog = None;
            remote.library_catalog = None;
            remote.library_target = None;
            remote.role_surfaces.invalidate_library();
            remote.crew_surfaces.invalidate_library();
            remote.catalog_target = None;
            remote.project_picker = None;
            remote.project_picker_subscription = None;
            remote.delete_project = None;
            *remote.shell_placement.lock().unwrap() = None;
            remote.actions_pending.clear();
            remote.closing_layouts.clear();
            remote.layout_hints.clear();
            remote.reopen_sessions.clear();
            remote.activate_session = None;
            remote.activate_placement = None;
            remote.activate_tab = None;
            remote.activate_mission = None;
            for pane in remote.terminals.values() {
                pane.attachment.terminal_input.update(cx, |input, cx| {
                    input.cancel_composition();
                    cx.notify();
                });
            }
        }
        if connection == Connection::Connected && remote.connection != connection {
            remote.errors.clear();
        }
        remote.connection = connection;
        let disconnected_computer =
            (!matches!(connection, Connection::Opening | Connection::Connected))
                .then(|| remote.computer.id.clone());
        let access = clients.update(cx, |state, cx| {
            state.front_page_access(&remote.computer, cx)
        });
        remote.front_page = crate::surfaces::front_page::FrontPageClient::remote(
            self.app_store.read(cx).client.clone(),
            access,
        );
        let sessions = clients
            .read(cx)
            .overviews
            .get(&remote.computer.id)
            .map(|overview| {
                overview
                    .sessions
                    .iter()
                    .map(|(id, session)| {
                        let native = remote.catalog.as_ref().is_some_and(|catalog| {
                            catalog.runtimes.iter().any(|runtime| {
                                Some(&runtime.name) == session.runtime.as_ref()
                                    && runtime.native_fork
                            })
                        });
                        let pinned = overview.tabs.iter().any(|tab| {
                            tab.pinned
                                && remote
                                    .layouts
                                    .get(&tab.id)
                                    .is_some_and(|layout| layout.contains_session(id))
                        });
                        (
                            id.clone(),
                            crate::surfaces::front_page::model::SessionDisplay::remote(
                                id,
                                session,
                                pinned,
                                native,
                                remote.front_page.supports(&["resume"]),
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        remote.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_front_page_target(remote.front_page.clone(), cx);
            sidebar.set_remote_presentation(
                remote.layouts.clone(),
                sessions,
                remote.active.clone(),
                cx,
            );
        });
        let workspace = remote.mission_workspace.clone();
        let mission = match &remote.active {
            Some(Item::Mission(id)) => Some(id.clone()),
            _ => None,
        };
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let client = remote.front_page.clone();
        workspace.update(cx, |workspace, cx| {
            workspace.sync_remote(client, mission, overview, clients, cx)
        });
        if let Some(computer) = disconnected_computer {
            self.clear_pending_archives_for(&computer, cx);
        }
        self.ensure_remote_library(cx);
        self.load_remote_metadata(cx);
        self.attach_remote_panes(cx);
        cx.notify();
        connection != Connection::Connected
    }

    pub(super) fn load_remote_metadata(&mut self, cx: &mut Context<NativeRoot>) {
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        if !remote.front_page.supports(&["catalog"])
            || remote.catalog_target.as_ref() == Some(&remote.front_page.target)
        {
            return;
        }
        remote.catalog = None;
        remote.catalog_target = Some(remote.front_page.target.clone());
        let client = remote.front_page.clone();
        let guard = client.clone();
        let epoch = self.ui.epoch;
        let work = cx.background_spawn(async move { client.catalog() });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |root, cx| {
                if root.remote_client.ui.epoch != epoch || !guard.valid() {
                    return;
                }
                if let Some(remote) = root
                    .remote_client
                    .workspace
                    .as_mut()
                    .filter(|remote| remote.front_page.target == guard.target)
                {
                    if let Ok(catalog) = result {
                        remote.catalog = Some(catalog);
                    }
                    root.sync_remote_state(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn select_remote_item(&mut self, item: RemoteItem) -> bool {
        let mut changed = false;
        if let Some(remote) = self.workspace.as_mut() {
            if remote.active.as_ref() != Some(&item) {
                changed = true;
                remote.form_generation = remote.form_generation.wrapping_add(1);
                remote.navigation_generation = remote.navigation_generation.wrapping_add(1);
                remote.activate_session = None;
                remote.activate_placement = None;
                remote.activate_tab = None;
                remote.activate_mission = None;
                for (id, hint) in &mut remote.layout_hints {
                    if let Some(previous) = remote.layouts.get(id) {
                        crate::surfaces::front_page::model::retain_presentation(hint, previous);
                    }
                }
                *remote.shell_placement.lock().unwrap() = None;
                remote.action_error = None;
            }
            remote.active = Some(item);
            remote.errors.clear();
        }
        changed
    }

    pub(crate) fn mark_remote_stopped_sessions_viewed(&mut self, cx: &mut Context<NativeRoot>) {
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.workspace.as_ref() else {
            return;
        };
        if remote.connection != Connection::Connected
            || !remote.front_page.supports(&["markViewed"])
        {
            return;
        }
        let Some(overview) = clients.read(cx).overviews.get(&remote.computer.id) else {
            return;
        };
        let sessions = remote
            .visible_sessions(overview, cx)
            .into_iter()
            .filter(|id| {
                overview.sessions.get(id).is_some_and(|session| {
                    let status = &session.status;
                    matches!(status.status, Status::Stopped | Status::Error)
                        && (status.unread
                            || (status.status == Status::Error && !status.error_acknowledged))
                })
            })
            .collect::<Vec<_>>();
        for session in sessions {
            self.remote_action(
                format!("mark-viewed:{session}"),
                &["markViewed"],
                move |client| {
                    client.mark_viewed(session)?;
                    Ok(Default::default())
                },
                cx,
            );
        }
    }

    pub(crate) fn attach_remote_panes(&mut self, cx: &mut Context<NativeRoot>) {
        let clients = state(&self.app_store, cx);
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        let overview = clients.read(cx).overviews.get(&remote.computer.id).cloned();
        let ids = overview
            .as_ref()
            .map(|overview| remote.visible_sessions(overview, cx))
            .unwrap_or_default();
        let running = overview
            .as_ref()
            .map(|overview| {
                overview
                    .sessions
                    .iter()
                    .filter(|(_, session)| {
                        !matches!(
                            session.status.status.effective(),
                            Status::Stopped | Status::Error
                        )
                    })
                    .map(|(id, _)| id)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let open = overview
            .as_ref()
            .map(|overview| remote.open_sessions(overview))
            .unwrap_or_default();
        let retained = if remote.connection == Connection::Connected {
            open.clone()
        } else {
            HashSet::new()
        };
        // A hidden chat dropped only because the route is down recovers
        // without taking control, as a visible one does.
        let dropped = remote
            .terminals
            .keys()
            .chain(remote.pending.keys())
            .filter(|id| !ids.contains(id) && !retained.contains(*id) && open.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        remote.recovery.extend(dropped);
        if overview.is_some() {
            remote
                .recovery
                .retain(|id| ids.contains(id) || open.contains(id));
        }
        remote
            .terminals
            .retain(|id, _| ids.contains(id) || retained.contains(id));
        remote
            .pending
            .retain(|id, _| (ids.contains(id) && running.contains(id)) || retained.contains(id));
        remote.errors.retain(|id| running.contains(id));
        if remote.connection != Connection::Connected {
            return;
        }
        for id in ids {
            if !running.contains(&id)
                || remote.terminals.contains_key(&id)
                || remote.pending.contains_key(&id)
                || remote.errors.contains(&id)
            {
                continue;
            }
            let Some(&(cols, rows)) = remote.sizes.get(&id) else {
                continue;
            };
            let request = Arc::new(());
            remote.pending.insert(id.clone(), request.clone());
            let config = RemoteTerminal {
                computer: remote.computer.id.clone(),
                session: id.clone(),
                cols,
                rows,
                take_control: !remote.recovery.contains(&id),
            };
            let address = config.address();
            let title = overview
                .as_ref()
                .and_then(|overview| overview.sessions.get(&id))
                .map(|session| session.title.clone())
                .unwrap_or_default();
            let (client, lifetime) = transport::RemoteTransport::client(
                self.app_store.read(cx).client.clone(),
                config,
                title,
                remote.connected.clone(),
            );
            let epoch = remote.epoch;
            let attachment_generation = remote.attachment_generation;
            let wake = remote.wake.clone();
            let wake_id = address.clone();
            let task = cx.background_spawn(async move {
                TerminalMirror::attach_remote(
                    client,
                    address,
                    Arc::new(move || {
                        let _ = wake.unbounded_send(wake_id.clone());
                    }),
                )
            });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |root, cx| {
                    let style = root.terminal_style(cx);
                    let settings = root.app_store.read(cx).settings.clone();
                    let Some(remote) = root.remote_client.workspace.as_mut().filter(|remote| {
                        remote.epoch == epoch
                            && remote.attachment_generation == attachment_generation
                    }) else {
                        return;
                    };
                    if !remote
                        .pending
                        .get(&id)
                        .is_some_and(|pending| Arc::ptr_eq(pending, &request))
                    {
                        return;
                    }
                    remote.pending.remove(&id);
                    let clients = state(&root.app_store, cx);
                    let wanted = clients
                        .read(cx)
                        .overviews
                        .get(&remote.computer.id)
                        .is_some_and(|overview| {
                            (remote.visible_sessions(overview, cx).contains(&id)
                                && overview.sessions.get(&id).is_some_and(|session| {
                                    !matches!(
                                        session.status.status.effective(),
                                        Status::Stopped | Status::Error
                                    )
                                }))
                                || (remote.connection == Connection::Connected
                                    && remote.open_sessions(overview).contains(&id))
                        });
                    if !wanted {
                        return;
                    }
                    match result {
                        Ok(terminal) => {
                            let input_address =
                                runner_core::remote::terminal_address(&remote.computer.id, &id);
                            let input_id = id.clone();
                            let input_terminal = terminal.clone();
                            let attachment = AttachedChat::new(
                                terminal,
                                &style,
                                &settings,
                                cx,
                                move |root, result, cx| {
                                    if let Err(error) = result {
                                        let current = root
                                            .remote_client
                                            .workspace
                                            .as_ref()
                                            .is_some_and(|remote| {
                                                remote.epoch == epoch
                                                    && remote.attachment_generation
                                                        == attachment_generation
                                                    && remote.terminals.get(&input_id).is_some_and(
                                                        |pane| {
                                                            Arc::ptr_eq(
                                                                &pane.attachment.terminal,
                                                                &input_terminal,
                                                            )
                                                        },
                                                    )
                                            });
                                        if current {
                                            root.remote_client.report_remote_input_error(
                                                &input_address,
                                                error,
                                                cx,
                                            );
                                        }
                                    }
                                },
                            );
                            let pane = RemotePane {
                                _lifetime: lifetime,
                                attachment,
                                control_failed: false,
                            };
                            remote.recovery.remove(&id);
                            remote.terminals.insert(id.clone(), pane);
                        }
                        Err(_) => {
                            remote.errors.insert(id.clone());
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
}
