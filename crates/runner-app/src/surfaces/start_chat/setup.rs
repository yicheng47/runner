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
#[path = "setup_tests.rs"]
mod tests;
