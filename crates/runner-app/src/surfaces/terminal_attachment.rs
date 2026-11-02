use crate::surfaces::app_shell::terminal_style_for;
use crate::*;

impl AttachedChat {
    pub(crate) fn new<T: 'static>(
        terminal: Arc<TerminalMirror>,
        style: &crate::terminal::element::TerminalStyle,
        settings: &AppSettings,
        cx: &mut Context<T>,
        mut on_write: impl FnMut(&mut T, Result<(), String>, &mut Context<T>) + 'static,
    ) -> Self {
        terminal.set_palette(style.palette);
        app_settings::configure_terminal(&terminal, settings);
        let terminal_scrollbar = cx.new(|_| Scrollbar::terminal(Arc::clone(&terminal)));
        let terminal_interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&terminal)));
        let terminal_focus = cx.focus_handle();
        let terminal_find = cx.new(|cx| {
            crate::terminal::TerminalFind::new(Arc::clone(&terminal), terminal_focus.clone(), cx)
        });
        let terminal_input = cx.new(|_| TerminalInput::new(Arc::clone(&terminal)));
        let terminal_input_subscription = cx.observe(&terminal_input, move |this, input, cx| {
            if let Some(result) = input.update(cx, |input, _| input.take_write_result()) {
                on_write(this, result, cx);
            }
            cx.notify();
        });
        Self {
            _terminal_view: terminal.view(),
            terminal,
            terminal_interaction,
            terminal_scrollbar,
            terminal_input,
            _terminal_input_subscription: terminal_input_subscription,
            _terminal_focus_subscription: None,
            terminal_focus,
            terminal_find,
        }
    }

    pub(crate) fn for_session<T: 'static>(
        session_id: &str,
        status: SessionStatus,
        app_store: &Entity<AppStore>,
        window: &mut Window,
        cx: &mut Context<T>,
        on_write: impl FnMut(&mut T, Result<(), String>, &mut Context<T>) + 'static,
    ) -> Option<Self> {
        if status != SessionStatus::Running {
            return None;
        }
        let store = app_store.read(cx);
        let terminal = store.bridge.session(session_id)?;
        let style = terminal_style_for(&store.settings);
        let settings = store.settings.clone();
        let mut attached = Self::new(terminal, &style, &settings, cx, on_write);
        let input = attached.terminal_input.clone();
        attached._terminal_focus_subscription =
            Some(
                cx.on_focus_out(&attached.terminal_focus, window, move |_, _, window, cx| {
                    input.update(cx, |input, input_cx| {
                        if input.cancel_composition() {
                            window.invalidate_character_coordinates();
                            input_cx.notify();
                        }
                    });
                }),
            );
        Some(attached)
    }
}
