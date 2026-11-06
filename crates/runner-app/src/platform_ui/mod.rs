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

thread_local! {
    static TITLEBAR_PRESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Capture-phase half of a titlebar press: a first click anywhere over a drag
/// area, including on a control inside it, forgets the last titlebar press.
pub(crate) fn capture_titlebar_press(event: &gpui::MouseDownEvent) {
    if event.click_count == 1 {
        TITLEBAR_PRESS.set(false);
    }
}

/// Records a press that reached a drag area and says whether it completes a
/// titlebar double-click. Both clicks must land on the drag area: a control
/// that hides itself on the first click must not turn the second into a zoom
/// on whatever it uncovered.
pub(crate) fn titlebar_double_click(click_count: usize) -> bool {
    if click_count == 1 {
        TITLEBAR_PRESS.set(true);
    }
    click_count == 2 && TITLEBAR_PRESS.get()
}

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
