//! The modal that runs an agent CLI's own update command in a real terminal
//! over Settings → Agents (#533). Its PTY is an unlisted process in the
//! session manager: no `sessions` row, so it never shows up as a chat.

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, Render, ScrollDelta, ScrollWheelEvent, Task, Window,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::button::spinner;
use runner_app::ui::{Button, ButtonSize, ButtonVariant, Scrollbar};
use runner_app::{Copy, Paste};
use runner_backend::model::Runtime;
use runner_backend::session::manager::{ExitEvent, OutputEvent, SessionEvents};
use runner_backend::AppCore;
use runner_terminal::terminal::{TerminalSession, TerminalView};

use crate::app_settings::{self, AppSettings};
use crate::app_store::AppStore;
use crate::chat_icon::ChatIcon;
use crate::surfaces::app_shell::terminal_style_for;
use crate::terminal::element::{TerminalElement, TerminalInteraction, TerminalStyle};
use crate::{theme, NativeRoot};

const PANEL_WIDTH: f32 = 640.;
const TERMINAL_HEIGHT: f32 = 320.;
/// How long a running update must stay silent, with its cursor after a
/// prompt rather than at the start of a line, before the footer says it is
/// waiting for input.
const WAITING_AFTER: Duration = Duration::from_secs(2);

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct AgentUpdateRequest {
    pub(crate) runtime: Runtime,
    pub(crate) display_name: String,
    /// Catalog command name (`codex`), for the `codex update` label.
    pub(crate) command: String,
    pub(crate) installed: String,
    pub(crate) available: String,
}

