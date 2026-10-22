use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{div, rems, svg, App, FontWeight, KeyDownEvent, SharedString};
use runner_core::protocol::terminal::TerminalController;
use runner_terminal::terminal::TerminalControl;

use crate::{theme, AttachedChat, NativeRoot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardState {
    name: String,
    left: bool,
    count: usize,
}

pub(crate) fn card_state(controls: &[TerminalControl]) -> Option<CardState> {
    let first = controls.first()?;
    let (name, left) = match &first.controller {
        TerminalController::Desktop => return None,
        TerminalController::Browser { name, .. } => (name.clone(), false),
        TerminalController::None => (
            first
                .last_remote_name
                .clone()
                .unwrap_or_else(|| crate::i18n::text("remote.another_computer").into()),
            true,
        ),
    };
    let same = controls.iter().all(|control| match &control.controller {
        TerminalController::Browser { name: other, .. } => !left && *other == name,
        TerminalController::None => left && control.last_remote_name == first.last_remote_name,
        TerminalController::Desktop => false,
    });
    same.then_some(CardState {
        name,
        left,
        count: controls.len(),
    })
}

pub(crate) fn group_covered<'a>(chats: impl IntoIterator<Item = Option<&'a AttachedChat>>) -> bool {
    let controls: Option<Vec<_>> = chats
        .into_iter()
        .map(|chat| Some(chat?.terminal.control()))
        .collect();
    controls.is_some_and(|controls| controls.len() > 1 && card_state(&controls).is_some())
}

#[derive(Clone, Copy)]
pub(crate) enum GroupKind {
    Tab,
    Mission(f32),
}

pub(crate) fn mission_covered<'a>(
    chats: impl IntoIterator<Item = Option<&'a AttachedChat>>,
) -> bool {
    let controls: Option<Vec<_>> = chats
        .into_iter()
        .map(|chat| Some(chat?.terminal.control()))
        .collect();
    controls.is_some_and(|controls| card_state(&controls).is_some())
}

pub(crate) fn cover<E: ParentElement + InteractiveElement>(
    surface: E,
    id: impl Into<SharedString>,
    chats: &[&AttachedChat],
    group_kind: GroupKind,
) -> E {
    let Some(state) = card_state(
        &chats
            .iter()
            .map(|chat| chat.terminal.control())
            .collect::<Vec<_>>(),
    ) else {
        return surface;
    };
    let id = id.into();
    let targets = chats
        .iter()
        .map(|chat| {
            (
                Arc::clone(&chat.terminal),
                chat.terminal_interaction.clone(),
            )
        })
        .collect::<Vec<_>>();
    let take: runner_app::ui::PressHandler = Rc::new(move |_, cx| take_back_targets(&targets, cx));
    let key_take = take.clone();
    surface
        .capture_key_down(move |event: &KeyDownEvent, window, cx| {
            if !event.keystroke.modifiers.platform
                && !matches!(event.keystroke.key.as_str(), "tab" | "space")
            {
                cx.stop_propagation();
                if event.keystroke.key == "enter" && !event.is_held {
                    key_take(window, cx);
                }
            }
        })
        .child(card(id, state, group_kind, take))
}

