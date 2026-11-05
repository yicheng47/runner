use super::*;

pub(crate) const TERMINAL_DRAWER_HEADER_HEIGHT: f32 = 40.;

pub(crate) type TerminalDrawerSessionCallback = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub(crate) type TerminalDrawerActionCallback = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct TerminalDrawerCallbacks {
    pub can_add: bool,
    pub can_close: bool,
    pub activate: TerminalDrawerSessionCallback,
    pub close: TerminalDrawerSessionCallback,
    pub add: TerminalDrawerActionCallback,
    pub hide: TerminalDrawerActionCallback,
}

pub(crate) fn render_terminal_drawer_strip(
    id_prefix: &str,
    shells: &[String],
    active_id: Option<&str>,
    labels: &[String],
    callbacks: TerminalDrawerCallbacks,
) -> AnyElement {
    let TerminalDrawerCallbacks {
        can_add,
        can_close,
        activate,
        close,
        add,
        hide,
    } = callbacks;
    let chips = shells
        .iter()
        .enumerate()
        .map(|(index, session_id)| {
            let active = active_id == Some(session_id.as_str());
            let label = labels.get(index).cloned().unwrap_or_else(|| "shell".into());
            let activate = Rc::clone(&activate);
            let close = Rc::clone(&close);
            let activate_id = session_id.clone();
            let close_id = session_id.clone();
            div()
                .id(SharedString::from(format!(
                    "{id_prefix}-drawer-chip-{session_id}"
                )))
                .flex_none()
                .h(rems(24. / 16.))
                .max_w(rems(180. / 16.))
                .px_2()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .rounded(rems(5. / 16.))
                .bg(if active {
                    theme::raised()
                } else {
                    gpui::transparent_black()
                })
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .cursor_pointer()
                .hover(|chip| chip.bg(theme::raised()))
                .child(
                    svg()
                        .path("terminal.svg")
                        .size(rems(12. / 16.))
                        .flex_none()
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .child(label),
                )
                .children(can_close.then(|| {
                    IconButton::new(
                        SharedString::from(format!("{id_prefix}-close-drawer-shell-{session_id}")),
                        "close.svg",
                    )
                    .size(IconButtonSize::Xs)
                    .stop_click_propagation(true)
                    .tooltip(crate::i18n::text("ui.close_terminal"))
                    .on_press(move |window, cx| close(close_id.clone(), window, cx))
                }))
                .on_click(move |_, window, cx| activate(activate_id.clone(), window, cx))
        })
        .collect::<Vec<_>>();
    let add_id = SharedString::from(format!("{id_prefix}-add-drawer-shell"));
    let hide_id = SharedString::from(format!("{id_prefix}-hide-terminal-drawer"));
    div()
        .flex_none()
        .h(rems(TERMINAL_DRAWER_HEADER_HEIGHT / 16.))
        .w_full()
        .pl(rems(10. / 16.))
        .pr(rems(10. / 16.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .map(|element| {
            #[cfg(test)]
            let element = crate::theme_snapshot::record_fill("DRAWER_HEADER", element);
            element
        })
        .children(chips)
        .children(can_add.then(|| {
            IconButton::new(add_id, "plus.svg")
                .size(IconButtonSize::Sm)
                .tooltip(crate::i18n::text("ui.new_terminal"))
                .on_press(move |window, cx| add(window, cx))
        }))
        .child(div().flex_1())
        .child(
            IconButton::new(hide_id, "chevron-down.svg")
                .size(IconButtonSize::Sm)
                .tooltip(crate::i18n::text("ui.hide_terminal_drawer"))
                .on_press(move |window, cx| hide(window, cx)),
        )
        .into_any_element()
}

pub(crate) struct TerminalSurface {
    pub(crate) terminal: Arc<TerminalMirror>,
    pub(crate) interaction: Entity<TerminalInteraction>,
    pub(crate) input: Entity<TerminalInput>,
    pub(crate) focus: FocusHandle,
    pub(crate) scrollbar: Entity<Scrollbar>,
    pub(crate) style: crate::terminal::element::TerminalStyle,
    pub(crate) interactive: bool,
    pub(crate) resize_owner: bool,
    pub(crate) scrollable: bool,
    pub(crate) find: Option<Entity<crate::terminal::TerminalFind>>,
    pub(crate) file_links: Option<crate::file_links::FileLinkRoute>,
}

impl TerminalSurface {
    pub(crate) fn render(self) -> Div {
        let background = crate::terminal::element::to_hsla(self.style.palette.background, 1.);
        div()
            .absolute()
            .inset_0()
            .key_context("Terminal")
            .track_focus(&self.focus)
            .flex()
            .py_3()
            .pl_3()
            .pr_1()
            .bg(background)
            .when_some(self.find.clone(), |surface, find| {
                surface.on_action(move |_: &crate::FindInTerminal, window, cx| {
                    find.update(cx, |find, cx| find.open(window, cx));
                })
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .pr(runner_app::ui::terminal_scrollbar_gutter())
                    .child(
                        TerminalElement::new(
                            self.terminal,
                            self.interaction,
                            self.input,
                            self.focus,
                            self.interactive,
                            self.resize_owner,
                            self.style,
                        )
                        .scrollable(self.scrollable)
                        .file_links(self.file_links)
                        .map(|element| match self.find {
                            Some(find) => element.find(find),
                            None => element,
                        })
                        .cached(),
                    )
                    .child(self.scrollbar),
            )
    }
}
