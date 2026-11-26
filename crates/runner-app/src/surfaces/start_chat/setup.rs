use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, svg, AnyElement, App, ClipboardItem, Context, Entity, FontWeight,
    SharedString, Task, WeakEntity, Window,
};
use runner_app::ui::button::spinner;
use runner_app::ui::{
    app_scrollbar_gutter, Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize,
    RoleAvatar, Scrollbar,
};
use runner_core::protocol::onboarding::{
    CreateStarterCrewArgs, OnboardingStatus, StarterCrewProposal,
};
use runner_core::protocol::project::ProjectScope;
use runner_core::protocol::runtime::{
    npm_install_command, RuntimeCatalogEntry, RuntimeExecutableStatus, RuntimeRowState,
    RuntimeSignInStatus, RuntimeStatusResponse, NODEJS_DOWNLOAD_URL,
};
use runner_core::protocol::{
    runtime_metadata, Runtime, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask,
};

use crate::app_settings::AppSettings;
use crate::app_store::AppStore;
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

fn enabled_chat_runtimes(settings: &AppSettings) -> Vec<Runtime> {
    Runtime::SETUP_ORDER
        .into_iter()
        .filter(|runtime| {
            runtime_metadata::catalog_for(*runtime)
                .is_some_and(|meta| settings.is_agent_enabled(*runtime, meta.default_enabled))
        })
        .collect()
}

fn starter_crew_args(settings: &AppSettings) -> CreateStarterCrewArgs {
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
                    root.setup.copy_command(name, cmd, cx);
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

fn render_setup_footer(
    finishing: bool,
    can_start: bool,
    reopened: bool,
    root_weak: WeakEntity<NativeRoot>,
) -> AnyElement {
    let root_skip = root_weak.clone();
    let root_start = root_weak;
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
                                    root.finish_setup(false, window, cx);
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
                        root.finish_setup(true, window, cx);
                    });
                }),
            ),
        )
        .into_any_element()
}

impl SetupState {
    pub(crate) fn is_visible(
        &self,
        page_has_update_retry: bool,
        onboarding_completed: bool,
    ) -> bool {
        if page_has_update_retry {
            return false;
        }
        self.open || !onboarding_completed
    }

    pub(crate) fn open_reset(&mut self) {
        self.open = true;
        self.create_starter_crew = None;
        self.more_agents = false;
        self.finishing = false;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.agents_scroll.set_offset(gpui::point(px(0.), px(0.)));
    }

    pub(crate) fn expand_more_agents(&mut self) {
        self.more_agents = true;
    }

    pub(crate) fn collapse_more_agents(&mut self) {
        self.more_agents = false;
        self.agents_scroll.set_offset(gpui::point(px(0.), px(0.)));
    }

    pub(crate) fn toggle_starter_crew(&mut self) {
        let current = self.create_starter_crew != Some(false);
        self.create_starter_crew = Some(!current);
    }

    pub(crate) fn can_start(&self, settings: &AppSettings) -> bool {
        if !self.loaded_once || self.loading || self.rechecking {
            return false;
        }
        let Some(status) = self.status.as_ref() else {
            return false;
        };
        if status.shell.checking {
            return false;
        }
        let Some(proposal) = self.proposal.as_ref() else {
            return false;
        };
        if proposal.slots.is_empty() {
            return false;
        }
        let enabled = enabled_chat_runtimes(settings);
        status.runtimes.iter().any(|runtime| {
            setup_row_installed(runtime)
                && runtime.name != Runtime::Shell
                && enabled.contains(&runtime.name)
        })
    }

    fn apply_probe(
        &mut self,
        status: Option<RuntimeStatusResponse>,
        catalog: Vec<RuntimeCatalogEntry>,
        proposal: Option<StarterCrewProposal>,
    ) {
        if status.is_some() {
            self.status = status;
        }
        if !catalog.is_empty() {
            self.catalog = catalog;
        }
        if proposal.is_some() {
            self.proposal = proposal;
        }
    }

    fn apply_refresh_result(
        &mut self,
        generation: u64,
        status: Option<RuntimeStatusResponse>,
        catalog: Vec<RuntimeCatalogEntry>,
        proposal: Option<StarterCrewProposal>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.loading = false;
        self.apply_probe(status, catalog, proposal);
        true
    }

    fn apply_recheck_result(
        &mut self,
        generation: u64,
        status: Option<RuntimeStatusResponse>,
        catalog: Vec<RuntimeCatalogEntry>,
        proposal: Option<StarterCrewProposal>,
    ) -> bool {
        self.rechecking = false;
        if self.generation != generation {
            return false;
        }
        self.apply_probe(status, catalog, proposal);
        true
    }