impl AgentUpdateRequest {
    fn command_label(&self) -> String {
        let args = runner_backend::router::runtime::runtime_definition(self.runtime)
            .map(|definition| definition.update_args.join(" "))
            .unwrap_or_default();
        format!("{} {args}", self.command).trim().to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Starting,
    Running,
    Verifying,
    Succeeded {
        version: Option<String>,
        running_sessions: usize,
    },
    Failed(String),
}

impl Phase {
    /// Stopping npm or brew halfway can break an install, so the modal only
    /// closes once the process is gone and the outcome is known.
    fn dismissible(&self) -> bool {
        matches!(self, Self::Succeeded { .. } | Self::Failed(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FooterTone {
    Text,
    Muted,
    Warning,
    Danger,
}

#[derive(Debug, PartialEq, Eq)]
struct Footer {
    lines: Vec<(String, FooterTone)>,
    busy: bool,
    button: Option<&'static str>,
}

fn footer(phase: &Phase, waiting: bool, request: &AgentUpdateRequest) -> Footer {
    let line = |text: String, tone| Footer {
        lines: vec![(text, tone)],
        busy: false,
        button: None,
    };
    match phase {
        Phase::Starting => Footer {
            busy: true,
            ..line(
                format!("Starting {}…", request.command_label()),
                FooterTone::Muted,
            )
        },
        Phase::Running if waiting => line(
            "Waiting for input in the terminal.".into(),
            FooterTone::Warning,
        ),
        Phase::Running => Footer {
            busy: true,
            ..line(
                format!("Running {}…", request.command_label()),
                FooterTone::Muted,
            )
        },
        Phase::Verifying => Footer {
            busy: true,
            ..line("Checking the installed version…".into(), FooterTone::Muted)
        },
        Phase::Succeeded {
            version,
            running_sessions,
        } => {
            let mut lines = vec![(
                version.as_ref().map_or_else(
                    || format!("{} update finished.", request.display_name),
                    |version| format!("{} is now {version}.", request.display_name),
                ),
                FooterTone::Text,
            )];
            if *running_sessions > 0 {
                lines.push((
                    format!(
                        "Running sessions keep {} until they relaunch.",
                        request.installed
                    ),
                    FooterTone::Muted,
                ));
            }
            Footer {
                lines,
                busy: false,
                button: Some("Done"),
            }
        }
        Phase::Failed(message) => Footer {
            button: Some("Close"),
            ..line(message.clone(), FooterTone::Danger)
        },
    }
}

fn exit_message(exit_code: Option<i32>) -> String {
    match exit_code {
        Some(code) => format!("Exited with code {code}."),
        None => "The update ended without an exit code.".into(),
    }
}

/// A prompt leaves the cursor after its text; progress output ends its lines.
/// Runner never reads what the CLI printed, only how long it has been quiet
/// and where the cursor sits.
fn waiting_for_input(quiet: Duration, cursor_column: usize) -> bool {
    quiet >= WAITING_AFTER && cursor_column > 0
}

/// A first guess at the terminal grid; the element resizes the PTY to the
/// grid it actually lays out on first paint.
fn initial_grid(style: &TerminalStyle) -> (u16, u16) {
    let cell_width = style.font_size * 0.6;
    let line_height = (style.font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round();
    let width = (PANEL_WIDTH - 44. - 28.) * style.app_zoom;
    let height = (TERMINAL_HEIGHT - 24.) * style.app_zoom;
    (
        (width / cell_width).floor().max(20.) as u16,
        (height / line_height).floor().max(5.) as u16,
    )
}

/// Hears the unlisted PTY: output feeds the modal's terminal and nothing
/// else, and the exit code waits for the modal's next tick.
struct UpdateTerminalEvents {
    terminal: Arc<TerminalSession>,
    exit: Arc<Mutex<Option<Option<i32>>>>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl SessionEvents for UpdateTerminalEvents {
    fn output(&self, event: &OutputEvent) {
        if let Err(error) = self.terminal.feed_output(event) {
            tracing::warn!("feed agent update terminal failed: {error}");
        }
    }

    fn exit(&self, event: &ExitEvent) {
        *self.exit.lock().unwrap() = Some(event.exit_code);
        (self.waker)();
    }
}

struct UpdateTerminal {
    terminal: Arc<TerminalSession>,
    _view: TerminalView,
    interaction: Entity<TerminalInteraction>,
    input: Entity<TerminalInput>,
    scrollbar: Entity<Scrollbar>,
    focus: FocusHandle,
    scroll_accumulator: f32,
}

pub(crate) struct AgentUpdateDialog {
    request: AgentUpdateRequest,
    app_store: Entity<AppStore>,
    phase: Phase,
    waiting: bool,
    terminal: Option<UpdateTerminal>,
    exit: Arc<Mutex<Option<Option<i32>>>>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    close: CloseHandler,
    _tasks: Vec<Task<()>>,
}

impl AgentUpdateDialog {
    fn new(
        request: AgentUpdateRequest,
        app_store: Entity<AppStore>,
        close: CloseHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window);
        Self {
            request,
            app_store,
            phase: Phase::Starting,
            waiting: false,
            terminal: None,
            exit: Arc::new(Mutex::new(None)),
            focus,
            previous_focus,
            close,
            _tasks: Vec::new(),
        }
    }

    /// Spawns the update command. Kept out of `new` so a dialog can be built
    /// without running anything.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.app_store.read(cx).core.clone();
        let size = initial_grid(&terminal_style_for(&self.app_store.read(cx).settings));
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let start = {
            let exit = Arc::clone(&self.exit);
            let runtime = self.request.runtime;
            cx.background_spawn(async move {
                start_update(core, runtime, size, exit, waker).map_err(|error| error.to_string())
            })
        };
        let started = cx.spawn_in(window, async move |this, cx| {
            let result = start.await;
            let _ = this.update_in(cx, |this, window, cx| this.started(result, window, cx));
        });
        let wakes = cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let ticks = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            match this.update(cx, |this, cx| {
                this.tick(cx);
                !this.phase.dismissible()
            }) {
                Ok(true) => {}
                _ => break,
            }
        });
        self._tasks = vec![started, wakes, ticks];
    }

    pub(crate) fn restore_focus(&self, window: &mut Window) {
        if let Some(focus) = &self.previous_focus {
            focus.focus(window);
        }
    }

    fn started(
        &mut self,
        result: Result<Arc<TerminalSession>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let terminal = match result {
            Ok(terminal) => terminal,
            Err(error) => {
                self.phase = Phase::Failed(error);
                cx.notify();
                return;
            }
        };
        let settings = self.app_store.read(cx).settings.clone();
        terminal.set_palette(app_settings::terminal_palette(
            &settings,
            theme::active_variant(),
        ));
        terminal.configure(
            app_settings::TERMINAL_SCROLLBACK_LINES,
            cursor_shape(&settings),
        );
        let focus = cx.focus_handle();
        focus.focus(window);
        self.terminal = Some(UpdateTerminal {
            _view: terminal.view(),
            interaction: cx.new(|_| TerminalInteraction::new(Arc::clone(&terminal))),
            input: cx.new(|_| TerminalInput::new(Arc::clone(&terminal))),
            scrollbar: cx.new(|_| Scrollbar::terminal(Arc::clone(&terminal))),
            terminal,
            focus,
            scroll_accumulator: 0.,
        });
        self.phase = Phase::Running;
        self.tick(cx);
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Running {
            let exit = self.exit.lock().unwrap().take();
            if let Some(exit_code) = exit {
                self.finish(exit_code, cx);
            } else if let Some(attached) = &self.terminal {
                let quiet = attached
                    .terminal
                    .output_activity()
                    .last_output_at
                    .map_or(Duration::ZERO, |at| at.elapsed());
                let column = attached
                    .terminal
                    .term
                    .lock_unfair()
                    .grid()
                    .cursor
                    .point
                    .column
                    .0;
                self.waiting = waiting_for_input(quiet, column);
            }
        }
        cx.notify();
    }

    /// The Agents pane re-probes the version on every exit; a clean exit
    /// waits for that probe so the footer can name the new version.
    fn finish(&mut self, exit_code: Option<i32>, cx: &mut Context<Self>) {
        self.waiting = false;
        self.phase = if exit_code == Some(0) {
            Phase::Verifying
        } else {
            Phase::Failed(exit_message(exit_code))
        };
        tracing::info!(
            "runtime update exited: runtime={} exit_code={exit_code:?}",
            self.request.runtime
        );
        let core = self.app_store.read(cx).core.clone();
        let runtime = self.request.runtime;
        let probe = cx.background_spawn(async move {
            let version = runner_backend::ops::runtime::runtime_probe_version(&core, runtime);
            let running = runner_backend::ops::session::live_session_counts(&core)
                .ok()
                .and_then(|counts| counts.get(&runtime).copied())
                .unwrap_or(0);
            (version, running)
        });
        cx.spawn(async move |this, cx| {
            let (version, running_sessions) = probe.await;
            let _ = this.update(cx, |this, cx| {
                if this.phase == Phase::Verifying {
                    this.phase = Phase::Succeeded {
                        version,
                        running_sessions,
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase.dismissible() {
            (self.close)(window, cx);
        }
    }

    fn on_terminal_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(attached) = &self.terminal else {
            return;
        };
        let keystroke = &event.keystroke;
        if runner_app::terminal_ime::swallows_option_copy(
            keystroke.modifiers.platform,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            &keystroke.key,
            keystroke.key_char.as_deref(),
        ) {
            cx.stop_propagation();
            return;
        }
        if runner_app::terminal_ime::terminal_key_route(
            attached.input.read(cx).is_composing(),
            keystroke.modifiers.platform,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.function,
            &keystroke.key,
        ) != runner_app::terminal_ime::TerminalKeyRoute::Raw
        {
            return;
        }
        match attached.terminal.send_key(
            &keystroke.key,
            keystroke.modifiers.control,
            keystroke.modifiers.alt,
            keystroke.modifiers.shift,
            keystroke.key_char.as_deref(),
        ) {
            Ok(true) => {
                attached.terminal.scroll_to_bottom();
                cx.stop_propagation();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!("agent update terminal input failed: {error}");
                cx.stop_propagation();
            }
        }
    }

    fn on_terminal_paste(&mut self, cx: &mut Context<Self>) {
        let Some(attached) = &self.terminal else {
            return;
        };
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if let Err(error) = attached.terminal.paste(&text) {
                tracing::warn!("agent update terminal paste failed: {error}");
            }
        }
    }

    fn on_terminal_copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self
            .terminal
            .as_ref()
            .and_then(|attached| attached.terminal.selection_text())
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            cx.stop_propagation();
        }
    }

    fn on_terminal_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(attached) = self.terminal.as_mut() else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(point) => point.y,
            ScrollDelta::Pixels(point) => f32::from(point.y) / f32::from(window.line_height()),
        };
        attached.scroll_accumulator += lines;
        let whole = attached.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            attached.scroll_accumulator -= whole as f32;
            attached.terminal.scroll(whole, event.modifiers.shift);
            cx.notify();
        }
    }

    fn render_terminal(&self, style: TerminalStyle, cx: &mut Context<Self>) -> AnyElement {
        let background = crate::terminal::element::to_hsla(style.palette.background, 1.);
        let Some(attached) = &self.terminal else {
            return div().absolute().inset_0().bg(background).into_any_element();
        };
        let running = self.phase == Phase::Running;
        div()
            .id("agent-update-terminal")
            .debug_selector(|| "AGENT_UPDATE_TERMINAL".into())
            .absolute()
            .inset_0()
            .key_context("Terminal")
            .track_focus(&attached.focus)
            .flex()
            .py_3()
            .pl_3()
            .pr_1()
            .bg(background)
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.on_terminal_copy(cx)))
            .on_scroll_wheel(
                cx.listener(|this, event, window, cx| this.on_terminal_scroll(event, window, cx)),
            )
            .when(running, |surface| {
                surface
                    .on_key_down(cx.listener(|this, event, _, cx| this.on_terminal_key(event, cx)))
                    .on_action(cx.listener(|this, _: &Paste, _, cx| this.on_terminal_paste(cx)))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .pr(runner_app::ui::terminal_scrollbar_gutter())
                    .child(TerminalElement::new(
                        Arc::clone(&attached.terminal),
                        attached.interaction.clone(),
                        attached.input.clone(),
                        attached.focus.clone(),
                        running,
                        running,
                        style,
                    ))
                    .child(attached.scrollbar.clone()),
            )
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let footer = footer(&self.phase, self.waiting, &self.request);
        let done = cx.weak_entity();
        div()
            .debug_selector(|| "AGENT_UPDATE_FOOTER".into())
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .items_start()
                    .gap_2()
                    .children(
                        footer
                            .busy
                            .then(|| spinner("agent-update-spinner", 12., theme::muted())),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .children(footer.lines.into_iter().map(|(text, tone)| {
                                div()
                                    .whitespace_normal()
                                    .text_size(theme::text_ui())
                                    .line_height(rems(18. / 16.))
                                    .text_color(match tone {
                                        FooterTone::Text => theme::text(),
                                        FooterTone::Muted => theme::muted(),
                                        FooterTone::Warning => theme::warning(),
                                        FooterTone::Danger => theme::danger(),
                                    })
                                    .child(text)
                            })),
                    ),
            )
            .children(footer.button.map(|label| {
                Button::new("agent-update-close", label)
                    .size(ButtonSize::Sm)
                    .variant(if label == "Done" {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_press(move |window, cx| {
                        let _ = done.update(cx, |dialog, cx| dialog.dismiss(window, cx));
                    })
            }))
            .into_any_element()
    }
}

fn start_update(
    core: AppCore,
    runtime: Runtime,
    size: (u16, u16),
    exit: Arc<Mutex<Option<Option<i32>>>>,
    waker: Arc<dyn Fn() + Send + Sync>,
) -> anyhow::Result<Arc<TerminalSession>> {
    let spec = runner_backend::ops::runtime::runtime_update_spawn_spec(&core, runtime, size)?;
    let terminal = TerminalSession::attach(
        core.clone(),
        spec.session_id.clone(),
        size.0,
        size.1,
        Arc::clone(&waker),
    )?;
    let events = Arc::new(UpdateTerminalEvents {
        terminal: Arc::clone(&terminal),
        exit,
        waker,
    });
    runner_backend::ops::runtime::runtime_update_start(&core, spec, events)?;
    Ok(terminal)
}

fn cursor_shape(settings: &AppSettings) -> alacritty_terminal::vte::ansi::CursorShape {
    use alacritty_terminal::vte::ansi::CursorShape;
    match settings.terminal_cursor_style {
        app_settings::TerminalCursorStyle::Block => CursorShape::Block,
        app_settings::TerminalCursorStyle::Underline => CursorShape::Underline,
        app_settings::TerminalCursorStyle::Bar => CursorShape::Beam,
    }
}

impl Render for AgentUpdateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = terminal_style_for(&self.app_store.read(cx).settings);
        let mark = ChatIcon::for_runtime(self.request.runtime.key());
        let title = format!("Updating {}", self.request.display_name);
        let subtitle = format!(
            "{} → {} · {}",
            self.request.installed,
            self.request.available,
            self.request.command_label()
        );
        let terminal = self.render_terminal(style.clone(), cx);
        let footer = self.render_footer(cx);
        let terminal_background = crate::terminal::element::to_hsla(style.palette.background, 1.);
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.dismiss(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.dismiss(window, cx);
                }
            }))
            .child(
                div()
                    .debug_selector(|| "AGENT_UPDATE_PANEL".into())
                    .w_full()
                    .max_w(rems(PANEL_WIDTH / 16.))
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .rounded(rems(14. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px(rems(22. / 16.))
                    .py(rems(20. / 16.))
                    .shadow_2xl()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(12. / 16.))
                            .child(
                                svg()
                                    .path(mark.path)
                                    .size(rems(20. / 16.))
                                    .flex_none()
                                    .text_color(mark.color(theme::text(), true)),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_title())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_meta())
                                            .text_color(theme::muted())
                                            .child(subtitle),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(rems(TERMINAL_HEIGHT / 16.))
                            .overflow_hidden()
                            .rounded(rems(10. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(terminal_background)
                            .child(terminal),
                    )
                    .child(footer),
            )
    }
}

