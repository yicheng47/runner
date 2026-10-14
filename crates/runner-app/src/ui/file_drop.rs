use gpui::prelude::*;
use gpui::{div, App, Div, ExternalPaths, Window};

use crate::theme;

/// Covers a terminal surface so files dragged in from Finder or Explorer land
/// on it. The highlight and the drop share this one hitbox, so the pane that
/// lights up is always the pane that takes the drop.
pub fn file_drop_target(
    session_id: &str,
    on_drop: impl Fn(&ExternalPaths, &mut Window, &mut App) + 'static,
) -> Div {
    let selector = format!("TERMINAL_FILE_DROP {session_id}");
    div()
        .absolute()
        .inset_0()
        .debug_selector(move || selector)
        .drag_over::<ExternalPaths>(|style, _, _, _| {
            style
                .bg(theme::with_alpha(theme::accent(), 0.12))
                .border_1()
                .border_color(theme::accent())
        })
        .on_drop(on_drop)
}
