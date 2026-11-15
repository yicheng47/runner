use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, svg, AnyElement, App, ClipboardItem, Context, Entity, FontWeight,
    SharedString, WeakEntity, Window,
};
use runner_app::ui::button::spinner;
use runner_app::ui::{
    app_scrollbar_gutter, Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize,
    RoleAvatar, Scrollbar,
};
use runner_core::protocol::onboarding::{CreateStarterCrewArgs, StarterCrewProposal};
use runner_core::protocol::project::ProjectScope;
use runner_core::protocol::runtime::{
    npm_install_command, RuntimeCatalogEntry, RuntimeExecutableStatus, RuntimeRowState,
    RuntimeSignInStatus, RuntimeStatusResponse, NODEJS_DOWNLOAD_URL,
};
use runner_core::protocol::{
    runtime_metadata, Runtime, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask,
};

use crate::chat_icon::ChatIcon;
use crate::surfaces::AppRoute;
use crate::{theme, NativeRoot};

#[derive(Default)]
pub(crate) struct SetupState {
    pub(crate) open: bool,
    pub(crate) create_starter_crew: Option<bool>,
    pub(crate) more_agents: bool,
    pub(crate) rechecking: bool,
    pub(crate) loading: bool,
    pub(crate) finishing: bool,
    pub(crate) status: Option<RuntimeStatusResponse>,
    pub(crate) catalog: Vec<RuntimeCatalogEntry>,
    pub(crate) proposal: Option<StarterCrewProposal>,
    pub(crate) copied_runtime: Option<Runtime>,
    pub(crate) scroll: gpui::ScrollHandle,
    pub(crate) agents_scroll: gpui::ScrollHandle,
    agents_scrollbar: Option<Entity<Scrollbar>>,
    copied_generation: u64,
    generation: u64,
    loaded_once: bool,
}

pub(crate) fn partition_setup_runtimes(
    runtimes: &[RuntimeExecutableStatus],
) -> (Vec<RuntimeExecutableStatus>, Vec<RuntimeExecutableStatus>) {
    let mut ordered = runtimes.to_vec();
    ordered.sort_by_key(|runtime| runtime.name.setup_rank());
    let split_at = ordered.len().min(4);
    let overflow = ordered.split_off(split_at);
    (ordered, overflow)
}

pub(crate) fn setup_row_installed(runtime: &RuntimeExecutableStatus) -> bool {
    matches!(
        runtime.state,
        RuntimeRowState::Detected | RuntimeRowState::Override
    )
}

fn enabled_chat_runtimes(settings: &crate::app_settings::AppSettings) -> Vec<Runtime> {
    Runtime::SETUP_ORDER
        .into_iter()
        .filter(|runtime| {
            runtime_metadata::catalog_for(*runtime)
                .is_some_and(|meta| settings.is_agent_enabled(*runtime, meta.default_enabled))
        })
        .collect()
}

fn starter_crew_args(settings: &crate::app_settings::AppSettings) -> CreateStarterCrewArgs {
    CreateStarterCrewArgs {
        default_runtime: Runtime::parse(&settings.default_runtime),
        enabled_runtimes: Some(enabled_chat_runtimes(settings)),
    }
}

fn active_setup_install_task(
    task: Option<&RuntimeUpdateTask>,
    installed: bool,
) -> Option<&RuntimeUpdateTask> {
    task.filter(|task| {
        task.kind == RuntimeTaskKind::Install
            && (!installed || task.state != RuntimeUpdateState::Succeeded)
    })
}

fn setup_install_task_badge_key(state: RuntimeUpdateState) -> &'static str {
    match state {
        RuntimeUpdateState::Running
        | RuntimeUpdateState::Verifying
        | RuntimeUpdateState::Stopping => "onboarding.installing",
        RuntimeUpdateState::MayNeedInput => "onboarding.may_need_input",
        RuntimeUpdateState::Failed | RuntimeUpdateState::BrokenAfterStop => {
            "onboarding.install_failed"
        }
        RuntimeUpdateState::Succeeded => "ui.installed",
    }
}

fn suggested_second_runtime(primary: Runtime) -> Runtime {
    Runtime::SETUP_ORDER
        .into_iter()
        .find(|candidate| *candidate != primary)
        .unwrap_or(Runtime::Codex)
}

fn runtime_tile(runtime: Runtime) -> AnyElement {
    let icon = ChatIcon::for_runtime(runtime.key());
    div()
        .size(rems(24. / 16.))
        .flex_none()
        .rounded(rems(6. / 16.))
        .bg(theme::raised())
        .flex()
        .items_center()
        .justify_center()
        .child(icon.render(rems(14. / 16.), icon.color(theme::text(), true), true))
        .into_any_element()
}

fn status_dot(color: gpui::Hsla) -> AnyElement {
    div()
        .size(rems(6. / 16.))
        .flex_none()
        .rounded_full()
        .bg(color)
        .into_any_element()
}

fn manual_install_detail(name: Runtime, catalog: Option<&RuntimeCatalogEntry>) -> Option<String> {
    match name {
        Runtime::Cursor => Some("curl https://cursor.com/install -fsS | bash".to_owned()),
        _ => catalog
            .map(|entry| entry.install_url.trim().to_owned())
            .filter(|url| !url.is_empty()),
    }
}

fn copy_text_for(
    runtime: &RuntimeExecutableStatus,
    catalog: Option<&RuntimeCatalogEntry>,
    npm_available: bool,
) -> Option<String> {
    if let Some(cmd) = npm_install_command(runtime.name) {
        return npm_available.then_some(cmd);
    }
    manual_install_detail(runtime.name, catalog)
}

fn setup_copy_button(
    name: Runtime,
    cmd: String,
    copied: bool,
    root_weak: WeakEntity<NativeRoot>,
) -> AnyElement {
    div()
        .debug_selector(move || format!("SETUP_COPY_{name}"))
        .child(
            IconButton::new(
                SharedString::from(format!("setup-copy-{name}")),
                if copied { "check.svg" } else { "copy.svg" },
            )
            .size(IconButtonSize::Sm)
            .tooltip(if copied {
                crate::i18n::text("onboarding.copied")
            } else {
                crate::i18n::text("onboarding.copy")
            })
            .on_press(move |_, cx| {
                let cmd = cmd.clone();
                let _ = root_weak.update(cx, |root, cx| {
                    root.copy_setup_command(name, cmd, cx);
                });
            }),
        )
        .into_any_element()
}

fn vocabulary_card(icon: &'static str, term: &'static str, definition: &'static str) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .p(rems(12. / 16.))
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(
                    svg()
                        .path(icon)
                        .size(rems(13. / 16.))
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .text_size(rems(12. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text(term)),
                ),
        )
        .child(
            div()
                .whitespace_normal()
                .text_size(rems(12. / 16.))
                .line_height(rems(17.4 / 16.))
                .text_color(theme::muted())
                .child(crate::i18n::text(definition)),
        )
        .into_any_element()
}

impl NativeRoot {
    pub(crate) fn setup_visible(&self, cx: &App) -> bool {
        if self.remote_client.is_active()
            || self.front_page_layout().is_some()
            || self
                .page_launcher(cx)
                .is_some_and(|launcher| launcher.read(cx).has_update_retry())
        {
            return false;
        }
        self.setup.open || !self.app_store.read(cx).onboarding_completed()
    }