fn card(
    id: SharedString,
    state: CardState,
    group_kind: GroupKind,
    take: runner_app::ui::PressHandler,
) -> impl IntoElement {
    let title = crate::i18n::tf(
        if state.left {
            "remote.control_left"
        } else {
            "remote.control_title"
        },
        &[("name", state.name.clone())],
    );
    let detail = crate::i18n::tf(
        match (state.left, state.count > 1) {
            (true, false) => "remote.control_left_detail",
            (true, true) => "remote.control_group_left_detail",
            (false, false) => "remote.control_detail",
            (false, true) => match group_kind {
                GroupKind::Tab => "remote.control_group_detail",
                GroupKind::Mission(_) => "remote.control_mission_detail",
            },
        },
        &[("name", state.name), ("count", state.count.to_string())],
    );
    let selector = format!("REMOTE_CONTROL_OVERLAY_{id}");
    div()
        .debug_selector(move || selector.clone())
        .absolute()
        .inset_0()
        .when_some(
            match group_kind {
                GroupKind::Tab => None,
                GroupKind::Mission(height) => Some(height),
            },
            |scrim, height| scrim.top(rems(height / 16.)),
        )
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .occlude()
        .bg(theme::with_alpha(theme::bg(), 0.85))
        .child(
            div()
                .w_full()
                .max_w(rems(400. / 16.))
                .flex()
                .flex_col()
                .items_center()
                .gap_4()
                .rounded_xl()
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .p_6()
                .text_center()
                .shadow_2xl()
                .flex_none()
                .debug_selector(|| "REMOTE_CONTROL_CARD".into())
                .child(
                    div()
                        .size(rems(44. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        .flex_none()
                        .child(
                            svg()
                                .path(if state.left {
                                    "unplug.svg"
                                } else {
                                    "globe.svg"
                                })
                                .size(rems(20. / 16.))
                                .text_color(if state.left {
                                    theme::muted()
                                } else {
                                    theme::accent()
                                }),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(rems(6. / 16.))
                        .child(
                            div()
                                .debug_selector(|| "REMOTE_CONTROL_TITLE".into())
                                .whitespace_normal()
                                .text_size(theme::text_lead())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .debug_selector(|| "REMOTE_CONTROL_DETAIL".into())
                                .whitespace_normal()
                                .text_size(theme::text_body())
                                .line_height(rems(20. / 16.))
                                .text_color(theme::muted())
                                .child(detail),
                        ),
                )
                .child(
                    div()
                        .debug_selector(|| "REMOTE_CONTROL_ACTION".into())
                        .pt(rems(2. / 16.))
                        .child(
                            div()
                                .id(id)
                                .tab_index(0)
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded(rems(6. / 16.))
                                .bg(theme::accent())
                                .px(rems(14. / 16.))
                                .py(rems(8. / 16.))
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::accent_ink())
                                .cursor_pointer()
                                .focus_visible(|button| {
                                    button.shadow(runner_app::ui::focus_ring(theme::accent()))
                                })
                                .hover(|button| button.bg(theme::with_alpha(theme::accent(), 0.9)))
                                .on_click({
                                    let take = take.clone();
                                    move |_, window, cx| {
                                        cx.stop_propagation();
                                        take(window, cx);
                                    }
                                })
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        take(window, cx);
                                    }
                                })
                                .child(
                                    svg()
                                        .path("monitor.svg")
                                        .size(rems(14. / 16.))
                                        .text_color(theme::accent_ink()),
                                )
                                .child(crate::i18n::text(if state.left {
                                    "remote.continue_here"
                                } else {
                                    "remote.take_back"
                                })),
                        ),
                ),
        )
}

pub(crate) fn measure_panes<E: ParentElement>(
    surface: E,
    chats: &[&AttachedChat],
    style: super::element::TerminalStyle,
) -> E {
    let interactions = chats
        .iter()
        .map(|chat| chat.terminal_interaction.clone())
        .collect::<Vec<_>>();
    surface.child(
        div()
            .absolute()
            .inset_0()
            .flex()
            .py_3()
            .pl_3()
            .pr_1()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(gpui::px(0.))
                    .min_h(gpui::px(0.))
                    .pr(runner_app::ui::terminal_scrollbar_gutter())
                    .child(
                        gpui::canvas(
                            move |bounds, window, cx| {
                                let font_size = gpui::px(style.font_size);
                                let font = window.text_system().resolve_font(&style.font);
                                let cell_width = window
                                    .text_system()
                                    .em_advance(font, font_size)
                                    .unwrap_or(gpui::px(style.font_size * 0.6));
                                let line_height =
                                    (style.font_size * super::element::LINE_HEIGHT_FACTOR).round();
                                let size = runner_app::terminal_resize::terminal_grid_size(
                                    f32::from(bounds.size.width),
                                    f32::from(bounds.size.height),
                                    f32::from(cell_width),
                                    line_height,
                                );
                                for interaction in &interactions {
                                    interaction
                                        .update(cx, |interaction, _| interaction.pane_size = size);
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .size_full(),
                    ),
            ),
    )
}

pub(crate) fn take_back(chats: &[&AttachedChat], cx: &mut App) {
    let targets = chats
        .iter()
        .map(|chat| {
            (
                Arc::clone(&chat.terminal),
                chat.terminal_interaction.clone(),
            )
        })
        .collect::<Vec<_>>();
    take_back_targets(&targets, cx);
}

fn take_back_targets(
    targets: &[(
        Arc<runner_terminal::terminal::TerminalMirror>,
        gpui::Entity<super::TerminalInteraction>,
    )],
    cx: &mut App,
) {
    for (terminal, interaction) in targets {
        let size = interaction.read(cx).pane_size;
        let (cols, rows) = size.map_or_else(|| terminal.size(), |size| (size.cols, size.rows));
        if let Err(error) = terminal.take_back(cols, rows) {
            if let Some(window) = cx
                .active_window()
                .and_then(|window| window.downcast::<NativeRoot>())
            {
                let message = error.to_string();
                cx.defer(move |cx| {
                    let _ = window.update(cx, |root, _, cx| {
                        root.show_toast(message, crate::toast::ToastTone::Error, cx)
                    });
                });
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
