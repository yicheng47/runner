use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, rems, svg, App, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton, Render,
    Window,
};
use runner_app::lifecycle::{quit_choice, QuitChoice, QuitRequest, QuitState, SessionSummary};
use runner_app::ui::{Button, ButtonVariant};
use runner_core::protocol::status::Activity;

use crate::{app_store::global_app_store, theme, NativeRoot};

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct QuitDialog {
    summary: SessionSummary,
    choice: QuitChoice,
    dont_ask: bool,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    close: CloseHandler,
}

pub(crate) fn finish_quit(choice: QuitChoice, cx: &mut App) {
    if cx.try_global::<QuitState>().is_none() {
        cx.set_global(QuitState::default());
    }
    cx.global_mut::<QuitState>().choice = choice;
    cx.quit();
}

impl QuitDialog {
    fn new(
        summary: SessionSummary,
        choice: QuitChoice,
        close: CloseHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            summary,
            choice,
            dont_ask: false,
            focus,
            previous_focus,
            close,
        }
    }

    fn confirm(&self, cx: &mut App) {
        global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.last_quit_choice = self.choice;
                    if self.dont_ask {
                        settings.quit_behavior = self.choice.behavior();
                    }
                    true
                },
                true,
                cx,
            );
        });
        finish_quit(self.choice, cx);
    }
}

impl Render for QuitDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handler = self.close.clone();
        let previous_focus = self.previous_focus.clone();
        let close: CloseHandler = Rc::new(move |window, cx| {
            if let Some(focus) = &previous_focus {
                focus.focus(window, cx);
            }
            handler(window, cx);
        });
        let cancel = close.clone();
        let mut choices = Vec::new();
        for (choice, title, description) in [
            (
                QuitChoice::Keep,
                "Keep them running",
                "Agents keep working in the background. Open Runner to see them again.".to_owned(),
            ),
            (
                QuitChoice::Stop,
                "Stop them",
                self.summary.stop_description(),
            ),
        ] {
            let selected = self.choice == choice;
            choices.push(
                div()
                    .id(if choice == QuitChoice::Keep {
                        "quit-keep"
                    } else {
                        "quit-stop"
                    })
                    .debug_selector(move || {
                        if choice == QuitChoice::Keep {
                            "QUIT_KEEP".into()
                        } else {
                            "QUIT_STOP".into()
                        }
                    })
                    .flex()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::border())
                    .bg(if selected {
                        theme::raised()
                    } else {
                        theme::bg()
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choice = choice;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .mt_1()
                            .size(rems(12. / 16.))
                            .rounded_full()
                            .border_1()
                            .border_color(if selected {
                                theme::accent()
                            } else {
                                theme::border()
                            })
                            .bg(if selected {
                                theme::accent()
                            } else {
                                theme::bg()
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |radio| {
                                radio.child(
                                    div().size(rems(5. / 16.)).rounded_full().bg(theme::bg()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_size(theme::text_body())
                                    .line_height(rems(18. / 16.))
                                    .text_color(theme::muted())
                                    .child(description),
                            ),
                    ),
            );
        }
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x00000099))
            .occlude()
            .track_focus(&self.focus)
            .key_context("QuitDialog")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        cx.stop_propagation();
                        close(window, cx);
                    }
                    "enter" => {
                        cx.stop_propagation();
                        this.confirm(cx);
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .w_full()
                    .max_w(rems(480. / 16.))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .shadow_2xl()
                    .debug_selector(|| "QUIT_DIALOG_PANEL".into())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                svg()
                                    .path("power.svg")
                                    .size(rems(1.))
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_title())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Quit Runner"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(self.summary.caption()),
                    )
                    .children(choices)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .mt_1()
                            .child(
                                div()
                                    .id("quit-dont-ask")
                                    .debug_selector(|| "QUIT_DONT_ASK".into())
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dont_ask = !this.dont_ask;
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .size(rems(13. / 16.))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(theme::faint())
                                            .when(self.dont_ask, |check| {
                                                check.child(
                                                    svg()
                                                        .path("check.svg")
                                                        .size_full()
                                                        .text_color(theme::accent()),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_body())
                                            .text_color(theme::muted())
                                            .child("Don't ask again"),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("quit-cancel", "Cancel")
                                    .variant(ButtonVariant::Secondary)
                                    .on_press(move |window, cx| cancel(window, cx)),
                            )
                            .child(
                                Button::new("quit-confirm", "Quit")
                                    .variant(ButtonVariant::Primary)
                                    .on_press({
                                        let dialog = cx.entity();
                                        move |_, cx| dialog.update(cx, |this, cx| this.confirm(cx))
                                    }),
                            ),
                    ),
            )
    }
}

