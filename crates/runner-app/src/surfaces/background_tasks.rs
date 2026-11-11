use gpui::prelude::*;
use gpui::{div, px, Animation, AnimationExt, AnyElement, Context, SharedString, Window};
use runner_app::ui::{Button, ButtonSize, ButtonVariant};
use runner_core::protocol::{Runtime, RuntimeUpdateState as State, RuntimeUpdateTask};
use std::collections::{HashMap, HashSet};

use crate::i18n::{text, tf};
use crate::{theme, NativeRoot};

#[derive(Default)]
pub(crate) struct BackgroundTasks {
    pub(crate) remote_polling: HashSet<(super::front_page::FrontPageTarget, Runtime)>,
    pub(crate) remote_tasks:
        HashMap<(super::front_page::FrontPageTarget, Runtime), RuntimeUpdateTask>,
    pub(crate) gates:
        HashMap<(super::front_page::FrontPageTarget, String), super::agent_recovery::SessionGate>,
    pub(crate) minimums: HashMap<Runtime, runner_core::protocol::RuntimeVersionGate>,
    pub(crate) waiting_sessions: HashMap<
        (super::front_page::FrontPageTarget, String),
        super::agent_recovery::WaitingSession,
    >,
    pub(crate) tasks: Vec<RuntimeUpdateTask>,
    pub(crate) hidden: bool,
    pub(crate) open: bool,
}

impl BackgroundTasks {
    pub(crate) fn apply(&mut self, tasks: Vec<RuntimeUpdateTask>) {
        if tasks.iter().any(|task| {
            task.state.priority() < 2
                && self
                    .tasks
                    .iter()
                    .find(|old| old.id == task.id)
                    .is_none_or(|old| old.state != task.state)
        }) {
            self.hidden = false;
        }
        if tasks.is_empty() {
            self.hidden = false;
            self.open = false;
        }
        self.tasks = tasks;
    }

    pub(crate) fn task(&self, runtime: Runtime) -> Option<&RuntimeUpdateTask> {
        self.tasks.iter().find(|task| task.runtime == runtime)
    }
}

pub(crate) fn label(task: &RuntimeUpdateTask) -> String {
    let name = task.runtime.display_name();
    match task.state {
        State::MayNeedInput => tf("agent_updates.may_need_input", &[("runtime", name.into())]),
        State::Failed => tf("agent_updates.failed", &[("runtime", name.into())]),
        State::BrokenAfterStop => tf("agent_updates.broken", &[("runtime", name.into())]),
        State::Succeeded => task.installed_version.as_ref().map_or_else(
            || tf("agent_updates.updated", &[("runtime", name.into())]),
            |version| {
                tf(
                    "agent_updates.updated_to",
                    &[("runtime", name.into()), ("version", version.clone())],
                )
            },
        ),
        _ => tf("agent_updates.updating", &[("runtime", name.into())]),
    }
}

fn progress() -> impl IntoElement {
    div()
        .debug_selector(|| "BACKGROUND_TASK_PROGRESS".into())
        .h(px(2.))
        .w_full()
        .rounded_full()
        .bg(theme::border())
        .overflow_hidden()
        .with_animation(
            "agent-task-progress",
            Animation::new(std::time::Duration::from_millis(1100)).repeat(),
            |track, delta| {
                track.child(
                    div()
                        .h_full()
                        .w(px(70.))
                        .rounded_full()
                        .bg(theme::accent())
                        .ml(px(delta * 300. - 70.)),
                )
            },
        )
}

impl NativeRoot {
    pub(crate) fn target_task(
        &self,
        target: &super::front_page::FrontPageTarget,
        runtime: Runtime,
    ) -> Option<&RuntimeUpdateTask> {
        match target {
            super::front_page::FrontPageTarget::Local => self.agent_tasks.task(runtime),
            target => self
                .agent_tasks
                .remote_tasks
                .get(&(target.clone(), runtime)),
        }
    }

