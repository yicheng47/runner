#[cfg(any(windows, target_os = "linux"))]
mod desktop;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(all(target_os = "macos", not(test)))]
pub(crate) use macos::show_about_panel;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    finish_window_close, initial_window_bounds, navigation_left, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, sync_traffic_lights,
    DESKTOP_APP_ID, PRIMARY_MODIFIER, SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET,
    SHORTCUT_REQUIREMENTS_KEY,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    activate_update_hint, finish_window_close, navigation_left, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, sync_traffic_lights,
    update_hint_tooltip, PRIMARY_MODIFIER, SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET,
    SHORTCUT_REQUIREMENTS_KEY,
};
#[cfg(windows)]
pub(crate) use windows::{
    activate_update_hint, finish_window_close, navigation_left, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, sync_traffic_lights,
    update_hint_tooltip, PRIMARY_MODIFIER, SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET,
    SHORTCUT_REQUIREMENTS_KEY,
};
