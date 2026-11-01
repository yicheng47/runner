pub(crate) mod actions;
pub(crate) mod client;
pub(crate) mod folder_picker;
mod library;
mod local;
pub(crate) mod model;
#[cfg(test)]
mod tests;

pub(crate) use client::{FrontPageClient, FrontPageTarget, RemoteAccess};

impl crate::NativeRoot {
    pub(crate) fn front_page_client(&self, cx: &gpui::App) -> FrontPageClient {
        self.remote_client.client(cx)
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_layout(&self) -> Option<&runner_app::pane_layout::PaneLayout> {
        if self.remote_client.is_active() {
            self.remote_client.layout()
        } else if self.route == crate::surfaces::AppRoute::Launcher {
            None
        } else {
            self.tabs.active()
        }
    }

    pub(crate) fn front_page_layout_mut(
        &mut self,
    ) -> Option<&mut runner_app::pane_layout::PaneLayout> {
        if self.remote_client.is_active() {
            self.remote_client.layout_mut()
        } else {
            self.tabs.active_mut()
        }
    }

    pub(crate) fn front_page_session(
        &self,
        id: &str,
        cx: &gpui::App,
    ) -> Option<model::SessionDisplay> {
        if self.remote_client.is_active() {
            self.remote_client.session(id, cx)
        } else {
            self.session_entry(id, cx).map(model::SessionDisplay::local)
        }
    }

    pub(crate) fn front_page_status(
        &self,
        id: &str,
        cx: &gpui::App,
    ) -> Option<runner_app::ui::agent_status::StatusPresentation> {
        if self.remote_client.is_active() {
            self.remote_client.status(id, cx)
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
        self.remote_client
            .sidebar()
            .unwrap_or_else(|| self.sidebar.clone())
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_has_islands(&self) -> bool {
        self.remote_client.is_active()
            || matches!(
                self.route,
                crate::surfaces::AppRoute::Chat
                    | crate::surfaces::AppRoute::Launcher
                    | crate::surfaces::AppRoute::Mission(_)
            )
    }

    pub(crate) fn front_page_is_chat(&self) -> bool {
        if self.remote_client.is_active() {
            !matches!(
                self.remote_client.active_item(),
                Some(crate::surfaces::remote_client::RemoteItem::Mission(_))
            )
        } else {
            matches!(
                self.route,
                crate::surfaces::AppRoute::Chat | crate::surfaces::AppRoute::Launcher
            )
        }
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_pane_bounds(
        &self,
    ) -> &std::collections::HashMap<crate::surfaces::PaneKey, gpui::Size<gpui::Pixels>> {
        self.remote_client
            .pane_bounds()
            .unwrap_or(&self.pane_bounds)
    }
    pub(crate) fn front_page_pane_bounds_mut(
        &mut self,
    ) -> &mut std::collections::HashMap<crate::surfaces::PaneKey, gpui::Size<gpui::Pixels>> {
        self.remote_client
            .pane_bounds_mut()
            .unwrap_or(&mut self.pane_bounds)
    }
    pub(crate) fn front_page_resizing_split(&self) -> Option<&str> {
        self.remote_client
            .resizing_split()
            .unwrap_or(self.resizing_split.as_deref())
    }
}

impl crate::NativeRoot {
    pub(crate) fn front_page_sidebar_collapsed(&self) -> bool {
        self.remote_client
            .sidebar_collapsed()
            .unwrap_or(self.sidebar_collapsed)
    }
    pub(crate) fn front_page_sidebar_width(&self, cx: &gpui::App) -> f32 {
        self.remote_client
            .sidebar_width()
            .unwrap_or(self.settings(cx).sidebar_width)
    }
}