    pub(crate) fn refresh(&mut self, store: &Entity<AppStore>, cx: &mut Context<NativeRoot>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.loaded_once = true;
        let (core, args) = {
            let store = store.read(cx);
            (store.client.clone(), starter_crew_args(&store.settings))
        };
        let task = cx.background_spawn(async move {
            let status = core.runtime_status_list().ok();
            let catalog = core.runtime_catalog().unwrap_or_default();
            let proposal = core.onboarding_starter_crew_proposal(args).ok();
            (status, catalog, proposal)
        });
        cx.spawn(async move |weak, cx| {
            let (status, catalog, proposal) = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root
                    .setup
                    .apply_refresh_result(generation, status, catalog, proposal)
                {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn recheck(&mut self, store: &Entity<AppStore>, cx: &mut Context<NativeRoot>) {
        if self.rechecking {
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        self.rechecking = true;
        let (core, model_runtimes, args) = {
            let store = store.read(cx);
            (
                store.client.clone(),
                store.settings.model_runtimes(),
                starter_crew_args(&store.settings),
            )
        };
        let task = cx.background_spawn(async move {
            let status = core.runtime_refresh(&model_runtimes).ok();
            let catalog = core.runtime_catalog().unwrap_or_default();
            let proposal = core.onboarding_starter_crew_proposal(args).ok();
            (status, catalog, proposal)
        });
        cx.spawn(async move |weak, cx| {
            let (status, catalog, proposal) = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root
                    .setup
                    .apply_recheck_result(generation, status, catalog, proposal)
                {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn copy_command(
        &mut self,
        runtime: Runtime,
        command: String,
        cx: &mut Context<NativeRoot>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(command));
        self.copied_runtime = Some(runtime);
        self.copied_generation += 1;
        let generation = self.copied_generation;
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

    pub(crate) fn begin_finish(
        &mut self,
        start_first_chat: bool,
        has_crews: bool,
        already_completed: bool,
        store: &Entity<AppStore>,
        cx: &mut Context<NativeRoot>,
    ) -> Option<(bool, Task<anyhow::Result<Option<OnboardingStatus>>>)> {
        if self.finishing {
            return None;
        }
        let (core, args, can_start) = {
            let store = store.read(cx);
            (
                store.client.clone(),
                starter_crew_args(&store.settings),
                self.can_start(&store.settings),
            )
        };
        if start_first_chat && !can_start {
            return None;
        }
        let proposal_has_slots = self
            .proposal
            .as_ref()
            .is_some_and(|proposal| !proposal.slots.is_empty());
        let create_crew = start_first_chat
            && !has_crews
            && proposal_has_slots
            && self.create_starter_crew != Some(false);
        self.finishing = true;
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
        Some((create_crew, work))
    }

    pub(crate) fn complete_finish(&mut self) {
        self.finishing = false;
        self.open = false;
        self.create_starter_crew = None;
        self.more_agents = false;
        self.agents_scroll.set_offset(gpui::point(px(0.), px(0.)));
    }

    pub(crate) fn fail_finish(&mut self) {
        self.finishing = false;
    }

    pub(crate) fn render(
        &mut self,
        store: &Entity<AppStore>,
        tasks: &[RuntimeUpdateTask],
        task_pill_visible: bool,
        cx: &mut Context<NativeRoot>,
    ) -> AnyElement {
        if !self.loaded_once && !self.loading {
            self.refresh(store, cx);
        }
        let (has_crews, can_start) = {
            let store = store.read(cx);
            (store.has_crews(), self.can_start(&store.settings))
        };
        let status = self.status.as_ref();
        let npm_available = status.is_some_and(|s| s.npm_available);
        let checking = self.rechecking
            || status.is_some_and(|s| s.shell.checking)
            || (self.loading && status.is_none());
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
        let agent_card = self.render_agents_card(
            &primary,
            &overflow,
            runtimes.is_empty(),
            npm_available,
            checking,
            tasks,
            cx,
        );

        let show_starter_crew = !(self.open && has_crews);
        let recheck_disabled = checking;
        let finishing = self.finishing;
        let reopened = self.open;
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
                        root.finish_setup(false, window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id("setup-surface")
                    .w_full()
                    .max_h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
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
                                                                    if !root
                                                                        .remote_client
                                                                        .is_active()
                                                                    {
                                                                        root.setup.recheck(
                                                                            &root.app_store,
                                                                            cx,
                                                                        );
                                                                    }
                                                                },
                                                            );
                                                        }),
                                                    ),
                                            )
                                            .child(agent_card),
                                    )
                                    .children(
                                        show_starter_crew
                                            .then(|| self.render_starter_crew(cx.weak_entity())),
                                    )
                                    .child(render_setup_footer(
                                        finishing,
                                        can_start,
                                        reopened,
                                        cx.weak_entity(),
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_agents_card(
        &mut self,
        primary: &[RuntimeExecutableStatus],
        overflow: &[RuntimeExecutableStatus],
        runtimes_empty: bool,
        npm_available: bool,
        checking: bool,
        tasks: &[RuntimeUpdateTask],
        cx: &mut Context<NativeRoot>,
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

        let root_weak = cx.weak_entity();
        let overflow_count = overflow.len();
        if self.more_agents && overflow_count > 0 {
            let rows: Vec<_> = primary
                .iter()
                .chain(overflow.iter())
                .map(|runtime| {
                    let task = tasks.iter().find(|task| task.runtime == runtime.name);
                    self.render_agent_row(runtime, npm_available, task, root_weak.clone())
                })
                .collect();
            let total_rows = primary.len() + overflow_count;
            let owner = cx.entity_id();
            let agents_scroll = self.agents_scroll.clone();
            let scrollbar = self
                .agents_scrollbar
                .get_or_insert_with(|| cx.new(|_| Scrollbar::app(agents_scroll, owner)))
                .clone();
            let measured = self.agents_scroll.clone();
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
                                .track_scroll(&self.agents_scroll)
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
                            root.setup.collapse_more_agents();
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

        let mut collapsed = card
            .p(rems(4. / 16.))
            .children(primary.iter().map(|runtime| {
                let task = tasks.iter().find(|task| task.runtime == runtime.name);
                self.render_agent_row(runtime, npm_available, task, root_weak.clone())
            }));
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
                            root.setup.expand_more_agents();
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

    fn render_agent_row(
        &self,
        runtime: &RuntimeExecutableStatus,
        npm_available: bool,
        task: Option<&RuntimeUpdateTask>,
        root_weak: WeakEntity<NativeRoot>,
    ) -> AnyElement {
        let name = runtime.name;
        let installed = setup_row_installed(runtime);
        let catalog = self.catalog.iter().find(|entry| entry.name == name);
        let npm_cmd = npm_install_command(name);
        let copy_cmd = copy_text_for(runtime, catalog, npm_available);
        let copied = self.copied_runtime == Some(name);

        let install_task = active_setup_install_task(task, installed);
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
                                            let core = root.core(cx).clone();
                                            cx.background_spawn(async move {
                                                let _ = core.runtime_task_stop(&task_id);
                                            })
                                            .detach();
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
                                let home = runner_core::app_paths::home_dir()
                                    .map(|path| path.to_string_lossy().into_owned());
                                root.new_terminal_tab_in(ProjectScope::Root, home, window, cx);
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
            if let Some(url) = catalog
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
            let detail = manual_install_detail(name, catalog);
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

    fn render_starter_crew(&self, root_toggle: WeakEntity<NativeRoot>) -> AnyElement {
        let proposal = self.proposal.as_ref();
        let slots = proposal.map(|p| p.slots.as_slice()).unwrap_or(&[]);
        let eligible = slots.len() >= 2;
        let creating = eligible && self.create_starter_crew != Some(false);
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
                                                root.setup.toggle_starter_crew();
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
}

impl NativeRoot {
    pub(crate) fn setup_visible(&self, cx: &App) -> bool {
        if self.remote_client.is_active() || self.front_page_layout().is_some() {
            return false;
        }
        let target = self.front_page_client(cx).target;
        self.setup.is_visible(
            self.launchers.page_has_update_retry(&target, cx),
            self.app_store.read(cx).onboarding_completed(),
        )
    }

    pub(crate) fn open_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        self.setup.open_reset();
        self.set_route(AppRoute::Launcher, cx);
        self.setup.refresh(&self.app_store, cx);
        self.chat_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn finish_setup(
        &mut self,
        start_first_chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }
        let (has_crews, already_completed) = {
            let store = self.app_store.read(cx);
            (store.has_crews(), store.onboarding_completed())
        };
        let Some((create_crew, work)) = self.setup.begin_finish(
            start_first_chat,
            has_crews,
            already_completed,
            &self.app_store,
            cx,
        ) else {
            return;
        };
        cx.spawn_in(window, async move |weak, cx| {
            let result = work.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                match result {
                    Ok(status) => {
                        root.setup.complete_finish();
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
                        root.setup.fail_finish();
                        root.error = Some(error.to_string().into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
