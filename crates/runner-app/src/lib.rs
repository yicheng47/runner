use gpui::actions;

actions!(runner_app_ui, [Copy, Cut, Paste, SelectAll]);

pub mod bootstrap;
pub mod logging;
pub mod pane_layout;
pub mod terminal_ime;
pub mod terminal_paste;
pub mod terminal_resize;
pub mod text_util;
pub mod theme;
pub mod ui;
pub mod updater;
pub mod version;