    pub(crate) fn open_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        self.setup.open = true;
        self.setup.create_starter_crew = None;
        self.setup.more_agents = false;
        self.setup.finishing = false;
        self.setup.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.setup
            .agents_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        self.set_route(AppRoute::Launcher, cx);
        self.refresh_setup_data(cx);
        self.chat_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn refresh_setup_data(&mut self, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        self.setup.generation += 1;
        let generation = self.setup.generation;
        self.setup.loading = true;
        self.setup.loaded_once = true;
        let core = self.core(cx).clone();
        let args = starter_crew_args(self.settings(cx));
        let task = cx.background_spawn(async move {
            let status = core.runtime_status_list().ok();
            let catalog = core.runtime_catalog().unwrap_or_default();
            let proposal = core.onboarding_starter_crew_proposal(args).ok();
            (status, catalog, proposal)
        });
        cx.spawn(async move |weak, cx| {
            let (status, catalog, proposal) = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root.setup.generation != generation {
                    return;
                }
                root.setup.loading = false;
                if status.is_some() {
                    root.setup.status = status;
                }
                if !catalog.is_empty() {
                    root.setup.catalog = catalog;
                }
                if proposal.is_some() {
                    root.setup.proposal = proposal;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn recheck_setup(&mut self, cx: &mut Context<Self>) {
        if self.setup.rechecking || self.remote_client.is_active() {
            return;
        }
        self.setup.generation += 1;
        let generation = self.setup.generation;
        self.setup.rechecking = true;
        let core = self.core(cx).clone();
        let settings = self.settings(cx).clone();
        let model_runtimes = settings.model_runtimes();
        let args = starter_crew_args(&settings);
        let task = cx.background_spawn(async move {
            let status = core.runtime_refresh(&model_runtimes).ok();
            let catalog = core.runtime_catalog().unwrap_or_default();
            let proposal = core.onboarding_starter_crew_proposal(args).ok();
            (status, catalog, proposal)
        });
        cx.spawn(async move |weak, cx| {
            let (status, catalog, proposal) = task.await;
            let _ = weak.update(cx, |root, cx| {
                root.setup.rechecking = false;
                if root.setup.generation != generation {
                    return;
                }
                if status.is_some() {
                    root.setup.status = status;
                }
                if !catalog.is_empty() {
                    root.setup.catalog = catalog;
                }
                if proposal.is_some() {
                    root.setup.proposal = proposal;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn copy_setup_command(&mut self, runtime: Runtime, command: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(command));
        self.setup.copied_runtime = Some(runtime);
        self.setup.copied_generation += 1;
        let generation = self.setup.copied_generation;
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = weak.update(cx, |root, cx| {
                if root.setup.copied_generation == generation {
                    root.setup.copied_runtime = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn open_setup_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let home =
            runner_core::app_paths::home_dir().map(|path| path.to_string_lossy().into_owned());
        self.new_terminal_tab_in(ProjectScope::Root, home, window, cx);
    }

    fn stop_setup_task(&mut self, task_id: String, cx: &mut Context<Self>) {
        let core = self.core(cx).clone();
        cx.background_spawn(async move {
            let _ = core.runtime_task_stop(&task_id);
        })
        .detach();
    }

    pub(crate) fn skip_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_setup(false, window, cx);
    }

    fn setup_can_start(&self, cx: &App) -> bool {
        if !self.setup.loaded_once || self.setup.loading || self.setup.rechecking {
            return false;
        }
        let Some(status) = self.setup.status.as_ref() else {
            return false;
        };
        if status.shell.checking {
            return false;
        }
        let Some(proposal) = self.setup.proposal.as_ref() else {
            return false;
        };
        if proposal.slots.is_empty() {
            return false;
        }
        let enabled = enabled_chat_runtimes(self.settings(cx));
        status.runtimes.iter().any(|runtime| {
            setup_row_installed(runtime)
                && runtime.name != Runtime::Shell
                && enabled.contains(&runtime.name)
        })
    }

    pub(crate) fn complete_setup_and_open_launcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.setup_can_start(cx) {
            return;
        }
        self.finish_setup(true, window, cx);
    }

    fn finish_setup(
        &mut self,
        start_first_chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.setup.finishing || self.remote_client.is_active() {
            return;
        }
        let has_crews = self.app_store.read(cx).has_crews();
        let proposal_has_slots = self
            .setup
            .proposal
            .as_ref()
            .is_some_and(|proposal| !proposal.slots.is_empty());
        let create_crew = start_first_chat
            && !has_crews
            && proposal_has_slots
            && self.setup.create_starter_crew != Some(false);
        let already_completed = self.app_store.read(cx).onboarding_completed();
        let core = self.core(cx).clone();
        let args = starter_crew_args(self.settings(cx));
        self.setup.finishing = true;
        cx.notify();
        let work = cx.background_spawn(async move {
            if create_crew {
                core.onboarding_create_starter_crew(args)?;
            }
            if !already_completed {
                Ok(Some(core.onboarding_complete()?))
            } else if create_crew {
                Ok(Some(core.onboarding_status()?))
            } else {
                Ok(None)
            }
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result: anyhow::Result<
                Option<runner_core::protocol::onboarding::OnboardingStatus>,
            > = work.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                root.setup.finishing = false;
                match result {
                    Ok(status) => {
                        root.setup.open = false;
                        root.setup.create_starter_crew = None;
                        root.setup.more_agents = false;
                        root.setup
                            .agents_scroll
                            .set_offset(gpui::point(px(0.), px(0.)));
                        root.app_store.update(cx, |store, cx| {
                            if let Some(status) = status {
                                store.onboarding = status;
                                store.revisions.onboarding += 1;
                            }
                            if create_crew {
                                store.refresh_crews_inner();
                                store.refresh_roles_inner();
                            }
                            cx.notify();
                        });
                        root.open_launcher_page(
                            ProjectScope::Root,
                            super::launcher::Step::Start,
                            false,
                            window,
                            cx,
                        );
                        if start_first_chat {
                            let launcher = root.render_launcher(None, true, window, cx);
                            launcher.update(cx, |launcher, cx| {
                                launcher.enable_sample_prompt(window, cx);
                            });
                        }
                    }
                    Err(error) => {
                        root.error = Some(error.to_string().into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_setup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.setup.loaded_once && !self.setup.loading {
            self.refresh_setup_data(cx);
        }
        let status = self.setup.status.as_ref();
        let npm_available = status.is_some_and(|s| s.npm_available);
        let checking = self.setup.rechecking
            || status.is_some_and(|s| s.shell.checking)
            || (self.setup.loading && status.is_none());
        let runtimes = status.map(|s| s.runtimes.as_slice()).unwrap_or(&[]);
        let installed_count = runtimes
            .iter()
            .filter(|runtime| setup_row_installed(runtime))
            .count();
        let count_label = if installed_count > 0 {
            crate::i18n::tf(
                "onboarding.agents_found",
                &[("count", installed_count.to_string())],
            )
        } else {
            crate::i18n::text("onboarding.no_agents_found").into()
        };
        let (primary, overflow) = partition_setup_runtimes(runtimes);
        let agent_card = self.render_setup_agents_card(
            &primary,
            &overflow,
            runtimes.is_empty(),
            npm_available,
            checking,
            cx,
        );

        let show_starter_crew = !(self.setup.open && self.app_store.read(cx).has_crews());
        let task_pill_visible = !self.agent_tasks.hidden && !self.agent_tasks.tasks.is_empty();
        let recheck_disabled = checking;
        let finishing = self.setup.finishing;
        let can_start = self.setup_can_start(cx);
        let reopened = self.setup.open;
        let root_panel = cx.weak_entity();
        div()
            .debug_selector(|| "SETUP_SURFACE".into())
            .relative()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .capture_key_down(
                cx.listener(move |root, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && root.setup.open {
                        cx.stop_propagation();
                        root.skip_setup(window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id("setup-surface")
                    .w_full()
                    .max_h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.setup.scroll)
                    .child(
                        div()
                            .debug_selector(|| "SETUP_CONTENT".into())
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .px(rems(24. / 16.))
                            .pt(rems(24. / 16.))
                            .pb(rems(if task_pill_visible { 76. } else { 24. } / 16.))
                            .child(
                                div()
                                    .debug_selector(|| "SETUP_BRAND".into())
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(rems(12. / 16.))
                                    .pb(rems(8. / 16.))
                                    .child(
                                        div()
                                            .w(rems(52. / 16.))
                                            .h(rems(40. / 16.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                svg()
                                                    .flex_none()
                                                    .path("brand-mark.svg")
                                                    .size(rems(70. / 16.))
                                                    .text_color(theme::accent()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(20. / 16.))
                                            .line_height(rems(24. / 16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child(crate::i18n::text("onboarding.welcome")),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .max_w(rems(600. / 16.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(20. / 16.))
                                    .child(
                                        div()
                                            .whitespace_normal()
                                            .text_size(rems(13. / 16.))
                                            .line_height(rems(19.5 / 16.))
                                            .text_color(theme::muted())
                                            .child(crate::i18n::text("onboarding.lead")),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .gap(rems(12. / 16.))
                                            .child(vocabulary_card(
                                                "message-square.svg",
                                                "onboarding.term_chat",
                                                "onboarding.def_chat",
                                            ))
                                            .child(vocabulary_card(
                                                "users.svg",
                                                "onboarding.term_crew",
                                                "onboarding.def_crew",
                                            ))
                                            .child(vocabulary_card(
                                                "flag.svg",
                                                "onboarding.term_mission",
                                                "onboarding.def_mission",
                                            )),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .flex_col()
                                            .gap(rems(8. / 16.))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .flex()
                                                    .items_center()
                                                    .gap(rems(8. / 16.))
                                                    .child(
                                                        div()
                                                            .text_size(rems(13. / 16.))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(theme::text())
                                                            .child(crate::i18n::text(
                                                                "onboarding.your_agents",
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .debug_selector(|| {
                                                                "SETUP_AGENTS_COUNT".into()
                                                            })
                                                            .text_size(rems(12. / 16.))
                                                            .text_color(theme::muted())
                                                            .child(count_label),
                                                    )
                                                    .child(div().flex_1())
                                                    .child(
                                                        Button::new(
                                                            "setup-recheck",
                                                            if checking {
                                                                crate::i18n::text(
                                                                    "onboarding.checking",
                                                                )
                                                            } else {
                                                                crate::i18n::text(
                                                                    "onboarding.recheck",
                                                                )
                                                            },
                                                        )
                                                        .debug_selector("SETUP_RECHECK")
                                                        .icon("refresh-cw.svg")
                                                        .size(ButtonSize::Sm)
                                                        .variant(ButtonVariant::Secondary)
                                                        .disabled(recheck_disabled)
                                                        .on_press(move |_, cx| {
                                                            let _ = root_panel.update(
                                                                cx,
                                                                |root, cx| {
                                                                    root.recheck_setup(cx);
                                                                },
                                                            );
                                                        }),
                                                    ),
                                            )
                                            .child(agent_card),
                                    )
                                    .children(
                                        show_starter_crew
                                            .then(|| self.render_setup_starter_crew(cx)),
                                    )
                                    .child(self.render_setup_footer(
                                        finishing, can_start, reopened, window, cx,
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_setup_agents_card(
        &mut self,
        primary: &[RuntimeExecutableStatus],
        overflow: &[RuntimeExecutableStatus],
        runtimes_empty: bool,
        npm_available: bool,
        checking: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card = div()
            .debug_selector(|| "SETUP_AGENTS_CARD".into())
            .w_full()
            .flex()
            .flex_col()
            .rounded(rems(10. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel());

        if runtimes_empty && checking {
            return card
                .p(rems(4. / 16.))
                .child(
                    div()
                        .p_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(theme::text_ui())
                        .text_color(theme::muted())
                        .child(spinner("setup-initial-probe", 12., theme::muted()))
                        .child(crate::i18n::text("onboarding.checking")),
                )
                .into_any_element();
        }

        let overflow_count = overflow.len();
        if self.setup.more_agents && overflow_count > 0 {
            let rows: Vec<_> = primary
                .iter()
                .chain(overflow.iter())
                .map(|runtime| self.render_setup_agent_row(runtime, npm_available, cx))
                .collect();
            let total_rows = primary.len() + overflow_count;
            let owner = cx.entity_id();
            let agents_scroll = self.setup.agents_scroll.clone();
            let scrollbar = self
                .setup
                .agents_scrollbar
                .get_or_insert_with(|| cx.new(|_| Scrollbar::app(agents_scroll, owner)))
                .clone();
            let measured = self.setup.agents_scroll.clone();
            let agents_scrolls = measured.max_offset().y >= px(1.);
            let scroll_more_count = total_rows.saturating_sub(5);
            let footer_meta = (agents_scrolls && scroll_more_count > 0).then(|| {
                crate::i18n::tf(
                    "onboarding.scroll_for_more",
                    &[
                        ("count", scroll_more_count.to_string()),
                        ("total", total_rows.to_string()),
                    ],
                )
            });

            return card
                .overflow_hidden()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .max_h(rems(216. / 16.))
                        .overflow_hidden()
                        .child(
                            div()
                                .id("setup-agents-scroll")
                                .debug_selector(|| "SETUP_AGENTS_SCROLL".into())
                                .w_full()
                                .max_h(rems(216. / 16.))
                                .overflow_y_scroll()
                                .scrollbar_width(px(0.))
                                .track_scroll(&self.setup.agents_scroll)
                                .flex()
                                .flex_col()
                                .p(rems(4. / 16.))
                                .when(agents_scrolls, |el| el.pr(app_scrollbar_gutter()))
                                .children(rows),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(rems(4. / 16.))
                                .bottom(rems(4. / 16.))
                                .left_0()
                                .right_0()
                                .child(
                                    div()
                                        .debug_selector(|| "SETUP_AGENTS_SCROLLBAR".into())
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .right_0()
                                        .w(app_scrollbar_gutter()),
                                )
                                .child(scrollbar),
                        )
                        .child(
                            canvas(
                                move |_, window, cx| {
                                    if (measured.max_offset().y >= px(1.)) != agents_scrolls {
                                        let view = window.current_view();
                                        cx.defer(move |cx| cx.notify(view));
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_0(),
                        ),
                )
                .child(div().w_full().h(px(1.)).bg(theme::border()))
                .child(
                    div()
                        .id("setup-more-agents")
                        .debug_selector(|| "SETUP_MORE_AGENTS".into())
                        .w_full()
                        .h(rems(32. / 16.))
                        .px(rems(12. / 16.))
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .cursor_pointer()
                        .hover(|row| row.bg(theme::sidebar_selected()))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.setup.more_agents = false;
                            root.setup
                                .agents_scroll
                                .set_offset(gpui::point(px(0.), px(0.)));
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .flex_none()
                                .path("chevron-up.svg")
                                .size(rems(12. / 16.))
                                .text_color(theme::muted()),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(rems(12. / 16.))
                                .text_color(theme::muted())
                                .child(crate::i18n::text("onboarding.hide_more_agents")),
                        )
                        .child(div().flex_1())
                        .children(footer_meta.map(|meta| {
                            div()
                                .flex_none()
                                .debug_selector(|| "SETUP_AGENTS_SCROLL_META".into())
                                .text_size(rems(11. / 16.))
                                .text_color(theme::muted())
                                .child(meta)
                        })),
                )
                .into_any_element();
        }

        let mut collapsed = card.p(rems(4. / 16.)).children(
            primary
                .iter()
                .map(|runtime| self.render_setup_agent_row(runtime, npm_available, cx)),
        );
        if overflow_count > 0 {
            let overflow_names = overflow
                .iter()
                .take(3)
                .map(|runtime| runtime.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let more_label = crate::i18n::tf(
                "onboarding.more_agents_sample",
                &[
                    ("count", overflow_count.to_string()),
                    ("names", overflow_names),
                ],
            );
            collapsed = collapsed
                .child(div().h(px(1.)).mx_2().my_1().bg(theme::border()))
                .child(
                    div()
                        .id("setup-more-agents")
                        .debug_selector(|| "SETUP_MORE_AGENTS".into())
                        .h(rems(32. / 16.))
                        .px(rems(10. / 16.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(rems(6. / 16.))
                        .cursor_pointer()
                        .hover(|row| row.bg(theme::sidebar_selected()))
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.setup.more_agents = true;
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .flex_none()
                                .path("chevron-right.svg")
                                .size(rems(12. / 16.))
                                .text_color(theme::muted()),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(rems(12. / 16.))
                                .text_color(theme::muted())
                                .child(more_label),
                        ),
                );
        }
        collapsed.into_any_element()
    }

    fn render_setup_agent_row(
        &self,
        runtime: &RuntimeExecutableStatus,
        npm_available: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = runtime.name;
        let installed = setup_row_installed(runtime);
        let catalog = self
            .setup
            .catalog
            .iter()
            .find(|entry| entry.name == name)
            .cloned();
        let catalog_ref = catalog.as_ref();
        let npm_cmd = npm_install_command(name);
        let task = self
            .agent_tasks
            .tasks
            .iter()
            .find(|task| task.runtime == name)
            .cloned();
        let copy_cmd = copy_text_for(runtime, catalog_ref, npm_available);
        let copied = self.setup.copied_runtime == Some(name);
        let root_weak = cx.weak_entity();

        let install_task = active_setup_install_task(task.as_ref(), installed);
        let (dot_color, badge_text, badge_color, detail_text, detail_mono, right_actions): (
            Option<gpui::Hsla>,
            Option<String>,
            gpui::Hsla,
            Option<String>,
            bool,
            Vec<AnyElement>,
        ) = if let Some(task) = install_task {
            let badge_key = setup_install_task_badge_key(task.state);
            match task.state {
                RuntimeUpdateState::Running
                | RuntimeUpdateState::Verifying
                | RuntimeUpdateState::Stopping => {
                    let task_id = task.id.clone();
                    let root_stop = root_weak.clone();
                    let step = (!task.step.is_empty()).then(|| task.step.clone());
                    (
                        None,
                        Some(crate::i18n::text(badge_key).into()),
                        theme::muted(),
                        step,
                        true,
                        vec![
                            spinner(
                                SharedString::from(format!("setup-install-spinner-{name}")),
                                12.,
                                theme::muted(),
                            )
                            .into_any_element(),
                            div()
                                .debug_selector(move || format!("SETUP_INSTALL_STOP_{name}"))
                                .child(
                                    IconButton::new(
                                        SharedString::from(format!("setup-install-stop-{name}")),
                                        "close.svg",
                                    )
                                    .size(IconButtonSize::Sm)
                                    .tooltip(crate::i18n::text("ui.stop"))
                                    .on_press(move |_, cx| {
                                        let task_id = task_id.clone();
                                        let _ = root_stop.update(cx, |root, cx| {
                                            root.stop_setup_task(task_id, cx);
                                        });
                                    }),
                                )
                                .into_any_element(),
                        ],
                    )
                }
                RuntimeUpdateState::MayNeedInput => {
                    let task_dialog = task.clone();
                    let root_answer = root_weak.clone();
                    let step = (!task.step.is_empty()).then(|| task.step.clone());
                    (
                        Some(theme::warning()),
                        Some(crate::i18n::text(badge_key).into()),
                        theme::warning(),
                        step,
                        false,
                        vec![Button::new(
                            SharedString::from(format!("setup-install-answer-{name}")),
                            crate::i18n::text("onboarding.answer"),
                        )
                        .debug_selector(format!("SETUP_INSTALL_ANSWER_{name}"))
                        .icon("corner-down-left.svg")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .on_press(move |window, cx| {
                            let task_dialog = task_dialog.clone();
                            let _ = root_answer.update(cx, |root, cx| {
                                root.show_target_task(task_dialog, None, window, cx);
                            });
                        })
                        .into_any_element()],
                    )
                }
                RuntimeUpdateState::Failed | RuntimeUpdateState::BrokenAfterStop => {
                    let root_retry = root_weak.clone();
                    let detail = match (task.exit_code, task.message.as_deref()) {
                        (Some(code), Some(reason)) => Some(crate::i18n::tf(
                            "onboarding.install_failed_exit",
                            &[("code", code.to_string()), ("reason", reason.to_owned())],
                        )),
                        (_, Some(reason)) => Some(reason.to_owned()),
                        (Some(code), None) => Some(crate::i18n::tf(
                            "format.exited_with_code",
                            &[("code", code.to_string())],
                        )),
                        (None, None) => None,
                    };
                    (
                        Some(theme::danger()),
                        Some(crate::i18n::text(badge_key).into()),
                        theme::danger(),
                        detail,
                        false,
                        vec![Button::new(
                            SharedString::from(format!("setup-install-retry-{name}")),
                            crate::i18n::text("onboarding.retry"),
                        )
                        .debug_selector(format!("SETUP_INSTALL_RETRY_{name}"))
                        .icon("rotate-cw.svg")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .on_press(move |_, cx| {
                            let _ = root_retry.update(cx, |root, cx| {
                                root.start_agent_install(name, cx);
                            });
                        })
                        .into_any_element()],
                    )
                }
                RuntimeUpdateState::Succeeded => (
                    Some(theme::accent()),
                    Some(crate::i18n::text(badge_key).into()),
                    theme::accent(),
                    None,
                    false,
                    Vec::new(),
                ),
            }
        } else if installed {
            match runtime.sign_in {
                RuntimeSignInStatus::SignedIn => (
                    Some(theme::accent()),
                    Some(crate::i18n::text("onboarding.signed_in").into()),
                    theme::accent(),
                    None,
                    false,
                    Vec::new(),
                ),
                RuntimeSignInStatus::SignInNeeded => {
                    let root_term = root_weak.clone();
                    let detail = crate::i18n::tf(
                        "onboarding.sign_in_detail",
                        &[("command", runtime.command.clone())],
                    );
                    (
                        Some(theme::warning()),
                        Some(crate::i18n::text("onboarding.sign_in_needed").into()),
                        theme::warning(),
                        Some(detail),
                        false,
                        vec![Button::new(
                            SharedString::from(format!("setup-open-terminal-{name}")),
                            crate::i18n::text("onboarding.open_terminal"),
                        )
                        .debug_selector(format!("SETUP_OPEN_TERMINAL_{name}"))
                        .icon("terminal.svg")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .on_press(move |window, cx| {
                            let _ = root_term.update(cx, |root, cx| {
                                root.open_setup_terminal(window, cx);
                            });
                        })
                        .into_any_element()],
                    )
                }
                RuntimeSignInStatus::Unknown => (
                    Some(theme::faint()),
                    Some(crate::i18n::text("ui.installed").into()),
                    theme::muted(),
                    None,
                    false,
                    Vec::new(),
                ),
            }
        } else if let Some(cmd) = npm_cmd {
            if npm_available {
                let mut actions = Vec::new();
                if let Some(copy) = copy_cmd {
                    actions.push(setup_copy_button(name, copy, copied, root_weak.clone()));
                }
                let root_install = root_weak.clone();
                actions.push(
                    Button::new(
                        SharedString::from(format!("setup-install-{name}")),
                        crate::i18n::text("onboarding.install"),
                    )
                    .debug_selector(format!("SETUP_INSTALL_{name}"))
                    .icon("download.svg")
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .on_press(move |_, cx| {
                        let _ = root_install.update(cx, |root, cx| {
                            root.start_agent_install(name, cx);
                        });
                    })
                    .into_any_element(),
                );
                (
                    Some(theme::faint()),
                    Some(crate::i18n::text("onboarding.not_installed").into()),
                    theme::muted(),
                    Some(cmd),
                    true,
                    actions,
                )
            } else {
                (
                    Some(theme::faint()),
                    Some(crate::i18n::text("onboarding.not_installed").into()),
                    theme::muted(),
                    Some(crate::i18n::text("onboarding.no_npm_detail").into()),
                    false,
                    vec![Button::new(
                        SharedString::from(format!("setup-nodejs-{name}")),
                        crate::i18n::text("onboarding.nodejs"),
                    )
                    .debug_selector(format!("SETUP_NODEJS_{name}"))
                    .icon("arrow-up-right.svg")
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .on_press(|_, cx| cx.open_url(NODEJS_DOWNLOAD_URL))
                    .into_any_element()],
                )
            }
        } else {
            let mut actions = Vec::new();
            if let Some(cmd) = copy_cmd {
                actions.push(setup_copy_button(name, cmd, copied, root_weak.clone()));
            }
            if let Some(url) = catalog_ref
                .map(|entry| entry.install_url.clone())
                .filter(|url| !url.is_empty())
                .or_else(|| {
                    runtime_metadata::catalog_for(name)
                        .map(|entry| entry.install_url.to_owned())
                        .filter(|url| !url.is_empty())
                })
            {
                actions.push(
                    Button::new(
                        SharedString::from(format!("setup-guide-{name}")),
                        crate::i18n::text("onboarding.guide"),
                    )
                    .debug_selector(format!("SETUP_GUIDE_{name}"))
                    .icon("arrow-up-right.svg")
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .on_press(move |_, cx| cx.open_url(&url))
                    .into_any_element(),
                );
            }
            let detail = manual_install_detail(name, catalog_ref);
            (
                Some(theme::faint()),
                Some(crate::i18n::text("onboarding.not_installed").into()),
                theme::muted(),
                detail,
                true,
                actions,
            )
        };

        let is_active_install = install_task.is_some_and(|t| {
            t.state.active() && !matches!(t.state, RuntimeUpdateState::MayNeedInput)
        });

        div()
            .debug_selector(move || format!("SETUP_AGENT_ROW_{name}"))
            .w_full()
            .min_h(rems(44. / 16.))
            .px(rems(10. / 16.))
            .py(rems(6. / 16.))
            .flex()
            .items_center()
            .gap(rems(10. / 16.))
            .rounded(rems(6. / 16.))
            .child(runtime_tile(name))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(rems(8. / 16.))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(runtime.display_name.clone()),
                            )
                            .when(is_active_install, |row| {
                                row.child(spinner(
                                    SharedString::from(format!("setup-inline-spin-{name}")),
                                    11.,
                                    theme::muted(),
                                ))
                            })
                            .children(badge_text.map(|label| {
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(rems(5. / 16.))
                                    .children(dot_color.map(status_dot))
                                    .child(
                                        div()
                                            .text_size(rems(12. / 16.))
                                            .text_color(badge_color)
                                            .child(label),
                                    )
                            })),
                    )
                    .children(detail_text.map(|detail| {
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .truncate()
                            .when(detail_mono, |el| el.font_family(theme::UI_MONOSPACE_FONT))
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(detail)
                    })),
            )
            .children(
                (installed && install_task.is_none())
                    .then(|| {
                        runtime
                            .installed_version
                            .as_ref()
                            .filter(|v| !v.is_empty())
                            .map(|version| {
                                div()
                                    .flex_none()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(version.clone())
                            })
                    })
                    .flatten(),
            )
            .children((!right_actions.is_empty()).then(|| {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .children(right_actions)
            }))
            .into_any_element()
    }

    fn render_setup_starter_crew(&self, cx: &mut Context<Self>) -> AnyElement {
        let proposal = self.setup.proposal.as_ref();
        let slots = proposal.map(|p| p.slots.as_slice()).unwrap_or(&[]);
        let eligible = slots.len() >= 2;
        let creating = eligible && self.setup.create_starter_crew != Some(false);
        let hint = if !eligible {
            crate::i18n::text("onboarding.starter_hint_needs_agent")
        } else if creating {
            crate::i18n::text("onboarding.starter_hint_will_create")
        } else {
            crate::i18n::text("onboarding.starter_hint_wont_create")
        };
        let note: String = if !eligible {
            crate::i18n::text("onboarding.starter_note_needs_agent").into()
        } else if slots[0].runtime != slots[1].runtime {
            crate::i18n::text("onboarding.starter_note_distinct").into()
        } else {
            let runtime = slots[0].runtime;
            let suggestion = suggested_second_runtime(runtime);
            crate::i18n::tf(
                "onboarding.starter_note_same",
                &[
                    ("runtime", runtime.display_name().into()),
                    ("suggestion", suggestion.display_name().into()),
                ],
            )
        };
        let root_toggle = cx.weak_entity();
        div()
            .debug_selector(|| "SETUP_STARTER_CREW".into())
            .w_full()
            .flex()
            .flex_col()
            .gap(rems(8. / 16.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(rems(8. / 16.))
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(crate::i18n::text("onboarding.starter_crew")),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::muted())
                            .child(hint),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(rems(14. / 16.))
                    .px(rems(14. / 16.))
                    .py(rems(12. / 16.))
                    .rounded(rems(10. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .when(!creating, |card| card.opacity(0.65))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .line_height(rems(18. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("onboarding.starter_crew_name")),
                            )
                            .when(eligible, |info| {
                                info.child(
                                    div()
                                        .debug_selector(|| "SETUP_STARTER_SLOTS".into())
                                        .w_full()
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(rems(8. / 16.))
                                        .children(slots.iter().enumerate().map(|(index, slot)| {
                                            let icon = ChatIcon::for_runtime(slot.runtime.key());
                                            div()
                                                .debug_selector(move || {
                                                    format!("SETUP_STARTER_SLOT_{index}")
                                                })
                                                .max_w_full()
                                                .flex()
                                                .items_center()
                                                .gap(rems(4. / 16.))
                                                .px(rems(6. / 16.))
                                                .py(rems(2. / 16.))
                                                .rounded(rems(4. / 16.))
                                                .bg(theme::raised())
                                                .child(RoleAvatar::new(slot.handle.clone(), 14.))
                                                .child(div().flex_none().child(icon.render(
                                                    rems(11. / 16.),
                                                    icon.color(theme::muted(), true),
                                                    true,
                                                )))
                                                .child(
                                                    div()
                                                        .min_w(px(0.))
                                                        .truncate()
                                                        .text_size(rems(11. / 16.))
                                                        .text_color(theme::muted())
                                                        .child(format!(
                                                            "@{} · {}",
                                                            slot.handle,
                                                            slot.runtime.display_name()
                                                        )),
                                                )
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_size(rems(12. / 16.))
                                    .line_height(rems(17.4 / 16.))
                                    .text_color(theme::muted())
                                    .child(note),
                            ),
                    )
                    .when(eligible, |card| {
                        card.child(
                            div()
                                .debug_selector(|| "SETUP_STARTER_STATUS".into())
                                .flex_none()
                                .flex()
                                .flex_col()
                                .items_end()
                                .gap(rems(4. / 16.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(rems(5. / 16.))
                                        .child(
                                            svg()
                                                .path(if creating {
                                                    "circle-check.svg"
                                                } else {
                                                    "circle-minus.svg"
                                                })
                                                .size(rems(13. / 16.))
                                                .text_color(if creating {
                                                    theme::accent()
                                                } else {
                                                    theme::muted()
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(rems(12. / 16.))
                                                .text_color(if creating {
                                                    theme::text()
                                                } else {
                                                    theme::muted()
                                                })
                                                .child(crate::i18n::text(if creating {
                                                    "onboarding.will_be_created"
                                                } else {
                                                    "onboarding.wont_be_created"
                                                })),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("setup-starter-toggle")
                                        .debug_selector(|| "SETUP_STARTER_TOGGLE".into())
                                        .cursor_pointer()
                                        .text_size(rems(12. / 16.))
                                        .text_color(theme::accent())
                                        .on_click(move |_, _, cx| {
                                            let _ = root_toggle.update(cx, |root, cx| {
                                                let current =
                                                    root.setup.create_starter_crew != Some(false);
                                                root.setup.create_starter_crew = Some(!current);
                                                cx.notify();
                                            });
                                        })
                                        .child(crate::i18n::text(if creating {
                                            "onboarding.dont_create"
                                        } else {
                                            "onboarding.create_it"
                                        })),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_setup_footer(
        &self,
        finishing: bool,
        can_start: bool,
        reopened: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root_skip = cx.weak_entity();
        let root_start = cx.weak_entity();
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(rems(1.))
            .gap_y(rems(8. / 16.))
            .pt(rems(4. / 16.))
            .child(
                div()
                    .flex_1()
                    .min_w(rems(120. / 16.))
                    .max_w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(rems(10. / 16.))
                    .gap_y(rems(4. / 16.))
                    .child(
                        div().flex_none().child(
                            Button::new("setup-skip", crate::i18n::text("onboarding.skip_setup"))
                                .debug_selector("SETUP_SKIP")
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .disabled(finishing)
                                .on_press(move |window, cx| {
                                    let _ = root_skip.update(cx, |root, cx| {
                                        root.skip_setup(window, cx);
                                    });
                                }),
                        ),
                    )
                    .when(!reopened, |left| {
                        left.child(
                            div()
                                .min_w(px(0.))
                                .max_w_full()
                                .whitespace_normal()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::muted())
                                .child(crate::i18n::text("onboarding.reopen_hint")),
                        )
                    }),
            )
            .child(
                div().flex_none().max_w_full().child(
                    Button::new(
                        "setup-start-chat",
                        crate::i18n::text("onboarding.start_first_chat"),
                    )
                    .debug_selector("SETUP_START_CHAT")
                    .icon("corner-down-left.svg")
                    .size(ButtonSize::Md)
                    .variant(ButtonVariant::Primary)
                    .disabled(finishing || !can_start)
                    .on_press(move |window, cx| {
                        let _ = root_start.update(cx, |root, cx| {
                            root.complete_setup_and_open_launcher(window, cx);
                        });
                    }),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::start_chat::launcher_model::EntryKey;
    use gpui::{px, size, Entity, Modifiers, TestAppContext, VisualTestContext};
    use runner_core::protocol::onboarding::StarterCrewSlotProposal;
    use runner_core::protocol::runtime::{
        RuntimeExecutableStatus, RuntimeRowState, RuntimeSignInStatus, ShellDiscoveryStatus,
    };
    use runner_core::protocol::{
        DiscoveryOutcome, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask,
    };

    fn runtime_exec(
        name: Runtime,
        display_name: &str,
        state: RuntimeRowState,
        sign_in: RuntimeSignInStatus,
        installed_version: Option<&str>,
    ) -> RuntimeExecutableStatus {
        RuntimeExecutableStatus {
            launch_options: Default::default(),
            name,
            display_name: display_name.into(),
            command: name.key().into(),
            default_model: None,
            default_effort: None,
            detected_path: (state == RuntimeRowState::Detected)
                .then(|| format!("/usr/local/bin/{}", name.key())),
            override_path: None,
            effective_command: (state == RuntimeRowState::Detected).then(|| name.key().into()),
            effective_source: None,
            state,
            invalid_reason: None,
            installed_version: installed_version.map(str::to_owned),
            available_version: None,
            min_version: None,
            below_minimum: false,
            sign_in,
        }
    }

    fn runtime_status(
        npm_available: bool,
        runtimes: Vec<RuntimeExecutableStatus>,
    ) -> RuntimeStatusResponse {
        RuntimeStatusResponse {
            shell: ShellDiscoveryStatus {
                shell: Some("/bin/zsh".into()),
                outcome: Some(DiscoveryOutcome::Ok),
                duration_ms: Some(10),
                checking: false,
                using_last_known_good: false,
                last_known_good_captured_at: None,
            },
            npm_available,
            runtimes,
        }
    }

    fn test_catalog() -> Vec<RuntimeCatalogEntry> {
        runtime_metadata::runtime_list()
            .into_iter()
            .map(|meta| {
                let capabilities = RuntimeCatalogEntry::for_runtime(meta.name)
                    .map(|entry| entry.capabilities)
                    .unwrap_or_default();
                RuntimeCatalogEntry {
                    name: meta.name,
                    display_name: meta.display_name,
                    command: meta.command,
                    description: String::new(),
                    install_url: if meta.name == Runtime::Trae {
                        String::new()
                    } else {
                        format!("https://example.com/{}/install", meta.name)
                    },
                    default_enabled: true,
                    available: false,
                    native_fork: meta.native_fork,
                    default_model: None,
                    default_effort: None,
                    installed_version: None,
                    available_version: None,
                    min_version: None,
                    below_minimum: false,
                    models: Vec::new(),
                    efforts: Vec::new(),
                    capabilities,
                }
            })
            .collect()
    }

    fn install_task(runtime: Runtime, state: RuntimeUpdateState) -> RuntimeUpdateTask {
        RuntimeUpdateTask {
            id: format!("install-{runtime}"),
            runtime,
            kind: RuntimeTaskKind::Install,
            state,
            from_version: None,
            to_version: None,
            installed_version: None,
            step: String::new(),
            message: None,
            exit_code: None,
            running_sessions: 0,
        }
    }

    fn two_slot_proposal() -> StarterCrewProposal {
        StarterCrewProposal {
            name: "Pair coding".into(),
            slots: vec![
                StarterCrewSlotProposal {
                    handle: "coder".into(),
                    display_name: "Coder".into(),
                    runtime: Runtime::ClaudeCode,
                },
                StarterCrewSlotProposal {
                    handle: "reviewer".into(),
                    display_name: "Reviewer".into(),
                    runtime: Runtime::Codex,
                },
            ],
        }
    }

    fn open_test_root_with_size(
        temp: &tempfile::TempDir,
        cx: &mut TestAppContext,
        window_size: gpui::Size<gpui::Pixels>,
        init: impl FnOnce(&mut NativeRoot, &mut Context<NativeRoot>),
    ) -> (
        Entity<crate::app_store::AppStore>,
        gpui::WindowHandle<NativeRoot>,
        VisualTestContext,
    ) {
        let _ = crate::theme_snapshot::ThemeGuard::new();
        let store = crate::app_store::test_lifecycle_store(cx, temp.path());
        let handle = cx.add_window(|window, cx| {
            window.resize(window_size);
            let mut root = NativeRoot::new(
                "main".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.setup.loaded_once = true;
            root.setup.catalog = test_catalog();
            init(&mut root, cx);
            root
        });
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(window_size);
        let _ = handle.update(&mut visual, |_, window, _| window.refresh());
        visual.run_until_parked();
        (store, handle, visual)
    }

    fn open_test_root(
        temp: &tempfile::TempDir,
        cx: &mut TestAppContext,
        init: impl FnOnce(&mut NativeRoot, &mut Context<NativeRoot>),
    ) -> (
        Entity<crate::app_store::AppStore>,
        gpui::WindowHandle<NativeRoot>,
        VisualTestContext,
    ) {
        open_test_root_with_size(temp, cx, size(px(1200.), px(960.)), init)
    }

    fn wait_until(
        visual: &mut VisualTestContext,
        mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        for _ in 0..200 {
            visual.run_until_parked();
            if predicate(visual) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("condition not satisfied before timeout");
    }

    fn click_selector(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual.debug_bounds(selector).expect(selector);
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
    }

    #[test]
    fn partition_setup_runtimes_preserves_catalog_order_across_status_and_tasks() {
        let mut runtimes = vec![
            runtime_exec(
                Runtime::OpenCode,
                "OpenCode",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
            runtime_exec(
                Runtime::Copilot,
                "GitHub Copilot",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                Some("1.2.0"),
            ),
            runtime_exec(
                Runtime::Pi,
                "pi",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                Some("1.5.2"),
            ),
            runtime_exec(
                Runtime::Codex,
                "Codex",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
            runtime_exec(
                Runtime::Antigravity,
                "Antigravity",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("1.5.9"),
            ),
            runtime_exec(
                Runtime::ClaudeCode,
                "Claude Code",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("1.0.0"),
            ),
        ];
        let (initial, overflow) = partition_setup_runtimes(&runtimes);
        assert_eq!(
            initial.iter().map(|r| r.name).collect::<Vec<_>>(),
            vec![
                Runtime::ClaudeCode,
                Runtime::Codex,
                Runtime::Antigravity,
                Runtime::Pi
            ]
        );
        assert_eq!(
            overflow.iter().map(|r| r.name).collect::<Vec<_>>(),
            vec![Runtime::Copilot, Runtime::OpenCode]
        );

        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (_store, _handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
            root.setup.status = Some(runtime_status(true, runtimes.clone()));
            root.setup.proposal = Some(two_slot_proposal());
            root.agent_tasks.apply(vec![install_task(
                Runtime::OpenCode,
                RuntimeUpdateState::Running,
            )]);
        });

        // Collapsed rows keep the fixed top four in catalog order even with uninstalled Codex
        // and installed / active-task overflow runtimes.
        let collapsed_order = [
            "SETUP_AGENT_ROW_claude-code",
            "SETUP_AGENT_ROW_codex",
            "SETUP_AGENT_ROW_antigravity",
            "SETUP_AGENT_ROW_pi",
        ]
        .into_iter()
        .map(|selector| {
            visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing collapsed row {selector}"))
                .origin
                .y
        })
        .collect::<Vec<_>>();
        assert!(collapsed_order.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(visual.debug_bounds("SETUP_INSTALL_codex").is_some());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_none());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_opencode").is_none());

        click_selector(&mut visual, "SETUP_MORE_AGENTS");
        let expanded_order = [
            "SETUP_AGENT_ROW_claude-code",
            "SETUP_AGENT_ROW_codex",
            "SETUP_AGENT_ROW_antigravity",
            "SETUP_AGENT_ROW_pi",
            "SETUP_AGENT_ROW_copilot",
            "SETUP_AGENT_ROW_opencode",
        ]
        .into_iter()
        .map(|selector| {
            visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing expanded row {selector}"))
                .origin
                .y
        })
        .collect::<Vec<_>>();
        assert!(expanded_order.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(visual.debug_bounds("SETUP_INSTALL_STOP_opencode").is_some());

        for runtime in &mut runtimes {
            runtime.state = RuntimeRowState::Detected;
        }
        let (all_installed_initial, all_installed_overflow) = partition_setup_runtimes(&runtimes);
        assert_eq!(
            all_installed_initial
                .iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            vec![
                Runtime::ClaudeCode,
                Runtime::Codex,
                Runtime::Antigravity,
                Runtime::Pi
            ]
        );
        assert_eq!(
            all_installed_overflow
                .iter()
                .map(|r| r.name)
                .collect::<Vec<_>>(),
            vec![Runtime::Copilot, Runtime::OpenCode]
        );
    }

    #[test]
    fn setup_visible_respects_onboarding_split_remote_and_update_retry() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(two_slot_proposal());
        });

        assert!(handle
            .read_with(&visual, |root, cx| root.setup_visible(cx))
            .unwrap());
        assert!(visual.debug_bounds("SETUP_SURFACE").is_some());

        // Active update-wait gate takes precedence over incomplete onboarding.
        handle
            .update(&mut visual, |root, window, cx| {
                root.open_launcher_page(
                    ProjectScope::Root,
                    super::super::launcher::Step::Start,
                    false,
                    window,
                    cx,
                );
                let launcher = root.render_launcher(None, true, window, cx);
                launcher.update(cx, |launcher, cx| {
                    launcher.set_update_retry_for_test(Runtime::Codex, cx);
                });
                assert!(!root.setup_visible(cx));
            })
            .unwrap();

        // Completing onboarding hides Setup even on a fresh root window.
        store.update(&mut cx, |store, _| {
            runner_daemon::ops::onboarding::complete(&store.test_core).unwrap();
        });
        cx.run_until_parked();

        let temp2 = tempfile::tempdir().unwrap();
        let mut cx2 = TestAppContext::single();
        let (store2, handle2, mut visual2) = open_test_root(&temp2, &mut cx2, |root, _| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
        });
        store2.update(&mut cx2, |store, _| {
            runner_daemon::ops::onboarding::complete(&store.test_core).unwrap();
        });
        cx2.run_until_parked();
        visual2.run_until_parked();
        assert!(!handle2
            .read_with(&visual2, |root, cx| root.setup_visible(cx))
            .unwrap());
        assert!(visual2.debug_bounds("START_SURFACE").is_some());

        // Reopening setup marks setup.open without clearing onboarding_completed;
        // opening a split/tab pane still hides Setup.
        handle2
            .update(&mut visual2, |root, window, cx| {
                root.open_setup(window, cx);
                assert!(root.setup_visible(cx));
                assert!(store2.read(cx).onboarding_completed());
                root.test_open_empty_tab(window, cx);
                assert!(!root.setup_visible(cx));
            })
            .unwrap();

        // Remote targets never replace their launcher with local Setup.
        let temp_remote = tempfile::tempdir().unwrap();
        let mut cx_remote = TestAppContext::single();
        let (remote_store, _fake, _) =
            crate::surfaces::remote_client::tests::fixture(&mut cx_remote, temp_remote.path());
        let remote_window = crate::surfaces::remote_client::tests::add_window(
            &mut cx_remote,
            &remote_store,
            temp_remote.path(),
            "remote-setup",
        );
        remote_window
            .update(
                &mut cx_remote,
                crate::surfaces::remote_client::tests::choose_host,
            )
            .unwrap();
        cx_remote.run_until_parked();
        assert!(!remote_window
            .read_with(&cx_remote, |root, cx| root.setup_visible(cx))
            .unwrap());
    }

    #[test]
    fn setup_renders_agent_states_overflow_and_install_actions() {
        let make_runtimes = || {
            vec![
                runtime_exec(
                    Runtime::ClaudeCode,
                    "Claude Code",
                    RuntimeRowState::Detected,
                    RuntimeSignInStatus::SignedIn,
                    Some("2.0.1"),
                ),
                runtime_exec(
                    Runtime::Codex,
                    "Codex",
                    RuntimeRowState::Detected,
                    RuntimeSignInStatus::SignInNeeded,
                    Some("0.155.0"),
                ),
                runtime_exec(
                    Runtime::Antigravity,
                    "Antigravity",
                    RuntimeRowState::Detected,
                    RuntimeSignInStatus::Unknown,
                    Some("0.5.0"),
                ),
                runtime_exec(
                    Runtime::Pi,
                    "pi",
                    RuntimeRowState::NotFound,
                    RuntimeSignInStatus::Unknown,
                    None,
                ),
                runtime_exec(
                    Runtime::Cursor,
                    "Cursor CLI",
                    RuntimeRowState::NotFound,
                    RuntimeSignInStatus::Unknown,
                    None,
                ),
            ]
        };

        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (_store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
            root.setup.status = Some(runtime_status(true, make_runtimes()));
            root.setup.proposal = Some(two_slot_proposal());
        });

        assert!(visual.debug_bounds("SETUP_AGENTS_COUNT").is_some());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_claude-code").is_some());
        assert!(visual
            .debug_bounds("SETUP_OPEN_TERMINAL_claude-code")
            .is_none());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_codex").is_some());
        assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_codex").is_some());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_antigravity").is_some());
        assert!(visual
            .debug_bounds("SETUP_OPEN_TERMINAL_antigravity")
            .is_none());
        assert!(visual.debug_bounds("SETUP_INSTALL_pi").is_some());
        assert!(visual.debug_bounds("SETUP_COPY_pi").is_some());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_cursor").is_none());

        // Copy npm command writes expected install command to clipboard.
        click_selector(&mut visual, "SETUP_COPY_pi");
        assert_eq!(
            visual.read_from_clipboard().and_then(|item| item.text()),
            runner_core::protocol::npm_install_command(Runtime::Pi)
        );

        // Reveal overflow row for non-npm agent (Cursor) and verify Copy writes its script.
        click_selector(&mut visual, "SETUP_MORE_AGENTS");
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_cursor").is_some());
        assert!(visual.debug_bounds("SETUP_COPY_cursor").is_some());
        assert!(visual.debug_bounds("SETUP_GUIDE_cursor").is_some());
        click_selector(&mut visual, "SETUP_COPY_cursor");
        assert_eq!(
            visual
                .read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("curl https://cursor.com/install -fsS | bash")
        );

        // Running, MayNeedInput, and Failed install task states render Stop / Answer / Retry.
        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks
                    .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Running)]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_STOP_pi").is_some());

        assert_eq!(
            setup_install_task_badge_key(RuntimeUpdateState::MayNeedInput),
            "onboarding.may_need_input"
        );
        assert_eq!(
            crate::i18n::text("onboarding.may_need_input"),
            "May need input"
        );

        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks.apply(vec![install_task(
                    Runtime::Pi,
                    RuntimeUpdateState::MayNeedInput,
                )]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_ANSWER_pi").is_some());

        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks
                    .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Failed)]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_RETRY_pi").is_some());

        // Even once the executable appears on PATH (e.g. npm symlink mid-install or leftover after
        // failure), active/input-needed/failed Install tasks win until Succeeded.
        handle
            .update(&mut visual, |root, _, cx| {
                let mut detected = make_runtimes();
                let pi = detected.iter_mut().find(|r| r.name == Runtime::Pi).unwrap();
                pi.state = RuntimeRowState::Detected;
                pi.sign_in = RuntimeSignInStatus::SignInNeeded;
                pi.detected_path = Some("/usr/local/bin/pi".into());
                pi.installed_version = Some("0.9.0".into());
                root.setup.status = Some(runtime_status(true, detected));
                root.agent_tasks
                    .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Running)]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_STOP_pi").is_some());
        assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks.apply(vec![install_task(
                    Runtime::Pi,
                    RuntimeUpdateState::MayNeedInput,
                )]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_ANSWER_pi").is_some());
        assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks
                    .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Failed)]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_INSTALL_RETRY_pi").is_some());
        assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

        handle
            .update(&mut visual, |root, _, cx| {
                root.agent_tasks.apply(vec![install_task(
                    Runtime::Pi,
                    RuntimeUpdateState::Succeeded,
                )]);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_some());

        // Without npm, npm-packaged Not installed agents offer a Node.js download link.
        let temp_no_npm = tempfile::tempdir().unwrap();
        let mut cx_no_npm = TestAppContext::single();
        let (_store, _handle, mut visual_no_npm) =
            open_test_root(&temp_no_npm, &mut cx_no_npm, |root, _| {
                root.setup.status = Some(runtime_status(false, make_runtimes()));
                root.setup.proposal = Some(two_slot_proposal());
            });
        assert!(visual_no_npm.debug_bounds("SETUP_NODEJS_pi").is_some());
        assert!(visual_no_npm.debug_bounds("SETUP_INSTALL_pi").is_none());
    }

    #[test]
    fn setup_starter_crew_skip_reopen_sample_prompt_and_zero_crew_launcher() {
        // Needs-an-agent proposal hides the starter toggle.
        let temp_ineligible = tempfile::tempdir().unwrap();
        let mut cx_ineligible = TestAppContext::single();
        let (_store, _handle, mut visual_ineligible) =
            open_test_root(&temp_ineligible, &mut cx_ineligible, |root, _| {
                root.setup.status = Some(runtime_status(true, Vec::new()));
                root.setup.proposal = Some(StarterCrewProposal {
                    name: "Pair coding".into(),
                    slots: Vec::new(),
                });
            });
        assert!(visual_ineligible
            .debug_bounds("SETUP_STARTER_CREW")
            .is_some());
        assert!(visual_ineligible
            .debug_bounds("SETUP_STARTER_TOGGLE")
            .is_none());

        // Toggle Will create <-> Won't create, then Skip setup and inspect zero-crew launcher.
        let temp_skip = tempfile::tempdir().unwrap();
        let mut cx_skip = TestAppContext::single();
        let (store_skip, handle_skip, mut visual_skip) =
            open_test_root(&temp_skip, &mut cx_skip, |root, _| {
                root.setup.status = Some(runtime_status(true, Vec::new()));
                root.setup.proposal = Some(two_slot_proposal());
            });
        assert!(visual_skip.debug_bounds("SETUP_STARTER_TOGGLE").is_some());
        click_selector(&mut visual_skip, "SETUP_STARTER_TOGGLE");
        assert_eq!(
            handle_skip
                .read_with(&visual_skip, |root, _| root.setup.create_starter_crew)
                .unwrap(),
            Some(false)
        );
        click_selector(&mut visual_skip, "SETUP_STARTER_TOGGLE");
        assert_eq!(
            handle_skip
                .read_with(&visual_skip, |root, _| root.setup.create_starter_crew)
                .unwrap(),
            Some(true)
        );

        click_selector(&mut visual_skip, "SETUP_SKIP");
        wait_until(&mut visual_skip, |visual| {
            handle_skip
                .read_with(visual, |root, cx| !root.setup_visible(cx))
                .unwrap()
        });
        assert!(visual_skip.debug_bounds("START_SURFACE").is_some());
        assert!(store_skip.read_with(&visual_skip, |store, _| {
            store.onboarding_completed() && !store.has_crews()
        }));

        // Switching to Crews step on zero-crew DB shows "No crews yet", Create a crew, and Open setup.
        handle_skip
            .update(&mut visual_skip, |root, window, cx| {
                root.open_launcher_page(
                    ProjectScope::Root,
                    super::super::launcher::Step::Crews,
                    false,
                    window,
                    cx,
                );
            })
            .unwrap();
        wait_until(&mut visual_skip, |visual| {
            visual.debug_bounds("LAUNCHER_NO_CREWS").is_some()
        });
        assert!(visual_skip
            .debug_bounds("LAUNCHER_NO_CREWS_CREATE")
            .is_some());
        assert!(visual_skip
            .debug_bounds("LAUNCHER_NO_CREWS_SETUP")
            .is_some());

        // Clicking Open setup reopens Setup without clearing onboarding_completed.
        click_selector(&mut visual_skip, "LAUNCHER_NO_CREWS_SETUP");
        assert!(handle_skip
            .read_with(&visual_skip, |root, cx| root.setup_visible(cx)
                && root.setup.open)
            .unwrap());
        assert!(store_skip.read_with(&visual_skip, |store, _| {
            runner_daemon::ops::onboarding::status(&store.test_core)
                .unwrap()
                .completed
        }));

        // Start your first chat creates starter crew "Pair coding" and prefills sample prompt.
        let fake_bin = tempfile::tempdir().unwrap();
        for binary in [
            Runtime::ClaudeCode.command().unwrap(),
            Runtime::Codex.command().unwrap(),
        ] {
            #[cfg(windows)]
            let file = fake_bin.path().join(format!("{binary}.cmd"));
            #[cfg(not(windows))]
            let file = fake_bin.path().join(binary);
            std::fs::write(&file, "#!/bin/sh\necho 99.0.0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&file).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&file, perms).unwrap();
            }
        }

        let temp_start = tempfile::tempdir().unwrap();
        let mut cx_start = TestAppContext::single();
        let (store_start, handle_start, mut visual_start) =
            open_test_root(&temp_start, &mut cx_start, |root, cx| {
                root.setup.status = Some(runtime_status(
                    true,
                    vec![
                        runtime_exec(
                            Runtime::ClaudeCode,
                            "Claude Code",
                            RuntimeRowState::Detected,
                            RuntimeSignInStatus::Unknown,
                            Some("99.0.0"),
                        ),
                        runtime_exec(
                            Runtime::Codex,
                            "Codex",
                            RuntimeRowState::Detected,
                            RuntimeSignInStatus::SignedIn,
                            Some("99.0.0"),
                        ),
                    ],
                ));
                root.setup.proposal = Some(two_slot_proposal());
                root.app_store
                    .read(cx)
                    .test_core
                    .runtime_discovery
                    .write()
                    .unwrap()
                    .checking = false;
                let mut env = root
                    .app_store
                    .read(cx)
                    .test_core
                    .runtime_shell_env
                    .write()
                    .unwrap();
                env.path = Some(fake_bin.path().display().to_string());
                env.vars
                    .insert("__RUNNER_TEST_ISOLATE_PATH".into(), "1".into());
                env.vars
                    .insert("HOME".into(), fake_bin.path().display().to_string());
                env.vars
                    .insert("OPENAI_API_KEY".into(), "sk-test-token".into());
            });
        click_selector(&mut visual_start, "SETUP_START_CHAT");
        wait_until(&mut visual_start, |visual| {
            handle_start
                .read_with(visual, |root, cx| !root.setup_visible(cx))
                .unwrap()
                && visual.debug_bounds("LAUNCHER_SAMPLE_PROMPT").is_some()
        });
        let crews = store_start.read_with(&visual_start, |store, _| store.crews.clone());
        assert_eq!(crews.len(), 1);
        assert_eq!(crews[0].crew.name, "Pair coding");
        assert_eq!(crews[0].members.len(), 2);
        assert_ne!(crews[0].members[0].runtime, crews[0].members[1].runtime);

        let launcher = handle_start
            .read_with(&visual_start, |root, cx| {
                root.page_launcher(cx).cloned().unwrap()
            })
            .unwrap();
        // Folder change keeps updating unedited sample prompt; manual edit disables auto-sync.
        launcher.update(&mut visual_start, |launcher, cx| {
            launcher.set_cwd_for_test("/tmp/alpha-repo", cx);
        });
        let synced = launcher.read_with(&visual_start, |launcher, cx| {
            launcher.sample_prompt_text_for_test(cx).unwrap()
        });
        assert!(
            synced.contains("alpha-repo"),
            "expected alpha-repo in {synced:?}"
        );

        launcher.update(&mut visual_start, |launcher, cx| {
            launcher.set_sample_prompt_for_test("Custom user prompt", cx);
            launcher.set_cwd_for_test("/tmp/beta-repo", cx);
        });
        let preserved = launcher.read_with(&visual_start, |launcher, cx| {
            launcher.sample_prompt_text_for_test(cx).unwrap()
        });
        assert_eq!(preserved, "Custom user prompt");

        let chat_start = super::super::launcher::launcher_input(
            &EntryKey::Agent("codex".into()),
            None,
            Some("/tmp/beta-repo".into()),
            (80, 24),
            Some(preserved),
        );
        assert_eq!(
            chat_start.initial_prompt.as_deref(),
            Some("Custom user prompt")
        );

        click_selector(&mut visual_start, "LAUNCHER_CLEAR_SAMPLE_PROMPT");
        assert_eq!(
            launcher.read_with(&visual_start, |launcher, cx| launcher
                .sample_prompt_text_for_test(cx)),
            None
        );

        // Reopening Setup when crews already exist hides Starter crew and keeps onboarding_completed.
        let temp_reopen = tempfile::tempdir().unwrap();
        let mut cx_reopen = TestAppContext::single();
        let (store_reopen, handle_reopen, mut visual_reopen) =
            open_test_root(&temp_reopen, &mut cx_reopen, |root, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                core.runtime_discovery.write().unwrap().checking = false;
                {
                    let mut env = core.runtime_shell_env.write().unwrap();
                    env.path = Some(fake_bin.path().display().to_string());
                    env.vars
                        .insert("__RUNNER_TEST_ISOLATE_PATH".into(), "1".into());
                }
                runner_daemon::ops::onboarding::create_starter_crew(&core, Default::default())
                    .unwrap()
                    .unwrap();
                runner_daemon::ops::onboarding::complete(&core).unwrap();
                root.app_store.update(cx, |store, _| {
                    store.refresh_onboarding_inner();
                    store.refresh_crews_inner();
                });
                root.setup.open = true;
                root.setup.status = Some(runtime_status(true, Vec::new()));
                root.setup.proposal = Some(two_slot_proposal());
            });
        assert!(handle_reopen
            .read_with(&visual_reopen, |root, cx| root.setup_visible(cx))
            .unwrap());
        assert!(
            store_reopen.read_with(&visual_reopen, |s, _| s.onboarding_completed()
                && s.has_crews())
        );
        assert!(visual_reopen.debug_bounds("SETUP_SURFACE").is_some());
        assert!(visual_reopen.debug_bounds("SETUP_STARTER_CREW").is_none());
    }

    #[test]
    fn setup_start_chat_requires_resolved_detection_and_eligible_enabled_agent() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: Vec::new(),
            });
        });

        // 1. No eligible agents keeps Start your first chat disabled and ignores complete calls.
        assert!(!handle
            .read_with(&visual, |root, cx| root.setup_can_start(cx))
            .unwrap());
        click_selector(&mut visual, "SETUP_START_CHAT");
        handle
            .update(&mut visual, |root, window, cx| {
                root.complete_setup_and_open_launcher(window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(handle
            .read_with(&visual, |root, cx| root.setup_visible(cx))
            .unwrap());
        assert!(!store.read_with(&visual, |store, _| store.onboarding_completed()));

        // 2. Installed but disabled agent (and empty proposal) still cannot start.
        handle
            .update(&mut visual, |root, _, cx| {
                root.setup.status = Some(runtime_status(
                    true,
                    vec![runtime_exec(
                        Runtime::Codex,
                        "Codex",
                        RuntimeRowState::Detected,
                        RuntimeSignInStatus::SignedIn,
                        Some("0.155.0"),
                    )],
                ));
                root.setup.proposal = Some(StarterCrewProposal {
                    name: "Pair coding".into(),
                    slots: vec![StarterCrewSlotProposal {
                        handle: "coder".into(),
                        display_name: "Coder".into(),
                        runtime: Runtime::Codex,
                    }],
                });
                root.app_store.update(cx, |store, cx| {
                    store.settings.disabled_agents.insert("codex".into());
                    cx.notify();
                });
            })
            .unwrap();
        assert!(!handle
            .read_with(&visual, |root, cx| root.setup_can_start(cx))
            .unwrap());

        // 3. Re-enabling Codex enables Start, while unresolved probe states block it again.
        handle
            .update(&mut visual, |root, _, cx| {
                root.app_store.update(cx, |store, cx| {
                    store.settings.disabled_agents.clear();
                    cx.notify();
                });
                assert!(root.setup_can_start(cx));

                root.setup.loading = true;
                assert!(!root.setup_can_start(cx));
                root.setup.loading = false;

                root.setup.rechecking = true;
                assert!(!root.setup_can_start(cx));
                root.setup.rechecking = false;

                root.setup.status.as_mut().unwrap().shell.checking = true;
                assert!(!root.setup_can_start(cx));
                root.setup.status.as_mut().unwrap().shell.checking = false;

                let saved_proposal = root.setup.proposal.take();
                assert!(!root.setup_can_start(cx));
                root.setup.proposal = saved_proposal;
                assert!(root.setup_can_start(cx));
            })
            .unwrap();

        // 4. Skip setup stays available even when no eligible agent exists.
        handle
            .update(&mut visual, |root, _, cx| {
                root.setup.status = Some(runtime_status(true, Vec::new()));
                root.setup.proposal = Some(StarterCrewProposal {
                    name: "Pair coding".into(),
                    slots: Vec::new(),
                });
                assert!(!root.setup_can_start(cx));
                cx.notify();
            })
            .unwrap();
        click_selector(&mut visual, "SETUP_SKIP");
        wait_until(&mut visual, |visual| {
            handle
                .read_with(visual, |root, cx| !root.setup_visible(cx))
                .unwrap()
        });
        assert!(store.read_with(&visual, |store, _| store.onboarding_completed()));
    }

    #[test]
    fn setup_centers_vertically_without_clipping_compact_windows() {
        let populated_runtimes = vec![
            runtime_exec(
                Runtime::ClaudeCode,
                "Claude Code",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("2.1.325"),
            ),
            runtime_exec(
                Runtime::Codex,
                "Codex",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("0.169.0"),
            ),
            runtime_exec(
                Runtime::Antigravity,
                "Antigravity CLI",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("1.5.9"),
            ),
            runtime_exec(
                Runtime::Pi,
                "pi",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                Some("1.5.2"),
            ),
        ];

        let tall_temp = tempfile::tempdir().unwrap();
        let mut tall_cx = TestAppContext::single();
        let (_, _, mut tall_visual) = open_test_root_with_size(
            &tall_temp,
            &mut tall_cx,
            size(px(1200.), px(960.)),
            |root, _| {
                root.setup.status = Some(runtime_status(true, populated_runtimes.clone()));
                root.setup.proposal = Some(two_slot_proposal());
            },
        );
        let tall_surface = tall_visual.debug_bounds("SETUP_SURFACE").unwrap();
        let tall_brand = tall_visual.debug_bounds("SETUP_BRAND").unwrap();
        let tall_start_chat = tall_visual.debug_bounds("SETUP_START_CHAT").unwrap();
        let top_gap = f32::from(tall_brand.origin.y - tall_surface.origin.y);
        let bottom_gap = f32::from(tall_surface.bottom() - tall_start_chat.bottom());
        assert!(
            top_gap > 60.,
            "expected Setup content to sit vertically centered rather than top-pinned (top_gap={top_gap}, bottom_gap={bottom_gap})"
        );
        assert!(
            (top_gap - bottom_gap).abs() <= 4.,
            "expected top and bottom vertical margins to balance (top_gap={top_gap}, bottom_gap={bottom_gap})"
        );

        let short_temp = tempfile::tempdir().unwrap();
        let mut short_cx = TestAppContext::single();
        let (_, _, mut short_visual) = open_test_root_with_size(
            &short_temp,
            &mut short_cx,
            size(px(1200.), px(480.)),
            |root, _| {
                root.setup.status = Some(runtime_status(true, populated_runtimes));
                root.setup.proposal = Some(two_slot_proposal());
            },
        );
        let short_surface = short_visual.debug_bounds("SETUP_SURFACE").unwrap();
        let short_brand = short_visual.debug_bounds("SETUP_BRAND").unwrap();
        assert!(
            short_surface.size.height < px(500.),
            "expected compact viewport height < 500px (got {:?})",
            short_surface.size.height
        );
        let compact_top_gap = f32::from(short_brand.origin.y - short_surface.origin.y);
        assert!(
            compact_top_gap >= 20.,
            "expected compact viewport to keep top header unclipped (compact_top_gap={compact_top_gap})"
        );
    }

    #[test]
    fn setup_compact_scroll_clears_footer_above_background_task_pill() {
        let runtimes = vec![
            runtime_exec(
                Runtime::ClaudeCode,
                "Claude Code",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("2.1.325"),
            ),
            runtime_exec(
                Runtime::Codex,
                "Codex",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
            runtime_exec(
                Runtime::Antigravity,
                "Antigravity CLI",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("1.5.9"),
            ),
            runtime_exec(
                Runtime::Pi,
                "pi",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                Some("1.5.2"),
            ),
            runtime_exec(
                Runtime::Copilot,
                "GitHub Copilot CLI",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
            runtime_exec(
                Runtime::OpenCode,
                "OpenCode",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
        ];

        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (_store, handle, mut visual) =
            open_test_root_with_size(&temp, &mut cx, size(px(1000.), px(600.)), |root, _| {
                root.setup.status = Some(runtime_status(true, runtimes));
                root.setup.proposal = Some(two_slot_proposal());
                root.agent_tasks.apply(vec![install_task(
                    Runtime::Copilot,
                    RuntimeUpdateState::Running,
                )]);
            });

        let assert_footer_clears_pill = |visual: &mut VisualTestContext, label: &str| {
            let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
            let content = visual.debug_bounds("SETUP_CONTENT").unwrap();
            let max_offset = handle
                .read_with(visual, |root, _| root.setup.scroll.max_offset().y)
                .unwrap();
            assert!(
                max_offset > px(0.),
                "{label}: expected scrollable overflow (max_offset={max_offset:?}, surface={surface:?}, content={content:?})"
            );
            handle
                .update(visual, |root, _, cx| {
                    root.setup
                        .scroll
                        .set_offset(gpui::point(px(0.), -max_offset));
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();

            let pill = visual
                .debug_bounds("BACKGROUND_TASK_PILL")
                .expect("expected active background task pill");
            let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
            let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
            let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();

            assert!(
                    skip.bottom() <= surface.bottom() && skip.bottom() < pill.origin.y,
                    "{label}: expected Skip setup ({skip:?}) above pill ({pill:?}) and inside surface ({surface:?})"
                );
            assert!(
                    start_chat.bottom() <= surface.bottom()
                        && start_chat.bottom() + px(8.) <= pill.origin.y,
                    "{label}: expected Start your first chat ({start_chat:?}) clear above pill ({pill:?}) inside surface ({surface:?})"
                );
        };

        assert_footer_clears_pill(&mut visual, "collapsed");

        handle
            .update(&mut visual, |root, _, cx| {
                root.setup.more_agents = true;
                root.setup.scroll.set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_some());

        assert_footer_clears_pill(&mut visual, "expanded");
    }

    #[gpui::test]
    fn setup_expanded_agent_list_bounds_scroll_clips_long_commands_and_keeps_1440x900_visible(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::TempDir::new().unwrap();
        let catalog = test_catalog();
        let all_runtimes: Vec<_> = catalog
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                runtime_exec(
                    entry.name,
                    &entry.display_name,
                    if index == 0 {
                        RuntimeRowState::Detected
                    } else {
                        RuntimeRowState::NotFound
                    },
                    if index == 0 {
                        RuntimeSignInStatus::SignedIn
                    } else {
                        RuntimeSignInStatus::Unknown
                    },
                    (index == 0).then_some("2.1.4"),
                )
            })
            .collect();

        let (_store, handle, mut visual) =
            open_test_root_with_size(&temp, cx, size(px(1440.), px(900.)), |root, _| {
                root.setup.status = Some(runtime_status(true, all_runtimes));
                root.setup.catalog = catalog;
                root.setup.proposal = Some(StarterCrewProposal {
                    name: "Pair coding".into(),
                    slots: vec![
                        StarterCrewSlotProposal {
                            handle: "coder".into(),
                            display_name: "Coder".into(),
                            runtime: Runtime::Antigravity,
                        },
                        StarterCrewSlotProposal {
                            handle: "reviewer".into(),
                            display_name: "Reviewer".into(),
                            runtime: Runtime::Copilot,
                        },
                    ],
                });
            });

        assert!(visual.debug_bounds("SETUP_AGENT_ROW_pi").is_some());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_none());
        assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL").is_none());
        assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_none());

        click_selector(&mut visual, "SETUP_MORE_AGENTS");
        visual.run_until_parked();

        assert!(handle
            .read_with(&visual, |root, _| root.setup.more_agents)
            .unwrap());
        let scroll_bounds = visual
            .debug_bounds("SETUP_AGENTS_SCROLL")
            .expect("expected bounded scroll viewport when expanded");
        let scrollbar_bounds = visual
            .debug_bounds("SETUP_AGENTS_SCROLLBAR")
            .expect("expected inner scrollbar track");
        assert!(
            scroll_bounds.size.height <= px(217.),
            "expected bounded viewport <= 216px (+1px tolerance), got {scroll_bounds:?}"
        );
        assert!(
            visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_some(),
            "expected right-hand overflow hint when 9 agents exceed 216px viewport"
        );
        let more_bounds = visual.debug_bounds("SETUP_MORE_AGENTS").unwrap();
        let card_bounds = visual.debug_bounds("SETUP_AGENTS_CARD").unwrap();
        assert!(more_bounds.origin.y >= scroll_bounds.bottom());
        assert!(more_bounds.bottom() <= card_bounds.bottom());

        let (outer_max, inner_max) = handle
            .read_with(&visual, |root, _| {
                (
                    root.setup.scroll.max_offset().y,
                    root.setup.agents_scroll.max_offset().y,
                )
            })
            .unwrap();
        let brand = visual.debug_bounds("SETUP_BRAND").unwrap();
        let card = visual.debug_bounds("SETUP_AGENTS_CARD").unwrap();
        let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
        let content = visual.debug_bounds("SETUP_CONTENT").unwrap();
        let starter = visual.debug_bounds("SETUP_STARTER_CREW").unwrap();
        let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
        let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();
        assert_eq!(
            outer_max,
            px(0.),
            "expanded 9-agent card at 1440x900 must not trigger outer scroll (surface={surface:?}, content={content:?}, brand={brand:?}, card={card:?}, starter={starter:?}, start_chat={start_chat:?})"
        );
        assert!(
            content.size.height <= px(852.),
            "expected expanded 9-agent content height <= 852px so Linux/Windows custom-titlebar 1440x900 keeps outer scroll at 0px (surface={surface:?}, content={content:?})"
        );
        assert!(
            starter.origin.y >= surface.origin.y
                && starter.bottom() <= surface.bottom()
                && skip.bottom() <= surface.bottom()
                && start_chat.bottom() <= surface.bottom(),
            "expected starter crew & footer visible in 1440x900 (surface={surface:?}, starter={starter:?}, start_chat={start_chat:?})"
        );
        assert!(
            inner_max > px(100.),
            "expected inner agent list scroll overflow for 9 rows, got {inner_max:?}"
        );

        let pi_row = visual.debug_bounds("SETUP_AGENT_ROW_pi").unwrap();
        let pi_install = visual.debug_bounds("SETUP_INSTALL_pi").unwrap();
        assert!(
            pi_row.right() <= scrollbar_bounds.origin.x + px(0.5)
                && pi_install.right() <= scrollbar_bounds.origin.x + px(0.5),
            "expected pi row and Install button left of scrollbar gutter (row={pi_row:?}, install={pi_install:?}, bar={scrollbar_bounds:?})"
        );

        let trae_before = visual.debug_bounds("SETUP_AGENT_ROW_trae").unwrap();
        assert!(
            trae_before.origin.y >= scroll_bounds.bottom(),
            "expected 9th row below 216px viewport before scrolling (trae={trae_before:?}, viewport={scroll_bounds:?})"
        );

        handle
            .update(&mut visual, |root, _, cx| {
                root.setup
                    .agents_scroll
                    .set_offset(gpui::point(px(0.), -inner_max));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();

        let trae_after = visual.debug_bounds("SETUP_AGENT_ROW_trae").unwrap();
        assert!(
            trae_after.origin.y >= scroll_bounds.origin.y
                && trae_after.bottom() <= scroll_bounds.bottom() + px(1.),
            "expected 9th row scrolled cleanly into viewport (trae={trae_after:?}, viewport={scroll_bounds:?})"
        );
        let copilot_install = visual.debug_bounds("SETUP_INSTALL_copilot").unwrap();
        assert!(
            copilot_install.right() <= scrollbar_bounds.origin.x + px(0.5),
            "expected overflow row button clear of scrollbar gutter (install={copilot_install:?}, bar={scrollbar_bounds:?})"
        );

        click_selector(&mut visual, "SETUP_MORE_AGENTS");
        visual.run_until_parked();
        assert!(!handle
            .read_with(&visual, |root, _| root.setup.more_agents)
            .unwrap());
        assert_eq!(
            handle
                .read_with(&visual, |root, _| root.setup.agents_scroll.offset().y)
                .unwrap(),
            px(0.)
        );
        assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL").is_none());
        assert!(visual.debug_bounds("SETUP_AGENT_ROW_trae").is_none());

        handle
            .update(&mut visual, |root, _, cx| {
                let mut five_catalog = test_catalog();
                five_catalog.truncate(5);
                let five_runtimes: Vec<_> = five_catalog
                    .iter()
                    .map(|entry| {
                        runtime_exec(
                            entry.name,
                            &entry.display_name,
                            RuntimeRowState::NotFound,
                            RuntimeSignInStatus::Unknown,
                            None,
                        )
                    })
                    .collect();
                root.setup.status = Some(runtime_status(true, five_runtimes));
                root.setup.catalog = five_catalog;
                root.setup.more_agents = true;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let five_max = handle
            .read_with(&visual, |root, _| root.setup.agents_scroll.max_offset().y)
            .unwrap();
        assert!(
            five_max <= px(16.),
            "expected 5th peeking row to spill <= 16px below 216px viewport, got {five_max:?}"
        );
        assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_none());
    }

    #[gpui::test]
    fn setup_starter_crew_long_names_never_overlap_status_across_widths_and_states(
        cx: &mut TestAppContext,
    ) {
        let temp = tempfile::TempDir::new().unwrap();
        let catalog = test_catalog();
        let runtimes = vec![
            runtime_exec(
                Runtime::Antigravity,
                "Antigravity CLI",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("1.4.2"),
            ),
            runtime_exec(
                Runtime::Copilot,
                "GitHub Copilot CLI",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("0.1.9"),
            ),
        ];

        let (_store, handle, mut visual) =
            open_test_root_with_size(&temp, cx, size(px(1440.), px(900.)), |root, _| {
                root.setup.status = Some(runtime_status(true, runtimes));
                root.setup.catalog = catalog;
                root.setup.proposal = Some(StarterCrewProposal {
                    name: "Pair coding".into(),
                    slots: vec![
                        StarterCrewSlotProposal {
                            handle: "coder".into(),
                            display_name: "Coder".into(),
                            runtime: Runtime::Antigravity,
                        },
                        StarterCrewSlotProposal {
                            handle: "reviewer".into(),
                            display_name: "Reviewer".into(),
                            runtime: Runtime::Copilot,
                        },
                    ],
                });
            });

        let assert_layout_contained = |visual: &mut VisualTestContext, label: &str| {
            let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
            let card = visual.debug_bounds("SETUP_STARTER_CREW").unwrap();
            let slots = visual
                .debug_bounds("SETUP_STARTER_SLOTS")
                .expect("expected slots row");
            let slot_0 = visual.debug_bounds("SETUP_STARTER_SLOT_0").unwrap();
            let slot_1 = visual.debug_bounds("SETUP_STARTER_SLOT_1").unwrap();
            let status = visual
                .debug_bounds("SETUP_STARTER_STATUS")
                .expect("expected right status column");
            let toggle = visual
                .debug_bounds("SETUP_STARTER_TOGGLE")
                .expect("expected toggle action");
            let skip = visual
                .debug_bounds("SETUP_SKIP")
                .expect("expected skip button");
            let start_chat = visual
                .debug_bounds("SETUP_START_CHAT")
                .expect("expected primary start chat button");

            assert!(
                slots.right() + px(12.) <= status.origin.x,
                "{label}: slots row must stay strictly left of status column (slots={slots:?}, status={status:?})"
            );
            assert!(
                slot_0.right() + px(12.) <= status.origin.x
                    && slot_1.right() + px(12.) <= status.origin.x
                    && slot_0.right() < toggle.origin.x
                    && slot_1.right() < toggle.origin.x,
                "{label}: slot pills must stay clear of status & toggle (slot_0={slot_0:?}, slot_1={slot_1:?}, status={status:?}, toggle={toggle:?})"
            );
            assert!(
                slot_0.origin.y >= card.origin.y && slot_1.bottom() <= card.bottom(),
                "{label}: slot pills must remain inside starter crew card (card={card:?}, slot_0={slot_0:?}, slot_1={slot_1:?})"
            );
            assert!(
                skip.origin.x >= surface.origin.x
                    && skip.right() <= surface.right()
                    && start_chat.origin.x >= surface.origin.x
                    && start_chat.right() <= surface.right(),
                "{label}: footer buttons must stay horizontally inside surface (surface={surface:?}, skip={skip:?}, start_chat={start_chat:?})"
            );
            let separated_horizontally = skip.right() + px(8.) <= start_chat.origin.x
                || start_chat.right() + px(8.) <= skip.origin.x;
            let separated_vertically = skip.bottom() + px(4.) <= start_chat.origin.y
                || start_chat.bottom() + px(4.) <= skip.origin.y;
            assert!(
                separated_horizontally || separated_vertically,
                "{label}: footer buttons must not overlap (skip={skip:?}, start_chat={start_chat:?})"
            );
            (slot_0, slot_1)
        };

        assert_layout_contained(&mut visual, "1440x900 will create");
        click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
        visual.run_until_parked();
        assert_layout_contained(&mut visual, "1440x900 wont create");
        click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
        visual.run_until_parked();

        handle
            .update(&mut visual, |_, window, cx| {
                window.set_rem_size(px(20.));
                window.refresh();
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_layout_contained(&mut visual, "1440x900 zoomed 125% will create");
        click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
        visual.run_until_parked();
        assert_layout_contained(&mut visual, "1440x900 zoomed 125% wont create");

        visual.simulate_resize(size(px(900.), px(800.)));
        handle
            .update(&mut visual, |_, window, cx| {
                window.set_rem_size(px(16. * 1.3));
                window.refresh();
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_layout_contained(&mut visual, "900x800 zoomed 130% wont create");
        click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
        visual.run_until_parked();
        assert_layout_contained(&mut visual, "900x800 zoomed 130% will create");

        let max_scroll = handle
            .read_with(&visual, |root, _| root.setup.scroll.max_offset().y)
            .unwrap();
        if max_scroll > px(0.) {
            handle
                .update(&mut visual, |root, _, cx| {
                    root.setup
                        .scroll
                        .set_offset(gpui::point(px(0.), -max_scroll));
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
        }
        let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
        let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
        let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();
        assert!(
            skip.bottom() <= surface.bottom() && start_chat.bottom() <= surface.bottom(),
            "900x800 zoomed 130%: expected footer buttons vertically reachable in surface (surface={surface:?}, skip={skip:?}, start_chat={start_chat:?})"
        );

        visual.simulate_resize(size(px(720.), px(800.)));
        handle
            .update(&mut visual, |root, window, cx| {
                root.setup.scroll.set_offset(gpui::point(px(0.), px(0.)));
                window.set_rem_size(px(16.));
                window.refresh();
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let (narrow_will_0, narrow_will_1) =
            assert_layout_contained(&mut visual, "720x800 narrow will create");
        assert!(
            narrow_will_1.origin.y >= narrow_will_0.bottom(),
            "expected wrapped slots in Will be created at narrow width (slot_0={narrow_will_0:?}, slot_1={narrow_will_1:?})"
        );
        click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
        visual.run_until_parked();
        let (narrow_0, narrow_1) =
            assert_layout_contained(&mut visual, "720x800 narrow wont create");
        assert!(
            narrow_1.origin.y >= narrow_0.bottom(),
            "expected second long slot pill to wrap cleanly at narrow width (slot_0={narrow_0:?}, slot_1={narrow_1:?})"
        );
    }
}
