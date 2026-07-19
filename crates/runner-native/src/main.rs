mod composer;
mod terminal_element;
mod theme;

use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use futures::StreamExt as _;
use gpui::{
    actions, div, prelude::*, px, size, AnyElement, App, Application, Bounds, Context, Entity,
    FocusHandle, KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, ScrollDelta,
    ScrollWheelEvent, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use runner_app::model::SessionStatus;
use runner_app::ops::session::DirectSessionEntry;
use runner_app::AppCore;
use runner_native::bootstrap::{
    boot_core, native_paths, stop_running_direct_sessions, NativePaths,
};
use runner_native::terminal::{TerminalBridge, TerminalSession};

use composer::Composer;
use terminal_element::TerminalElement;

actions!(runner_native_ui, [Quit, TermPaste]);

const INITIAL_COLS: u16 = 100;
const INITIAL_ROWS: u16 = 30;

struct NativeRoot {
    core: AppCore,
    bridge: Arc<TerminalBridge>,
    sessions: Vec<DirectSessionEntry>,
    selected_id: Option<String>,
    terminal: Option<Arc<TerminalSession>>,
    composer: Option<Entity<Composer>>,
    terminal_focus: FocusHandle,
    waker: Arc<dyn Fn() + Send + Sync>,
    scroll_accumulator: f32,
    error: Option<String>,
}

impl NativeRoot {
    fn new(core: AppCore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let bridge =
            TerminalBridge::new(core.clone(), Arc::clone(&waker)).expect("start event bridge");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let terminal_focus = cx.focus_handle();
        terminal_focus.focus(window);
        let (sessions, error) = match runner_app::ops::session::session_list_recent_direct(&core) {
            Ok(sessions) => (sessions, None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };

        Self {
            core,
            bridge,
            sessions,
            selected_id: None,
            terminal: None,
            composer: None,
            terminal_focus,
            waker,
            scroll_accumulator: 0.,
            error,
        }
    }

    fn refresh_sessions(&mut self) {
        match runner_app::ops::session::session_list_recent_direct(&self.core) {
            Ok(sessions) => self.sessions = sessions,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn open_chat(&mut self, session_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_id.as_deref() == Some(&session_id) {
            self.terminal_focus.focus(window);
            return;
        }
        match self.try_open_chat(session_id, cx) {
            Ok(()) => {
                self.error = None;
                self.terminal_focus.focus(window);
                self.refresh_sessions();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn try_open_chat(&mut self, session_id: String, cx: &mut Context<Self>) -> Result<()> {
        let entry = runner_app::ops::session::session_get(&self.core, &session_id)?
            .with_context(|| format!("direct chat not found: {session_id}"))?;
        if entry.archived_at.is_some() {
            bail!("this chat is archived");
        }
        let (cols, rows) = self
            .terminal
            .as_ref()
            .map(|terminal| terminal.size())
            .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
        if entry.status != SessionStatus::Running {
            runner_app::ops::session::session_resume(
                &self.core,
                &session_id,
                Some(cols),
                Some(rows),
            )?;
        }

        let terminal = TerminalSession::attach(
            self.core.clone(),
            session_id.clone(),
            cols,
            rows,
            Arc::clone(&self.waker),
        )?;
        self.bridge.attach(Arc::clone(&terminal))?;
        let composer_focus = cx.focus_handle();
        let composer = cx.new(|_| Composer::new(composer_focus, Arc::clone(&terminal)));

        self.selected_id = Some(session_id);
        self.terminal = Some(terminal);
        self.composer = Some(composer);
        self.scroll_accumulator = 0.;
        Ok(())
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform {
            return;
        }
        let Some(terminal) = self.terminal.as_ref() else {
            return;
        };
        let keystroke = &event.keystroke;
        match terminal.send_key(
            &keystroke.key,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                terminal.scroll_to_bottom();
                self.error = None;
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.as_ref() else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        self.scroll_accumulator += lines;
        let whole = self.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            self.scroll_accumulator -= whole as f32;
            terminal.scroll(whole);
            cx.notify();
        }
    }

    fn on_paste(&mut self, _: &TermPaste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal.as_ref() else {
            return;
        };
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Err(error) = terminal.paste(&text) {
            self.error = Some(error.to_string());
        }
    }

    fn selected_label(&self) -> String {
        let Some(selected_id) = self.selected_id.as_deref() else {
            return "Direct chat".to_string();
        };
        self.sessions
            .iter()
            .find(|entry| entry.session_id == selected_id)
            .map(session_label)
            .unwrap_or_else(|| "Direct chat".to_string())
    }
}

impl Render for NativeRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.bridge.take_session_refresh() {
            self.refresh_sessions();
        }

        let selected_id = self.selected_id.clone();
        let session_rows = self
            .sessions
            .iter()
            .map(|entry| {
                (
                    entry.session_id.clone(),
                    session_label(entry),
                    status_label(entry.status),
                )
            })
            .collect::<Vec<_>>();
        let sidebar = div()
            .w(px(248.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::composer_bg())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .px_4()
                    .pt_10()
                    .pb_3()
                    .text_sm()
                    .text_color(theme::muted())
                    .child("DIRECT CHATS"),
            )
            .child(
                div()
                    .id("session-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_2()
                    .children(session_rows.into_iter().enumerate().map(
                        |(index, (session_id, label, status))| {
                            let selected = selected_id.as_deref() == Some(&session_id);
                            let click_id = session_id.clone();
                            div()
                                .id(("direct-session", index))
                                .w_full()
                                .mb_1()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .when(selected, |row| row.bg(theme::border()))
                                .hover(|row| row.bg(theme::border()))
                                .child(div().text_sm().text_color(theme::text()).child(label))
                                .child(div().text_xs().text_color(theme::muted()).child(status))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_chat(click_id.clone(), window, cx);
                                }))
                        },
                    )),
            );

        let workspace: AnyElement =
            if let (Some(terminal), Some(composer)) = (&self.terminal, &self.composer) {
                let focused = self.terminal_focus.is_focused(window);
                let title = terminal.title();
                let subtitle = if title.is_empty() {
                    self.selected_label()
                } else {
                    title
                };
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .h(px(42.))
                            .px_3()
                            .flex()
                            .items_center()
                            .border_b_1()
                            .border_color(theme::border())
                            .text_sm()
                            .text_color(theme::text())
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .id("terminal")
                            .key_context("Terminal")
                            .track_focus(&self.terminal_focus)
                            .flex_1()
                            .min_h(px(0.))
                            .p_2()
                            .on_key_down(cx.listener(Self::on_key_down))
                            .on_scroll_wheel(cx.listener(Self::on_scroll))
                            .on_action(cx.listener(Self::on_paste))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, _| {
                                    this.terminal_focus.focus(window);
                                }),
                            )
                            .child(TerminalElement::new(Arc::clone(terminal), focused)),
                    )
                    .child(composer.clone())
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::muted())
                    .child(if self.sessions.is_empty() {
                        "No existing direct chats"
                    } else {
                        "Select a direct chat"
                    })
                    .into_any_element()
            };

        div()
            .size_full()
            .flex()
            .bg(theme::bg())
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(workspace)
                    .children(self.error.as_ref().map(|error| {
                        div()
                            .flex_none()
                            .px_3()
                            .py_2()
                            .bg(gpui::rgb(0x3b1d2b))
                            .text_sm()
                            .text_color(gpui::rgb(0xf7768e))
                            .child(SharedString::from(error.clone()))
                    })),
            )
    }
}

