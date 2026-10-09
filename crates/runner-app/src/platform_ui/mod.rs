#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::{
    activate_update_hint, finish_window_close, navigation_left, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, update_hint_tooltip,
    PRIMARY_MODIFIER, SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET, SHORTCUT_REQUIREMENTS,
};
#[cfg(windows)]
pub(crate) use windows::{
    activate_update_hint, finish_window_close, navigation_left, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, update_hint_tooltip,
    PRIMARY_MODIFIER, SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET, SHORTCUT_REQUIREMENTS,
};
