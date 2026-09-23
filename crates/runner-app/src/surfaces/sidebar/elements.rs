use runner_backend::model::Runtime;
use std::path::Path;

use super::*;
use crate::surfaces::sidebar_logic::AttentionState;
use crate::surfaces::*;
use crate::*;
use gpui::{radians, svg, FontWeight, Hsla, Transformation};
use runner_app::ui::{focus_ring, Tooltip};

pub(super) fn project_name_from_path(path: &str) -> String {
    Path::new(path.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn section_title(label: &'static str) -> AnyElement {
    div()
        .px_5()
        .pb_2()
        .text_size(theme::text_caption())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

pub(super) fn workspace_new_chat_row(
    shortcut: Option<String>,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .id("workspace-new-chat")
        .w_full()
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .flex()
        .items_center()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .hover(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
        })
        .child(
            svg()
                .path("message-square-plus.svg")
                .size(rems(12. / 16.))
                .flex_none()
                .text_color(theme::accent()),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .text_size(theme::text_title())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child("New chat"),
        )
        .children(shortcut.map(|shortcut| {
            div()
                .flex_none()
                .text_size(theme::text_meta())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::faint())
                .child(shortcut)
        }))
        .on_click(move |_, window, cx| on_click(window, cx))
        .into_any_element()
}

pub(super) fn project_row_action(
    id: SharedString,
    icon: &'static str,
    icon_size: f32,
    tooltip: &'static str,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let id = gpui::ElementId::from(id);
    let tooltip_id = (id.clone(), "tooltip");
    let on_press = Rc::new(on_press);
    let key_press = Rc::clone(&on_press);
    let button = div()
        .id(id)
        .tab_index(0)
        .tab_stop(true)
        .size(rems(1.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .opacity(0.)
        .group_hover("sidebar-row-actions", |button| button.opacity(1.))
        .cursor_pointer()
        .hover(|button| button.bg(theme::raised()))
        .focus_visible(|button| {
            button
                .opacity(1.)
                .shadow(focus_ring(theme::border_strong()))
        })
        .child(
            svg()
                .flex_none()
                .path(icon)
                .size(rems(icon_size / 16.))
                .text_color(theme::muted())
                .group_hover("sidebar-row-actions", |icon| icon.text_color(theme::text())),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_press(window, cx);
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_press(window, cx);
            }
        });
    Tooltip::new(tooltip_id, tooltip, button).into_any_element()
}

pub(super) fn workspace_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    active: bool,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_click = Rc::new(on_click);
    let key_click = Rc::clone(&on_click);
    div()
        .id(id)
        .tab_index(0)
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .flex()
        .items_center()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(if active {
            theme::sidebar_selected_border()
        } else {
            gpui::transparent_black()
        })
        .when(active, |row| row.bg(theme::sidebar_selected()).shadow_sm())
        .cursor_pointer()
        .text_size(theme::text_title())
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
        .focus_visible(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
        .child(
            svg()
                .path(icon)
                .size(rems(12. / 16.))
                .flex_none()
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                }),
        )
        .child(label)
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_click(window, cx);
            }
        })
        .into_any_element()
}