    pub(crate) fn start_target_update(
        &mut self,
        client: super::front_page::FrontPageClient,
        runtime: Runtime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !client.is_remote() {
            self.start_agent_update(runtime, cx);
            return;
        }
        let target = client.target.clone();
        if !self
            .agent_tasks
            .remote_polling
            .insert((target.clone(), runtime))
        {
            return;
        }
        let starter = client.clone();
        let work = cx.background_spawn(async move { starter.start_runtime_task(runtime) });
        cx.spawn_in(window, async move |owner, cx| {
            let mut result = work.await.map(Some);
            loop {
                let task = match result {
                    Ok(Some(task)) => task,
                    Ok(None) => {
                        let _ = owner.update(cx, |root, cx| {
                            root.agent_tasks
                                .remote_tasks
                                .remove(&(target.clone(), runtime));
                            root.refresh_remote_agent_versions(client.clone(), cx);
                            root.notify_update_launchers(cx);
                            cx.notify();
                        });
                        break;
                    }
                    Err(error) => {
                        let _ = owner.update(cx, |root, cx| {
                            root.agent_tasks
                                .remote_tasks
                                .remove(&(target.clone(), runtime));
                            root.chat_error = Some(error.message.into());
                            root.notify_update_launchers(cx);
                            cx.notify();
                        });
                        break;
                    }
                };
                let active = task.state.active();
                let succeeded = task.state == State::Succeeded;
                if owner
                    .update_in(cx, |root, window, cx| {
                        root.agent_tasks
                            .remote_tasks
                            .insert((target.clone(), runtime), task.clone());
                        if let Some(dialog) = &root.agent_update {
                            let id = match &target {
                                super::front_page::FrontPageTarget::Remote { computer, .. } => {
                                    runner_core::remote::terminal_address(computer, &task.id)
                                }
                                _ => task.id.clone(),
                            };
                            dialog.update(cx, |dialog, cx| {
                                if dialog.task_id() == id {
                                    dialog.apply_task(&task, cx);
                                }
                            });
                        }
                        if succeeded {
                            root.refresh_remote_agent_versions(client.clone(), cx);
                            root.retry_gated_launches(runtime, &target, window, cx);
                            root.retry_remote_gated_sessions(runtime, &target, window, cx);
                        }
                        root.notify_update_launchers(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if !active || !client.valid() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let polling = client.clone();
                result = cx
                    .background_spawn(async move { polling.runtime_task(runtime) })
                    .await;
            }
            let _ = owner.update(cx, |root, _| {
                root.agent_tasks.remote_polling.remove(&(target, runtime));
            });
        })
        .detach();
    }

    pub(crate) fn start_agent_update(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        let client = self.core(cx).clone();
        let work = cx.background_spawn(async move { client.runtime_task_start(runtime) });
        cx.spawn(async move |root, cx| {
            let result = work.await;
            let _ = root.update(cx, |root, cx| {
                if let Err(error) = result {
                    root.error = Some(error.to_string().into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop_or_dismiss_task(&mut self, task: RuntimeUpdateTask, cx: &mut Context<Self>) {
        let client = self.core(cx).clone();
        let work = cx.background_spawn(async move {
            if task.state.active() {
                client.runtime_task_stop(&task.id)
            } else {
                client.runtime_task_dismiss(&task.id)
            }
        });
        cx.spawn(async move |root, cx| {
            let result = work.await;
            let _ = root.update(cx, |root, cx| {
                if let Err(error) = result {
                    root.error = Some(error.to_string().into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_agent_tasks(
        &mut self,
        tasks: Vec<RuntimeUpdateTask>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let completed = tasks
            .iter()
            .filter(|task| {
                task.state == State::Succeeded
                    && !self
                        .agent_tasks
                        .tasks
                        .iter()
                        .any(|old| old.id == task.id && old.state == State::Succeeded)
            })
            .map(|task| task.runtime)
            .collect::<Vec<_>>();
        self.agent_tasks.apply(tasks);
        if let Some(dialog) = &self.agent_update {
            dialog.update(cx, |dialog, cx| {
                if let Some(task) = self
                    .agent_tasks
                    .tasks
                    .iter()
                    .find(|task| task.id == dialog.task_id())
                {
                    dialog.apply_task(task, cx);
                }
            });
        }
        self.refresh_agents_tasks(&self.agent_tasks.tasks, cx);
        self.notify_update_launchers(cx);
        for runtime in completed {
            self.retry_gated_launches(
                runtime,
                &super::front_page::FrontPageTarget::Local,
                window,
                cx,
            );
            self.retry_gated_sessions(runtime, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn render_background_tasks(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.agent_tasks.hidden || self.agent_tasks.tasks.is_empty() {
            return None;
        }
        let mut tasks = self.agent_tasks.tasks.clone();
        tasks.sort_by_key(|task| task.state.priority());
        let urgent = &tasks[0];
        let tone = match urgent.state {
            State::MayNeedInput => theme::warning(),
            State::Failed | State::BrokenAfterStop => theme::danger(),
            State::Succeeded => theme::accent(),
            _ => theme::text(),
        };
        let owner = cx.weak_entity();
        let hide = cx.weak_entity();
        let pill = div()
            .id("background-task-pill")
            .debug_selector(|| "BACKGROUND_TASK_PILL".into())
            .occlude()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow_lg()
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |root, cx| {
                    root.agent_tasks.open = !root.agent_tasks.open;
                    cx.notify();
                });
            })
            .child(match urgent.state {
                State::Running | State::Verifying | State::Stopping => {
                    runner_app::ui::button::spinner("background-task-spinner", 14., tone)
                        .into_any_element()
                }
                state => gpui::svg()
                    .debug_selector(move || format!("BACKGROUND_TASK_ICON:{state:?}"))
                    .path(match state {
                        State::MayNeedInput => "keyboard.svg",
                        State::Succeeded => "circle-check.svg",
                        _ => "circle-x.svg",
                    })
                    .size(px(14.))
                    .text_color(tone)
                    .into_any_element(),
            })
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(tone)
                    .child(label(urgent)),
            )
            .children(
                matches!(urgent.state, State::Running | State::Verifying)
                    .then(|| div().w(px(45.)).child(progress())),
            )
            .children((tasks.len() > 1).then(|| {
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .debug_selector(|| "BACKGROUND_TASK_COUNT".into())
                    .child(format!("+{}", tasks.len() - 1))
            }))
            .child(
                Button::new("background-task-hide", "×")
                    .debug_selector("BACKGROUND_TASK_HIDE")
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |_, cx| {
                        let _ = hide.update(cx, |root, cx| {
                            root.agent_tasks.hidden = true;
                            root.agent_tasks.open = false;
                            cx.notify();
                        });
                    }),
            );
        let mut surface = div()
            .absolute()
            .right_4()
            .bottom_4()
            .flex()
            .flex_col()
            .items_end()
            .gap_2();
        if self.agent_tasks.open {
            let mut list = div()
                .id("background-task-list")
                .debug_selector(|| "BACKGROUND_TASK_LIST".into())
                .occlude()
                .w(px(360.))
                .max_h(px(480.))
                .overflow_y_scroll()
                .p_3()
                .flex()
                .flex_col()
                .gap_3()
                .rounded_lg()
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .shadow_lg()
                .child(
                    div()
                        .text_size(theme::text_ui())
                        .child(text("agent_updates.title")),
                );
            for task in tasks {
                let runtime = task.runtime;
                let output = task.clone();
                let close_task = task.clone();
                let owner = cx.weak_entity();
                let closing = cx.weak_entity();
                let retry = cx.weak_entity();
                let mut row = div()
                    .debug_selector(|| format!("BACKGROUND_TASK_{}", task.runtime))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(match task.state {
                                        State::MayNeedInput => theme::warning(),
                                        State::Failed | State::BrokenAfterStop => theme::danger(),
                                        _ => theme::text(),
                                    })
                                    .child(label(&task)),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("task-close-{}", task.id)),
                                    "×",
                                )
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_press(move |_, cx| {
                                    let _ = closing.update(cx, |root, cx| {
                                        root.stop_or_dismiss_task(close_task.clone(), cx)
                                    });
                                }),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(format!(
                                "{} → {}",
                                task.from_version
                                    .as_deref()
                                    .unwrap_or(text("agent_updates.unknown")),
                                task.to_version
                                    .as_deref()
                                    .unwrap_or(text("agent_updates.latest"))
                            )),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .whitespace_normal()
                            .child(match task.state {
                                State::MayNeedInput if task.step.is_empty() => {
                                    text("agent_updates.input_may").into()
                                }
                                State::Succeeded if task.running_sessions > 0 => tf(
                                    "agent_updates.running_keep",
                                    &[
                                        ("runtime", runtime.display_name().into()),
                                        (
                                            "version",
                                            task.from_version.clone().unwrap_or_else(|| {
                                                text("agent_updates.current_version").into()
                                            }),
                                        ),
                                    ],
                                ),
                                State::Succeeded => text("agent_updates.finished").into(),
                                _ => task.message.clone().unwrap_or_else(|| task.step.clone()),
                            }),
                    );
                if matches!(task.state, State::Running | State::Verifying) {
                    row = row.child(progress());
                }
                let mut actions = div().flex().gap_2();
                if matches!(task.state, State::Failed | State::BrokenAfterStop) {
                    actions = actions.child(
                        Button::new(
                            SharedString::from(format!("task-retry-{}", task.id)),
                            if task.state == State::BrokenAfterStop {
                                text("agent_updates.reinstall")
                            } else {
                                text("ui.retry")
                            },
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .on_press(move |_, cx| {
                            let _ =
                                retry.update(cx, |root, cx| root.start_agent_update(runtime, cx));
                        }),
                    );
                }
                actions = actions.child(
                    Button::new(
                        SharedString::from(format!("task-output-{}", task.id)),
                        if task.state == State::MayNeedInput {
                            text("agent_updates.answer")
                        } else {
                            text("agent_updates.show_output")
                        },
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |window, cx| {
                        let _ = owner.update(cx, |root, cx| {
                            root.show_agent_task(output.clone(), window, cx)
                        });
                    }),
                );
                list = list.child(row.child(actions));
            }
            surface = surface.child(list);
        }
        Some(surface.child(pill).into_any_element())
    }

    pub(crate) fn show_agent_task(
        &mut self,
        task: RuntimeUpdateTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_target_task(task, None, window, cx);
    }

    pub(crate) fn show_target_task(
        &mut self,
        task: RuntimeUpdateTask,
        client: Option<super::front_page::FrontPageClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote_terminal = client.and_then(|client| client.task_terminal(&task.id));
        let request = super::agent_update::AgentUpdateRequest {
            task_id: remote_terminal
                .as_ref()
                .map(|(id, _, _)| id.clone())
                .unwrap_or_else(|| task.id.clone()),
            terminal: remote_terminal.map(|(_, client, lifetime)| (client, lifetime)),
            runtime: task.runtime,
            display_name: task.runtime.display_name().into(),
            command: task.runtime.command().unwrap_or_default().into(),
            installed: task
                .from_version
                .clone()
                .unwrap_or_else(|| text("agent_updates.unknown").into()),
            available: task
                .to_version
                .clone()
                .unwrap_or_else(|| text("agent_updates.latest").into()),
        };
        if self.agent_update.is_some() {
            self.close_agent_update(window, cx);
        }
        self.open_agent_update(request, window, cx);
        if let Some(dialog) = &self.agent_update {
            dialog.update(cx, |dialog, cx| dialog.apply_task(&task, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(runtime: Runtime, state: State) -> RuntimeUpdateTask {
        RuntimeUpdateTask {
            id: runtime.key().into(),
            runtime,
            state,
            from_version: Some("0.166.0".into()),
            to_version: Some("0.167.0".into()),
            step: "Installing packages…".into(),
            message: None,
            exit_code: None,
            installed_version: None,
            running_sessions: 0,
        }
    }

    #[test]
    fn pill_priority_and_hidden_attention_follow_task_transitions() {
        let mut tasks = BackgroundTasks::default();
        let running = task(Runtime::Codex, State::Running);
        let mut input = task(Runtime::Pi, State::MayNeedInput);
        let failed = task(Runtime::ClaudeCode, State::Failed);
        tasks.apply(vec![running.clone(), failed.clone(), input.clone()]);
        assert_eq!(
            tasks
                .tasks
                .iter()
                .min_by_key(|task| task.state.priority())
                .unwrap()
                .runtime,
            Runtime::Pi
        );
        tasks.hidden = true;
        tasks.apply(vec![running.clone(), failed.clone(), input.clone()]);
        assert!(
            tasks.hidden,
            "repeated output from an already acknowledged task must stay hidden"
        );
        input.state = State::Running;
        tasks.apply(vec![running.clone(), failed.clone(), input.clone()]);
        assert!(tasks.hidden);
        input.state = State::MayNeedInput;
        tasks.apply(vec![running.clone(), failed, input]);
        assert!(!tasks.hidden);
        tasks.hidden = true;
        let mut success = running.clone();
        success.state = State::Succeeded;
        tasks.apply(vec![success]);
        assert!(tasks.hidden, "success never reveals a hidden pill");
        tasks.apply(vec![task(Runtime::Codex, State::Failed)]);
        assert!(!tasks.hidden, "a new failure reveals it");
        tasks.apply(Vec::new());
        assert!(!tasks.open);
        assert!(!tasks.hidden);
    }

    #[test]
    fn task_ui_keeps_focus_and_only_opens_output_on_request() {
        use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
        let mut catalog =
            runner_core::protocol::RuntimeCatalogEntry::for_runtime(Runtime::Codex).unwrap();
        catalog.available = true;
        let mut harness = modal_harness(1200., 900., Vec::new(), vec![catalog], ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            cx.notify();
        });
        let mut focus = None;
        harness.act(|_, window, cx| focus = window.focused(cx));
        harness.act(|root, window, cx| {
            root.apply_agent_tasks(
                vec![
                    task(Runtime::Codex, State::Running),
                    task(Runtime::Pi, State::Running),
                ],
                window,
                cx,
            );
            assert!(root.agent_update.is_none());
            assert_eq!(window.focused(cx), focus);
        });
        assert!(harness.bounds("BACKGROUND_TASK_PILL").is_some());
        let progress = harness.bounds("BACKGROUND_TASK_PROGRESS").unwrap();
        let count = harness.bounds("BACKGROUND_TASK_COUNT").unwrap();
        assert!(progress.origin.x < count.origin.x);
        let pill = harness.bounds("BACKGROUND_TASK_PILL").unwrap();
        harness
            .visual
            .simulate_click(pill.center(), Default::default());
        harness.visual.run_until_parked();
        assert!(harness.bounds("BACKGROUND_TASK_LIST").is_some());
        let hide = harness.bounds("BACKGROUND_TASK_HIDE").unwrap();
        harness
            .visual
            .simulate_click(hide.center(), Default::default());
        harness.visual.run_until_parked();
        assert!(harness.bounds("BACKGROUND_TASK_PILL").is_none());
        harness.act(|root, window, cx| {
            root.apply_agent_tasks(vec![task(Runtime::Pi, State::MayNeedInput)], window, cx);
            assert!(root.agent_update.is_none());
            assert_eq!(window.focused(cx), focus);
        });
        assert!(harness.bounds("BACKGROUND_TASK_PILL").is_some());
        assert!(harness
            .bounds("BACKGROUND_TASK_ICON:MayNeedInput")
            .is_some());
        assert!(harness.bounds("BACKGROUND_TASK_PROGRESS").is_none());
        for state in [State::Failed, State::Succeeded] {
            harness.act(|root, window, cx| {
                root.apply_agent_tasks(vec![task(Runtime::Pi, state)], window, cx)
            });
            assert!(harness
                .bounds(&format!("BACKGROUND_TASK_ICON:{state:?}"))
                .is_some());
            assert!(harness.bounds("BACKGROUND_TASK_PROGRESS").is_none());
        }
    }
}
