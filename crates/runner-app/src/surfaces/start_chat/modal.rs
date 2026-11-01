use super::*;

impl NativeRoot {
    pub(crate) fn open_new_tab_modal(
        &mut self,
        _: &NewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() {
            return;
        }
        if self.launcher_page_visible() {
            if self.remote_client.is_active() {
                self.select_remote_item(crate::surfaces::remote_client::RemoteItem::Launcher, cx);
            } else {
                self.set_route(AppRoute::Launcher, cx);
            }
            let launcher = self.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.open_step(launcher::Step::Start, window, cx)
            });
            return;
        }
        self.open_launcher_page(
            ProjectScope::or_root(self.active_project_id(cx)),
            launcher::Step::Start,
            window,
            cx,
        );
    }

    pub(crate) fn open_sidebar_chat_modal(
        &mut self,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_launcher_page(scope, launcher::Step::Start, window, cx);
    }

    pub(crate) fn open_remote_role_chat(
        &mut self,
        role_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_remote_role_direct(role_id, window, cx);
    }

    pub(super) fn open_add_role_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (roles, role_error) = match self.core(cx).role_list() {
            Ok(roles) => (roles.into_iter().map(Into::into).collect(), None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        let (runtimes, _, error) = load_selectable_runtimes(self.core(cx), self.settings(cx));
        self.build_add_role_form(
            roles,
            runtimes.into_iter().map(Into::into).collect(),
            window,
            cx,
        );
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.error = role_error.or(error).map(Into::into);
        }
    }

    pub(super) fn build_add_role_form(
        &mut self,
        roles: Vec<RoleChoice>,
        runtimes: Vec<RuntimeChoice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let role_id = roles.first().map(|role| role.id.clone());
        let role = roles.first();
        let model = cx.new(|cx| {
            TextField::new(
                cx.focus_handle(),
                "",
                crate::i18n::message("word.default"),
                false,
            )
            .placeholder_as_value(true)
        });
        let root = cx.entity();
        let handler = selection_handler(&root, StartChatSelection::Role);
        let role_select = cx.new(|cx| {
            StyledSelect::new(
                "add-role-role",
                cx.focus_handle(),
                role_id.clone().unwrap_or_default(),
                role_options(&roles),
                handler,
                cx,
            )
            .picker(true)
            .placeholder(crate::i18n::message("ui.no_roles_yet"))
            .disabled(roles.is_empty())
        });
        let handler = selection_handler(&root, StartChatSelection::RoleRuntime);
        let role_runtime_select = cx.new(|cx| {
            StyledSelect::new(
                "add-role-runtime",
                cx.focus_handle(),
                role.map(|role| role.runtime.clone()).unwrap_or_default(),
                role_runtime_options(&runtimes, role),
                handler,
                cx,
            )
            .full_width(true)
            .min_menu_width(px(200.))
        });
        let handler = selection_handler(&root, StartChatSelection::Effort);
        let effort_select = cx.new(|cx| {
            StyledSelect::new(
                "add-role-effort",
                cx.focus_handle(),
                "",
                Vec::new(),
                handler,
                cx,
            )
            .full_width(true)
            .min_menu_width(px(200.))
        });
        let handler = selection_handler(&root, StartChatSelection::Speed);
        let speed_select = cx.new(|cx| {
            StyledSelect::new(
                "add-role-speed",
                cx.focus_handle(),
                "inherit",
                speed_options(None, false),
                handler,
                cx,
            )
            .full_width(true)
            .min_menu_width(px(160.))
        });
        let model_field = cx.new(|cx| ModelField::new(model.clone(), &[], cx));
        let scroll_handle = ScrollHandle::new();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll_handle.clone(), root.entity_id()));
        let subscription = cx.observe(&model, |this, _, cx| {
            if let Some(modal) = this.start_chat_modal.as_mut() {
                sync_model_marker(modal, cx);
                sync_effort_control(modal, cx);
            }
            cx.notify();
        });
        self.start_chat_modal = Some(StartChatModal {
            mission_role: None,
            roles,
            runtimes,
            role_id,
            role_runtime_override: None,
            effort: String::new(),
            speed: "inherit".into(),
            model,
            model_field,
            role_select,
            role_runtime_select,
            effort_select,
            speed_select,
            scroll_handle,
            scrollbar,
            reset_focus: std::array::from_fn(|_| cx.focus_handle()),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
            _model_subscription: subscription,
        });
        let modal = self.start_chat_modal.as_mut().unwrap();
        restore_baseline(modal, cx);
        modal.role_select.read(cx).focus_handle().focus(window, cx);
        self.refresh_start_chat_models(cx);
        cx.notify();
    }

    pub(super) fn refresh_start_chat_models(&self, cx: &Context<Self>) {
        if let Some(runtime) = self
            .start_chat_modal
            .as_ref()
            .and_then(|modal| modal.active_runtime())
            .filter(|runtime| self.settings(cx).model_runtimes().contains(&runtime.name))
        {
            let _ = self.core(cx).runtime_refresh_models(&[runtime.name]);
        }
    }

    pub(crate) fn refresh_start_chat_runtimes(&mut self, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        self.refresh_launchers(cx);
        let (runtimes, _, _) = load_selectable_runtimes(self.core(cx), self.settings(cx));
        if let Some(modal) = self.start_chat_modal.as_mut() {
            modal.runtimes = runtimes.into_iter().map(Into::into).collect();
            sync_role_runtime_select(modal, cx);
            sync_runtime_controls(modal, cx);
        }
        cx.notify();
    }

    pub(crate) fn sync_start_chat_default_runtime(&mut self, cx: &mut Context<Self>) {
        self.refresh_launchers(cx);
    }

    pub(crate) fn remember_active_role(&mut self, cx: &App) {
        let Some(session_id) = self.active_focused_session_id() else {
            return;
        };
        self.last_focused_role_id = self
            .session_entry(&session_id, cx)
            .and_then(|entry| entry.role_id.clone());
    }

    pub(super) fn close_start_chat_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .start_chat_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.start_chat_modal = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn select_start_chat_choice(
        &mut self,
        selection: StartChatSelection,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        match selection {
            StartChatSelection::Role => {
                modal.role_id = Some(value.into());
                clear_role_overrides(modal, cx);
            }
            StartChatSelection::RoleRuntime => {
                let next = (!value.is_empty()
                    && !modal
                        .selected_role()
                        .is_some_and(|role| role.runtime == value))
                .then(|| value.into());
                if next != modal.role_runtime_override {
                    modal.role_runtime_override = next;
                    restore_baseline(modal, cx);
                }
            }
            StartChatSelection::Effort => modal.effort = value.into(),
            StartChatSelection::Speed => modal.speed = value.into(),
        }
        if selection == StartChatSelection::Role {
            self.suggest_add_role_handle(cx);
        }
        self.refresh_start_chat_models(cx);
        cx.notify();
    }

    pub(super) fn on_start_chat_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.close_start_chat_modal(window, cx);
        }
    }
}