impl NativeRoot {
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.try_global::<QuitState>().is_none() {
            cx.set_global(QuitState::default());
        }
        let owner = window.window_handle().window_id();
        let Some(generation) = cx.global_mut::<QuitState>().begin(owner) else {
            return;
        };
        let client = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let live = client.session_live_ids()?;
            let statuses = client.session_status_snapshot()?;
            let count = client.live_session_counts()?.values().sum();
            let mut mission_sessions = std::collections::HashMap::new();
            for mission in client.mission_list_summary_impl(None)? {
                for row in client.session_list(&mission.mission.id)? {
                    if row.runtime != "shell" {
                        mission_sessions.insert(
                            row.session.id.clone(),
                            row.live_title.unwrap_or_else(|| format!("@{}", row.handle)),
                        );
                    }
                }
            }
            let mut working = Vec::new();
            for id in &live {
                if statuses
                    .get(id)
                    .is_some_and(|status| status.observation.activity == Activity::Working)
                {
                    let detail = client.session_get(id)?;
                    if let Some(entry) = detail {
                        if entry.agent_runtime != "shell" {
                            working.push(
                                entry
                                    .preferred_title(entry.live_title.as_deref())
                                    .unwrap_or(entry.display_name),
                            );
                        }
                    } else if let Some(title) = mission_sessions.get(id) {
                        working.push(title.clone());
                    }
                }
            }
            Ok::<_, runner_core::protocol::ClientError>(SessionSummary {
                live: count,
                working,
            })
        });
        let app_cx = cx.to_async();
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            if weak
                .update_in(cx, |this, window, cx| {
                    if !cx.global::<QuitState>().is_current(owner, generation) {
                        return;
                    }
                    let summary = match result {
                        Ok(summary) => summary,
                        Err(error) => {
                            cx.global_mut::<QuitState>().complete(owner, generation);
                            eprintln!("Quit could not read the background service: {error}");
                            finish_quit(QuitChoice::Keep, cx);
                            return;
                        }
                    };
                    let settings = this.settings(cx);
                    let behavior = settings.quit_behavior;
                    let previous = settings.last_quit_choice;
                    if let Some(choice) = quit_choice(QuitRequest::User, behavior, summary.live) {
                        cx.global_mut::<QuitState>().complete(owner, generation);
                        finish_quit(choice, cx);
                        return;
                    }
                    let root = cx.weak_entity();
                    let close: CloseHandler = Rc::new(move |_, cx| {
                        let _ = root.update(cx, |this, cx| {
                            this.quit_dialog = None;
                            cx.global_mut::<QuitState>().complete(owner, generation);
                            cx.notify();
                        });
                    });
                    this.quit_dialog =
                        Some(cx.new(|cx| QuitDialog::new(summary, previous, close, window, cx)));
                    cx.notify();
                })
                .is_err()
            {
                app_cx.update(|cx| cx.global_mut::<QuitState>().complete(owner, generation));
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::AppSettings;
    use crate::app_store::{AppStore, GlobalAppStore};
    use crate::theme_snapshot::ThemeGuard;
    use gpui::{px, size, Entity, Modifiers, TestAppContext, VisualTestContext};
    use runner_daemon::{db, session, shell_path};
    use std::cell::Cell;
    use std::sync::{Arc, RwLock};

    struct DialogHost(Entity<QuitDialog>);
    impl Render for DialogHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    #[test]
    fn dialog_renders_choices_cancels_and_persists_both_confirmed_choices() {
        let _theme = ThemeGuard::new();
        for choice in [QuitChoice::Keep, QuitChoice::Stop] {
            let temp = tempfile::tempdir().unwrap();
            let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
            let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
            let core = crate::test_support::core(
                Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
                temp.path().to_owned(),
                session::SessionManager::new(
                    env.clone(),
                    discovery.clone(),
                    Arc::new(session::pty_runtime::PtyRuntime::new()),
                ),
                env,
                discovery,
            );
            let mut cx = TestAppContext::single();
            let store = cx.new(|cx| {
                AppStore::new(
                    core,
                    None,
                    None,
                    temp.path().join("settings.json"),
                    AppSettings::default(),
                    None,
                    cx,
                )
            });
            cx.update(|cx| {
                cx.set_global(GlobalAppStore(store.clone()));
                cx.set_global(QuitState::default());
            });
            let cancelled = Rc::new(Cell::new(0));
            let cancel = cancelled.clone();
            let window = cx.add_window(|window, cx| {
                window.resize(size(px(1000.), px(700.)));
                DialogHost(cx.new(|cx| {
                    QuitDialog::new(
                        SessionSummary {
                            live: 3,
                            working: vec!["Refactor the settings store".into()],
                        },
                        QuitChoice::Keep,
                        Rc::new(move |_, _| cancel.set(cancel.get() + 1)),
                        window,
                        cx,
                    )
                }))
            });
            cx.run_until_parked();
            let dialog = window.read_with(&cx, |host, _| host.0.clone()).unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            let panel = visual.debug_bounds("QUIT_DIALOG_PANEL").unwrap();
            let keep = visual.debug_bounds("QUIT_KEEP").unwrap();
            let stop = visual.debug_bounds("QUIT_STOP").unwrap();
            assert!(keep.top() >= panel.top() && stop.bottom() < panel.bottom());
            assert!(stop.top() >= keep.bottom());
            dialog.read_with(&cx, |dialog, _| assert_eq!(dialog.choice, QuitChoice::Keep));
            visual.simulate_keystrokes("escape");
            cx.run_until_parked();
            assert_eq!(cancelled.get(), 1);
            assert_eq!(
                store.read_with(&cx, |store, _| store.settings.last_quit_choice),
                QuitChoice::Keep
            );
            visual.simulate_click(
                if choice == QuitChoice::Keep {
                    keep.center()
                } else {
                    stop.center()
                },
                Modifiers::default(),
            );
            cx.run_until_parked();
            let check = visual.debug_bounds("QUIT_DONT_ASK").unwrap();
            visual.simulate_click(check.center(), Modifiers::default());
            cx.run_until_parked();
            visual.simulate_keystrokes("enter");
            cx.run_until_parked();
            assert_eq!(
                store.read_with(&cx, |store, _| store.settings.quit_behavior),
                choice.behavior()
            );
            let reloaded = AppSettings::load(&temp.path().join("settings.json")).unwrap();
            assert_eq!(reloaded.last_quit_choice, choice);
            assert_eq!(reloaded.quit_behavior, choice.behavior());
            assert_eq!(visual.read(|cx| cx.global::<QuitState>().choice), choice);
        }
    }
    struct LiveService(runner_daemon::daemon::InProcessTransport);
    impl runner_core::protocol::Transport for LiveService {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            use runner_core::protocol::{Request, Response};
            match request {
                Request::session_live_ids {} => {
                    Ok(Response::session_live_ids(Ok(vec!["test-live".into()])))
                }
                Request::live_session_counts {} => Ok(Response::live_session_counts(Ok([(
                    runner_core::Runtime::Shell,
                    1,
                )]
                .into_iter()
                .collect()))),
                request => self.0.call(request),
            }
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.0.subscribe()
        }
    }

    #[test]
    fn closing_the_quit_owner_allows_reopen_and_ignores_its_late_result() {
        let _theme = ThemeGuard::new();
        for summary_delivered in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| {
                store.client = runner_core::protocol::DaemonClient::new(Arc::new(LiveService(
                    runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                )));
            });
            let first = cx.add_window(|window, cx| {
                NativeRoot::new(
                    "quit-first".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            });
            first
                .update(&mut cx, |root, window, cx| root.request_quit(window, cx))
                .unwrap();
            if summary_delivered {
                cx.run_until_parked();
                first
                    .read_with(&cx, |root, _| assert!(root.quit_dialog.is_some()))
                    .unwrap();
            }
            first
                .update(&mut cx, |root, window, cx| {
                    root.prepare_window_close(window, cx);
                    assert!(!cx.global::<QuitState>().asking);
                    window.remove_window();
                })
                .unwrap();
            let second = cx.add_window(|window, cx| {
                NativeRoot::new(
                    "quit-second".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            });
            second
                .update(&mut cx, |root, window, cx| {
                    let state = cx.global_mut::<QuitState>();
                    let owner = window.window_handle().window_id();
                    let old = state.begin(owner).unwrap();
                    state.cancel(owner);
                    let new = state.begin(owner).unwrap();
                    state.complete(owner, old);
                    assert!(state.is_current(owner, new));
                    state.cancel(owner);
                    root.request_quit(window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            second
                .read_with(&cx, |root, cx| {
                    assert!(root.quit_dialog.is_some());
                    assert!(cx.global::<QuitState>().asking);
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(second.into(), &cx);
            assert!(visual.debug_bounds("QUIT_DIALOG_PANEL").is_some());
            visual.simulate_keystrokes("escape");
            cx.run_until_parked();
            second
                .read_with(&cx, |root, cx| {
                    assert!(root.quit_dialog.is_none());
                    assert!(!cx.global::<QuitState>().asking);
                })
                .unwrap();
        }
    }

    struct UnavailableService;
    impl runner_core::protocol::Transport for UnavailableService {
        fn call(
            &self,
            _: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            Err(runner_core::protocol::ClientError::msg("service stopped"))
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            panic!("this test replaces the client after event subscription")
        }
    }

    #[test]
    fn user_quit_without_live_sessions_or_a_service_exits_without_a_dialog() {
        let _theme = ThemeGuard::new();
        for unavailable in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
            let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
            let core = crate::test_support::core(
                Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
                temp.path().to_owned(),
                session::SessionManager::new(
                    env.clone(),
                    discovery.clone(),
                    Arc::new(session::pty_runtime::PtyRuntime::new()),
                ),
                env,
                discovery,
            );
            let mut cx = TestAppContext::single();
            let store = cx.new(|cx| {
                AppStore::new(
                    core,
                    None,
                    None,
                    temp.path().join("settings.json"),
                    AppSettings::default(),
                    None,
                    cx,
                )
            });
            cx.update(|cx| {
                cx.set_global(GlobalAppStore(store.clone()));
                cx.set_global(QuitState::default());
                cx.set_global(crate::WindowLayoutCheckpoint::default());
                #[cfg(not(windows))]
                let updater = cx.new(|cx| crate::Updater::new(false, cx));
                #[cfg(windows)]
                let updater =
                    cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
                cx.set_global(crate::GlobalUpdater(updater));
            });
            let window = cx.add_window(|window, cx| {
                NativeRoot::new(
                    "quit-test".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store,
                    window,
                    cx,
                )
            });
            cx.run_until_parked();
            window
                .update(&mut cx, |root, window, cx| {
                    if unavailable {
                        root.app_store.update(cx, |store, _| {
                            store.client = runner_core::protocol::DaemonClient::new(Arc::new(
                                UnavailableService,
                            ));
                        });
                    }
                    root.request_quit(window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            window
                .read_with(&cx, |root, cx| {
                    assert!(root.quit_dialog.is_none());
                    assert!(!cx.global::<QuitState>().asking);
                    assert_eq!(cx.global::<QuitState>().choice, QuitChoice::Keep);
                })
                .unwrap();
        }
    }
}