fn session_label(entry: &DirectSessionEntry) -> String {
    entry.title.clone().unwrap_or_else(|| {
        entry
            .handle
            .as_ref()
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| entry.display_name.clone())
    })
}

fn status_label(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Running => "running",
        SessionStatus::Stopped => "stopped · click to resume",
        SessionStatus::Crashed => "crashed · click to resume",
    }
}

fn run() -> Result<()> {
    let paths = native_paths()?;
    let core = boot_core(&paths)?;
    print_startup_paths(&paths);
    let shutdown_core = core.clone();

    Application::new().run(move |cx: &mut App| {
        let quit_core = core.clone();
        cx.on_action(move |_: &Quit, cx| {
            let _ = stop_running_direct_sessions(&quit_core);
            cx.quit();
        });
        let close_core = core.clone();
        cx.on_window_closed(move |cx| {
            let _ = stop_running_direct_sessions(&close_core);
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-v", TermPaste, Some("Terminal")),
            KeyBinding::new("backspace", composer::Backspace, Some("Composer")),
            KeyBinding::new("delete", composer::Delete, Some("Composer")),
            KeyBinding::new("left", composer::Left, Some("Composer")),
            KeyBinding::new("right", composer::Right, Some("Composer")),
            KeyBinding::new("shift-left", composer::SelectLeft, Some("Composer")),
            KeyBinding::new("shift-right", composer::SelectRight, Some("Composer")),
            KeyBinding::new("cmd-a", composer::SelectAll, Some("Composer")),
            KeyBinding::new("home", composer::Home, Some("Composer")),
            KeyBinding::new("end", composer::End, Some("Composer")),
            KeyBinding::new("cmd-v", composer::Paste, Some("Composer")),
            KeyBinding::new("enter", composer::Submit, Some("Composer")),
            KeyBinding::new(
                "ctrl-cmd-space",
                composer::ShowCharacterPalette,
                Some("Composer"),
            ),
        ]);
        cx.set_menus(vec![Menu {
            name: "Runner Native".into(),
            items: vec![MenuItem::action("Quit", Quit)],
        }]);

        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Runner Native".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| NativeRoot::new(core.clone(), window, cx)),
        )
        .expect("open Runner Native window");
        cx.activate(true);
    });

    stop_running_direct_sessions(&shutdown_core)?;
    Ok(())
}

fn print_startup_paths(paths: &NativePaths) {
    eprintln!(
        "Runner Native: database={} logs={}",
        paths.app_data_dir.join("runner.db").display(),
        paths.log_dir.display()
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Runner Native failed: {error:#}");
        std::process::exit(1);
    }
}
