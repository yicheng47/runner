pub(crate) mod actions;
pub(crate) mod client;
pub(crate) mod folder_picker;
pub(crate) mod model;
#[cfg(test)]
mod tests;

pub(crate) use client::{FrontPageClient, FrontPageTarget, RemoteAccess};

impl crate::NativeRoot {
    pub(crate) fn front_page_client(&self, cx: &gpui::App) -> FrontPageClient {
        self.remote_window
            .as_ref()
            .map(|remote| remote.front_page.clone())
            .unwrap_or_else(|| FrontPageClient::local(self.core(cx).clone()))
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_layout(&self) -> Option<&runner_app::pane_layout::PaneLayout> {
        if let Some(remote) = &self.remote_window {
            if let Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) = &remote.active {
                return remote
                    .resize_draft
                    .as_ref()
                    .filter(|layout| &layout.id == id)
                    .or_else(|| remote.layouts.get(id));
            }
            None
        } else {
            self.tabs.active()
        }
    }

    pub(crate) fn front_page_layout_mut(
        &mut self,
    ) -> Option<&mut runner_app::pane_layout::PaneLayout> {
        if let Some(remote) = &mut self.remote_window {
            if let Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) = &remote.active {
                return remote.layouts.get_mut(id);
            }
            None
        } else {
            self.tabs.active_mut()
        }
    }

    pub(crate) fn front_page_session(
        &self,
        id: &str,
        cx: &gpui::App,
    ) -> Option<model::SessionDisplay> {
        if let Some(remote) = &self.remote_window {
            let state = self.app_store.read(cx).remote_clients.read(cx);
            let overview = state.overviews.get(&remote.computer.id)?;
            let session = overview.sessions.get(id)?;
            let pinned = overview.tabs.iter().any(|tab| {
                tab.pinned
                    && remote
                        .layouts
                        .get(&tab.id)
                        .is_some_and(|layout| layout.contains_session(id))
            });
            let native_fork = remote.catalog.as_ref().is_some_and(|catalog| {
                catalog.runtimes.iter().any(|runtime| {
                    Some(&runtime.name) == session.runtime.as_ref() && runtime.native_fork
                })
            });
            Some(model::SessionDisplay::remote(
                id,
                session,
                pinned,
                native_fork,
                remote.front_page.supports(&["resume"]),
            ))
        } else {
            self.session_entry(id, cx).map(model::SessionDisplay::local)
        }
    }

    pub(crate) fn front_page_status(
        &self,
        id: &str,
        cx: &gpui::App,
    ) -> Option<runner_app::ui::agent_status::StatusPresentation> {
        if let Some(remote) = &self.remote_window {
            let state = self.app_store.read(cx).remote_clients.read(cx);
            let session = state.overviews.get(&remote.computer.id)?.sessions.get(id)?;
            let status = super::remote_client::model::agent_status(&session.status);
            let mut display = runner_app::ui::agent_status::StatusPresentation::new(&status);
            display.waiting_since = None;
            display.estimated = false;
            Some(display)
        } else {
            self.session_entry(id, cx).map(|entry| {
                super::sidebar::direct_chat_display_status(
                    entry,
                    self.app_store.read(cx).session_statuses.get(id),
                )
            })
        }
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_sidebar(&self) -> gpui::Entity<crate::surfaces::Sidebar> {
        self.remote_window
            .as_ref()
            .map(|remote| remote.sidebar.clone())
            .unwrap_or_else(|| self.sidebar.clone())
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_is_chat(&self) -> bool {
        self.remote_window.as_ref().map_or(
            self.route == crate::surfaces::AppRoute::Chat,
            |remote| {
                !matches!(
                    remote.active,
                    Some(crate::surfaces::remote_client::RemoteItem::Mission(_))
                )
            },
        )
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_pane_bounds(
        &self,
    ) -> &std::collections::HashMap<crate::surfaces::PaneKey, gpui::Size<gpui::Pixels>> {
        self.remote_window
            .as_ref()
            .map(|remote| &remote.pane_bounds)
            .unwrap_or(&self.pane_bounds)
    }
    pub(crate) fn front_page_pane_bounds_mut(
        &mut self,
    ) -> &mut std::collections::HashMap<crate::surfaces::PaneKey, gpui::Size<gpui::Pixels>> {
        self.remote_window
            .as_mut()
            .map(|remote| &mut remote.pane_bounds)
            .unwrap_or(&mut self.pane_bounds)
    }
    pub(crate) fn front_page_resizing_split(&self) -> Option<&str> {
        self.remote_window
            .as_ref()
            .map(|remote| remote.resizing_split.as_deref())
            .unwrap_or_else(|| self.resizing_split.as_deref())
    }
    pub(crate) fn front_page_drawer_resizing(&self) -> bool {
        self.remote_window
            .as_ref()
            .map(|remote| remote.drawer_resizing)
            .unwrap_or(self.drawer_resizing)
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_sidebar_collapsed(&self) -> bool {
        self.remote_window
            .as_ref()
            .map(|remote| remote.sidebar_collapsed)
            .unwrap_or(self.sidebar_collapsed)
    }
    pub(crate) fn front_page_sidebar_width(&self, cx: &gpui::App) -> f32 {
        self.remote_window
            .as_ref()
            .map(|remote| remote.sidebar_width)
            .unwrap_or(self.settings(cx).sidebar_width)
    }
}
