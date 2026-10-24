use crate::surfaces::remote_client::RemoteItem;
use crate::*;

impl NativeRoot {
    pub(crate) fn library_view_current(
        &self,
        client: &super::FrontPageClient,
        epoch: u64,
        cx: &App,
    ) -> bool {
        if client.is_remote() {
            self.remote_window.is_some()
                && self.client_ui.epoch == epoch
                && self.front_page_client(cx).target == client.target
                && client.valid()
        } else {
            self.remote_window.is_none()
        }
    }

    pub(crate) fn render_library_refresh(&self, cx: &Context<Self>) -> gpui::AnyElement {
        use gpui::prelude::*;
        let root = cx.entity();
        let client = self.front_page_client(cx);
        let epoch = self.client_ui.epoch;
        gpui::div()
            .when(cfg!(test), |entry| {
                entry.debug_selector(|| "REMOTE_LIBRARY_REFRESH".into())
            })
            .child(
                runner_app::ui::Button::new(
                    "refresh-host-library",
                    crate::i18n::text("client.refresh_library"),
                )
                .disabled(!self.front_page_client(cx).supports(&["catalog"]))
                .on_press(move |_, cx| {
                    root.update(cx, |root, cx| {
                        if root.library_view_current(&client, epoch, cx) {
                            root.refresh_remote_library(cx);
                        }
                    })
                }),
            )
            .into_any_element()
    }

    pub(crate) fn open_remote_library(
        &mut self,
        item: RemoteItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.front_page_client(cx).supports(&["catalog"]) {
            return;
        }
        self.dismiss_sidebar_transients(cx);
        if let Some(remote) = self.remote_window.as_mut() {
            remote.library_target = None;
        }
        self.select_remote_item(item, cx);
        window.focus(&self.root_focus, cx);
    }

    pub(crate) fn ensure_remote_library(&mut self, cx: &mut Context<Self>) {
        if self.remote_window.as_ref().is_some_and(|remote| {
            matches!(remote.active, Some(RemoteItem::Roles | RemoteItem::Crews))
                && remote.library_target.as_ref() != Some(&remote.front_page.target)
                && remote.front_page.supports(&["catalog"])
        }) {
            self.refresh_remote_library(cx);
        }
    }

    pub(crate) fn refresh_remote_library(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        if !matches!(remote.active, Some(RemoteItem::Roles | RemoteItem::Crews))
            || !remote.front_page.supports(&["catalog"])
        {
            return;
        }
        remote.library_request = remote.library_request.wrapping_add(1);
        let request = remote.library_request;
        let navigation = remote.navigation_generation;
        let client = remote.front_page.clone();
        remote.library_target = Some(client.target.clone());
        remote.role_surfaces.set_library_load(None);
        remote.crew_surfaces.set_library_load(None);
        let epoch = self.client_ui.epoch;
        let guard = client.clone();
        let task = cx.background_spawn(async move { client.catalog() });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root.client_ui.epoch != epoch || !guard.valid() {
                    return;
                }
                let Some(remote) = root.remote_window.as_mut().filter(|remote| {
                    remote.front_page.target == guard.target
                        && remote.library_request == request
                        && remote.navigation_generation == navigation
                }) else {
                    return;
                };
                match result {
                    Ok(catalog) => {
                        remote.library_catalog = Some(catalog);
                        root.load_role_page(cx);
                        root.load_crew_page(cx);
                    }
                    Err(error) => {
                        let error = error.to_string();
                        remote.role_surfaces.set_library_load(Some(error.clone()));
                        remote.crew_surfaces.set_library_load(Some(error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
