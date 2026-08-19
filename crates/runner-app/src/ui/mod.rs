pub mod avatar;
pub mod button;
pub mod copy_value_button;
pub mod field;
pub mod list;
pub mod menu;
pub mod model_field;
pub mod overlay;
pub mod scrollbar;
pub mod select;
pub mod session_control;
pub mod session_overlay;
pub mod settings;
pub mod surfaces;
pub mod toggle;
pub mod tooltip;

use gpui::{Pixels, Window};

pub(crate) fn app_zoom(window: &Window) -> f32 {
    zoom_from_rem_size(window.rem_size())
}

fn zoom_from_rem_size(rem_size: Pixels) -> f32 {
    f32::from(rem_size) / 16.
}

pub use avatar::{
    cells_for_seed, hue_for_seed, lead_badge, AvatarHue, RunnerAvatar, RunnerPresence,
};
pub use button::{Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize, PressHandler};
pub use copy_value_button::CopyValueButton;
pub use field::{
    effective_working_dir, working_dir_placeholder, working_dir_text_field, Field, FieldError,
    FieldValidation, Label, TextField, TextFieldKind, WorkingDirField,
};
pub use list::{
    clamp_page, page_window, EmptyStateCard, PageHandler, PageWindowItem, Pager, PaginatedListPage,
    SearchHandler, SearchInput, PAGE_SIZE,
};
pub use menu::{ContextMenu, MenuAction, MenuItem, MenuKey, MenuState, PopoverMenu};
pub use model_field::ModelField;
pub use overlay::{ConfirmDialog, Drawer, Modal, OverlayWidth};
pub use scrollbar::{Scrollbar, ScrollbarKind, ScrollbarMetrics};
pub use select::{
    runtime_select_options, RuntimeSelect, SelectHandler, SelectOption, SelectState, StyledSelect,
};
pub use session_control::{SessionControl, SessionControlKind, SessionControlVariant};
pub use session_overlay::{SessionOverlay, SessionOverlayKind};
pub use settings::{PaneHeader, SettingsCard, SettingsHeader, SettingsRow, StepHandler, Stepper};
pub use surfaces::{pill, status_badge, Badge, Card, Tone};
pub use toggle::{Toggle, ToggleHandler};
pub use tooltip::Tooltip;

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::zoom_from_rem_size;

    #[test]
    fn app_zoom_tracks_the_window_rem_size() {
        assert_eq!(zoom_from_rem_size(px(12.8)), 0.8);
        assert_eq!(zoom_from_rem_size(px(16.)), 1.);
        assert_eq!(zoom_from_rem_size(px(24.)), 1.5);
    }
}
