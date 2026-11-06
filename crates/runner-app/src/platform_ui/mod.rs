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
    finish_window_close, navigation_left, other_shortcut_modifiers_held, primary_modifier_held,
    sidebar_row_wrapper, sidebar_section, sync_traffic_lights, PRIMARY_MODIFIER,
    SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET, SHORTCUT_REQUIREMENTS_KEY,
};
#[cfg(windows)]
pub(crate) use windows::{
    finish_window_close, navigation_left, other_shortcut_modifiers_held, primary_modifier_held,
    sidebar_row_wrapper, sidebar_section, sync_traffic_lights, PRIMARY_MODIFIER,
    SETTINGS_CONTENT_TOP, SETTINGS_DRAG_INSET, SHORTCUT_REQUIREMENTS_KEY,
};

#[cfg(any(windows, target_os = "linux"))]
pub(crate) use desktop::titlebar_mouse_down;
#[cfg(target_os = "macos")]
pub(crate) use macos::titlebar_mouse_down;
#[cfg(all(target_os = "macos", test))]
pub(crate) use macos::{take_titlebar_actions, TitlebarAction};

#[cfg(target_os = "linux")]
pub mod fonts_linux;
#[cfg(target_os = "macos")]
pub mod fonts_macos;
#[cfg(windows)]
pub mod fonts_windows;
#[cfg(all(target_os = "macos", not(test)))]
pub(crate) mod material_macos;

#[cfg(target_os = "linux")]
pub use fonts_linux as platform_fonts;
#[cfg(target_os = "macos")]
pub use fonts_macos as platform_fonts;
#[cfg(windows)]
pub use fonts_windows as platform_fonts;
