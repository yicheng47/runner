#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::{
    activate_update_hint, finish_window_close, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, update_hint_tooltip,
    PRIMARY_MODIFIER, SHORTCUT_REQUIREMENTS,
};
#[cfg(windows)]
pub(crate) use windows::{
    activate_update_hint, finish_window_close, other_shortcut_modifiers_held,
    primary_modifier_held, sidebar_row_wrapper, sidebar_section, update_hint_tooltip,
    PRIMARY_MODIFIER, SHORTCUT_REQUIREMENTS,
};