impl NativeRoot {
    pub(crate) fn open_agent_update(
        &mut self,
        request: AgentUpdateRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_update.is_some() {
            return;
        }
        let root = cx.weak_entity();
        let close: CloseHandler = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = root.update(cx, |root, cx| root.close_agent_update(window, cx));
        });
        let app_store = self.app_store.clone();
        self.agent_update = Some(cx.new(|cx| {
            let mut dialog = AgentUpdateDialog::new(request, app_store, close, window, cx);
            dialog.start(window, cx);
            dialog
        }));
        cx.notify();
    }

    pub(crate) fn close_agent_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.agent_update.take() {
            dialog.read(cx).restore_focus(window);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AgentUpdateRequest {
        AgentUpdateRequest {
            runtime: Runtime::Codex,
            display_name: "Codex".into(),
            command: "codex".into(),
            installed: "0.153.4".into(),
            available: "0.155.0".into(),
        }
    }

    #[test]
    fn footer_follows_the_four_states() {
        let request = request();
        assert_eq!(request.command_label(), "codex update");
        assert_eq!(
            footer(&Phase::Running, false, &request),
            Footer {
                lines: vec![("Running codex update…".into(), FooterTone::Muted)],
                busy: true,
                button: None,
            }
        );
        assert_eq!(
            footer(&Phase::Running, true, &request),
            Footer {
                lines: vec![(
                    "Waiting for input in the terminal.".into(),
                    FooterTone::Warning
                )],
                busy: false,
                button: None,
            }
        );
        assert_eq!(
            footer(
                &Phase::Succeeded {
                    version: Some("0.155.0".into()),
                    running_sessions: 0,
                },
                false,
                &request
            ),
            Footer {
                lines: vec![("Codex is now 0.155.0.".into(), FooterTone::Text)],
                busy: false,
                button: Some("Done"),
            }
        );
        assert_eq!(
            footer(
                &Phase::Succeeded {
                    version: Some("0.155.0".into()),
                    running_sessions: 3,
                },
                false,
                &request
            )
            .lines,
            [
                ("Codex is now 0.155.0.".into(), FooterTone::Text),
                (
                    "Running sessions keep 0.153.4 until they relaunch.".into(),
                    FooterTone::Muted
                ),
            ]
        );
        assert_eq!(
            footer(&Phase::Failed(exit_message(Some(243))), false, &request),
            Footer {
                lines: vec![("Exited with code 243.".into(), FooterTone::Danger)],
                busy: false,
                button: Some("Close"),
            }
        );
        assert!(footer(&Phase::Starting, false, &request).button.is_none());
        assert!(footer(&Phase::Verifying, false, &request).button.is_none());
    }

    #[test]
    fn only_a_finished_update_can_be_dismissed() {
        assert!(!Phase::Starting.dismissible());
        assert!(!Phase::Running.dismissible());
        assert!(!Phase::Verifying.dismissible());
        assert!(Phase::Succeeded {
            version: None,
            running_sessions: 0
        }
        .dismissible());
        assert!(Phase::Failed("Exited with code 1.".into()).dismissible());
    }

    fn test_store(path: &std::path::Path, cx: &mut gpui::TestAppContext) -> Entity<AppStore> {
        use runner_backend::{
            db, event_bus, events, mcp, router, session, shell_path, windows, AppCore,
        };
        use std::sync::RwLock;
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&path.join("runner.db")).unwrap()),
            app_data_dir: path.into(),
            sessions: session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            usage: Arc::new(runner_backend::usage::UsageService::default()),
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        };
        cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                path.join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        })
    }

    struct Host(Entity<AgentUpdateDialog>);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }

    /// Builds the dialog without `start`, so no update process ever runs.
    #[test]
    fn running_update_ignores_escape_and_the_scrim_and_a_finished_one_closes() {
        use gpui::{point, px, size, Modifiers, TestAppContext, VisualTestContext};
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = test_store(temp.path(), &mut cx);
        let closes = Rc::new(std::cell::Cell::new(0));
        let counter = Rc::clone(&closes);
        let window = cx.add_window(|window, cx| {
            let close: CloseHandler = Rc::new(move |_, _| counter.set(counter.get() + 1));
            Host(cx.new(|cx| AgentUpdateDialog::new(request(), store, close, window, cx)))
        });
        let host = window.update(&mut cx, |host, _, _| host.0.clone()).unwrap();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let viewport = size(px(1200.), px(800.));
        window.simulate_resize(viewport);
        host.update(&mut window, |dialog, cx| {
            dialog.phase = Phase::Running;
            cx.notify();
        });
        window.run_until_parked();

        let panel = window.debug_bounds("AGENT_UPDATE_PANEL").unwrap();
        let footer = window.debug_bounds("AGENT_UPDATE_FOOTER").unwrap();
        assert!((panel.center().x - viewport.width / 2.).abs() <= px(1.));
        assert!((panel.center().y - viewport.height / 2.).abs() <= px(1.));
        assert!(footer.bottom() <= panel.bottom());

        window.simulate_keystrokes("escape");
        window.simulate_click(point(px(8.), px(8.)), Modifiers::default());
        window.run_until_parked();
        assert_eq!(closes.get(), 0);

        host.update(&mut window, |dialog, cx| {
            dialog.phase = Phase::Failed(exit_message(Some(1)));
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_click(point(px(8.), px(8.)), Modifiers::default());
        window.run_until_parked();
        assert_eq!(closes.get(), 1);
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        assert_eq!(closes.get(), 2);
    }

    #[test]
    fn waiting_needs_silence_and_a_cursor_after_a_prompt() {
        assert!(!waiting_for_input(Duration::from_millis(500), 10));
        assert!(!waiting_for_input(Duration::from_secs(5), 0));
        assert!(waiting_for_input(Duration::from_secs(2), 10));
        assert_eq!(exit_message(None), "The update ended without an exit code.");
    }
}