pub(super) fn empty_sidebar_label(label: &'static str) -> AnyElement {
    div()
        .px(rems(10. / 16.))
        .py_1()
        .text_size(theme::text_body())
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

pub(super) fn sidebar_row_shell(
    id: SharedString,
    selected: bool,
    accent_bar: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .group("sidebar-row-actions")
        .w_full()
        .px(rems(10. / 16.))
        .py(rems(6. / 16.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_sm()
        .border_1()
        .border_color(if selected {
            theme::sidebar_selected_border()
        } else {
            gpui::transparent_black()
        })
        .when(selected, |row| row.bg(theme::sidebar_selected()))
        .when(accent_bar, |row| {
            row.child(
                div()
                    .absolute()
                    .left_0()
                    .top(rems(2. / 16.))
                    .bottom(rems(2. / 16.))
                    .w(rems(2. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
        })
        .cursor_pointer()
        .text_size(theme::text_body())
        .text_color(if selected {
            theme::text()
        } else {
            theme::muted()
        })
        .hover(|row| {
            row.border_color(theme::sidebar_selected_border())
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.4))
                .text_color(theme::text())
        })
}

pub(super) fn sidebar_row_label(label: String, selected: bool, monospace: bool) -> AnyElement {
    div()
        .min_w(px(0.))
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .when(monospace, |label| {
            label.font_family(theme::UI_MONOSPACE_FONT)
        })
        .font_weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .child(label)
        .into_any_element()
}

pub(super) fn sidebar_row_trailing_slot() -> gpui::Div {
    div()
        .w(rems(34. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap(rems(6. / 16.))
}

pub(super) fn chat_tab_row_active(
    route: &AppRoute,
    active_tab_id: Option<&str>,
    node_id: &str,
) -> bool {
    matches!(route, AppRoute::Chat) && active_tab_id == Some(node_id)
}

pub(super) fn sidebar_tab_target<'a>(
    layout: &PaneLayout,
    members: &'a [DirectSessionEntry],
) -> &'a DirectSessionEntry {
    let focused = layout
        .focused_session_id()
        .or_else(|| members.first().map(|member| member.session_id.as_str()))
        .unwrap_or_default();
    members
        .iter()
        .find(|member| member.session_id == focused)
        .unwrap_or(&members[0])
}

pub(super) fn sidebar_fork_menu_target(
    layout: &PaneLayout,
    members: &[DirectSessionEntry],
) -> Option<SidebarForkMenuTarget> {
    if layout.root.leaves().len() != 1 {
        return None;
    }
    let entry = members
        .first()
        .filter(|entry| Runtime::parse(&entry.agent_runtime) != Some(Runtime::Shell))?;
    let (disabled, description) = if !entry.native_fork {
        (true, None)
    } else if !entry.forkable {
        (true, Some("No session key captured yet"))
    } else {
        (false, None)
    };
    Some(SidebarForkMenuTarget {
        session_id: entry.session_id.clone(),
        disabled,
        description,
    })
}

pub(super) fn project_row_label(label: String) -> AnyElement {
    div()
        .min_w(px(0.))
        .flex_1()
        .overflow_hidden()
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .child(label)
        .into_any_element()
}

pub(super) fn sidebar_icon(icon: ChatIcon, live: bool) -> AnyElement {
    svg()
        .path(icon.path)
        .size(rems(12. / 16.))
        .flex_none()
        .text_color(sidebar_icon_color(icon, live))
        .into_any_element()
}

pub(super) fn sidebar_icon_color(icon: ChatIcon, live: bool) -> Hsla {
    icon.color(
        theme::with_alpha(theme::text(), if live { 1. } else { 0.45 }),
        live,
    )
}

/// Leading pin glyph. It stays put while the ⌘ shortcut pills show: the
/// pills only replace the trailing slot.
pub(super) fn pin_indicator() -> AnyElement {
    svg()
        .path("pin.svg")
        .size(rems(10. / 16.))
        .flex_none()
        .text_color(theme::faint())
        .with_transformation(Transformation::rotate(radians(
            -std::f32::consts::FRAC_PI_4,
        )))
        .into_any_element()
}

pub(super) fn tab_shortcut_pill(index: u8, selected: bool) -> AnyElement {
    div()
        .size(rems(1.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(4. / 16.))
        .bg(theme::raised())
        .font_family(theme::UI_MONOSPACE_FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_size(theme::text_caption())
        .text_color(if selected {
            theme::text()
        } else {
            theme::muted()
        })
        .child(index.to_string())
        .into_any_element()
}

pub(super) fn other_modifiers_held(modifiers: gpui::Modifiers) -> bool {
    crate::platform_ui::other_shortcut_modifiers_held(modifiers)
}

pub(super) fn command_held_alone(modifiers: gpui::Modifiers) -> bool {
    crate::platform_ui::primary_modifier_held(modifiers) && !other_modifiers_held(modifiers)
}

pub(super) fn attention_indicator(attention: AttentionState) -> AnyElement {
    let slot = div()
        .size(rems(12. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    match attention {
        AttentionState::Working => slot
            .child(runner_app::ui::button::spinner(
                "sidebar-working-indicator",
                12.,
                theme::faint(),
            ))
            .into_any_element(),
        AttentionState::Unread => slot
            .child(
                div()
                    .size(rems(6. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
            .into_any_element(),
        AttentionState::Unavailable | AttentionState::NeedsYou | AttentionState::Error => slot
            .child(
                svg()
                    .path(match attention {
                        AttentionState::Error => "circle-alert.svg",
                        AttentionState::NeedsYou => "triangle-alert.svg",
                        _ => "circle-question-mark.svg",
                    })
                    .size(px(12.))
                    .text_color(match attention {
                        AttentionState::Error => theme::danger(),
                        AttentionState::NeedsYou => theme::warning(),
                        _ => theme::muted(),
                    }),
            )
            .into_any_element(),
        AttentionState::None => slot.into_any_element(),
    }
}

pub(crate) fn direct_chat_display_status(
    session: &DirectSessionEntry,
    status: Option<&runner_backend::session::status::AgentStatus>,
) -> runner_app::ui::agent_status::StatusPresentation {
    use runner_backend::session::status::{AgentStatus, Lifecycle};
    let mut status = status.cloned().unwrap_or(AgentStatus {
        lifecycle: Lifecycle::Running,
        ..Default::default()
    });
    status.lifecycle = match session.status {
        SessionStatus::Stopped => Lifecycle::Stopped,
        SessionStatus::Crashed => Lifecycle::Error,
        SessionStatus::Running => status.lifecycle,
    };
    runner_app::ui::agent_status::StatusPresentation::new(&status)
}

pub(super) fn tab_label_live(
    layout: &PaneLayout,
    sessions: &[DirectSessionEntry],
    live_title: impl Fn(&str) -> Option<String>,
) -> String {
    if let Some(name) = &layout.name {
        return name.clone();
    }
    layout
        .session_ids()
        .iter()
        .find_map(|id| sessions.iter().find(|entry| entry.session_id == *id))
        .map(|entry| session_label_live(entry, live_title(&entry.session_id).as_deref()))
        .unwrap_or_else(|| "Empty tab".into())
}

pub(crate) fn session_label(entry: &DirectSessionEntry) -> String {
    session_label_live(entry, None)
}

pub(crate) fn session_label_live(entry: &DirectSessionEntry, live: Option<&str>) -> String {
    entry
        .preferred_title(live)
        .unwrap_or_else(|| default_session_label(entry))
}

pub(crate) fn default_session_label(entry: &DirectSessionEntry) -> String {
    default_session_label_parts(
        &entry.agent_runtime,
        &entry.agent_command,
        entry.handle.as_deref(),
        &entry.display_name,
    )
}

pub(super) fn default_session_label_parts(
    runtime: &str,
    command: &str,
    handle: Option<&str>,
    display_name: &str,
) -> String {
    if Runtime::parse(runtime) == Some(Runtime::Shell) {
        return std::path::Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("shell")
            .to_owned();
    }
    handle
        .map(|handle| format!("@{handle}"))
        .unwrap_or_else(|| display_name.to_owned())
}
