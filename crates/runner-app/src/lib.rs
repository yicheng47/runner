extern crate self as runner_app;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod theme_snapshot;

actions!(runner_app_ui, [Copy, Cut, Paste, Redo, SelectAll, Undo]);

pub mod appearance;
pub mod bootstrap;
pub mod i18n;
pub mod lifecycle;
pub mod logging;
pub mod pane_layout;
pub mod text_util;
pub mod theme;
pub mod ui;
pub mod updater;
pub mod version;

#[cfg(target_os = "macos")]
mod wake;

#[cfg(test)]
mod render_counts;

mod app_settings;
mod app_store;
mod assets;
#[cfg(test)]
mod catalog_golden;
mod chat_icon;
mod file_icons;
mod file_links;
mod keymap;
mod list_controls;
// The controller's callers are the app entry point and its unix-only tests,
// and a Windows test build compiles neither.
#[cfg_attr(all(test, windows), allow(dead_code))]
mod notifications;
mod platform_ui;
mod runtime_ui;
// Its only caller is the app entry point, which a test build does not compile.
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
mod single_instance;
mod surfaces;
mod terminal;
mod window_state;

mod startup;
mod toast;

pub use platform_ui::platform_fonts;
use startup::app_menus;
use surfaces::root::*;
use surfaces::windowing::*;
pub use terminal::{ime as terminal_ime, paste as terminal_paste, resize as terminal_resize};

#[cfg(not(test))]
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
#[cfg(all(target_os = "macos", not(test)))]
use gpui::QuitMode;
#[cfg(not(target_os = "linux"))]
use gpui::SystemMenuType;
use gpui::{
    actions, div, point, prelude::*, px, relative, rems, size, AnyElement, App, Bounds, Context,
    CursorStyle, DragMoveEvent, Entity, FocusHandle, Global, KeyDownEvent, Menu, MenuItem,
    MouseButton, OsAction, Pixels, ScrollHandle, SharedString, Size, Subscription, TitlebarOptions,
    Window, WindowBounds, WindowOptions,
};
use runner_app::bootstrap::NativePaths;
use runner_app::pane_layout::{
    DropSide, MissionLayout, PaneLayout, PaneLeaf, PaneNode, SplitOrientation, TabSet,
    MAX_DRAWER_HEIGHT, MIN_DRAWER_HEIGHT,
};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::{
    Button, ButtonSize, ConfirmDialog, ContextMenu, CopyValueButton, DuplicateSubjectKind,
    DuplicateSubjectOverlay, IconButton, IconButtonSize, MenuItem as UiMenuItem, PopoverMenu,
    Scrollbar, SessionControlKind, Tooltip, WorkspaceHeader, WORKSPACE_HEADER_HEIGHT,
};
use runner_core::protocol::model::SessionStatus;
use runner_core::protocol::session::DirectSessionEntry;
use runner_core::protocol::session::SessionActivityState;
use runner_core::protocol::DaemonClient;
#[cfg(test)]
use runner_daemon::AppCore;
use runner_terminal::terminal::{TerminalMirror, TerminalView};

use app_settings::{settings_path, AppSettings};
#[cfg(not(test))]
use app_store::CommandInstallSupport;
use app_store::{global_app_store, AppStore, GlobalAppStore, StoreRefreshKind, StoreRevisions};
#[cfg(not(test))]
use assets::{Assets, INTER_FONTS, JETBRAINS_MONO_FONTS};
use chat_icon::ChatIcon;
use runner_app::updater::{global_updater, GlobalUpdater, Updater};
use terminal::{TerminalElement, TerminalInteraction};
use toast::ToastHost;

#[cfg(not(test))]
use surfaces::{pane_close_behavior, PaneCloseBehavior};
use surfaces::{
    AppRoute, CommandPaletteState, CrewSurfaces, MissionWorkspace, PaneKey, ProjectModal,
    RoleSurfaces, SettingsPane, SettingsState, Sidebar, SplitMenuKey, StartChatModal,
};

actions!(
    runner_app_ui,
    [
        AboutRunner,
        CloseFileTab,
        CloseTab,
        CloseWindow,
        CheckForUpdates,
        CommandPalette,
        FindInFile,
        FindInTerminal,
        FocusNextPane,
        FocusPreviousPane,
        Hide,
        HideOthers,
        Minimize,
        MissionTabNext,
        MissionTabPrevious,
        NavigateNextPage,
        NavigatePreviousPage,
        NewTab,
        NewTerminal,
        NextFileTab,
        NewMission,
        ConfirmStartChat,
        NewWindow,
        OpenSettings,
        PreviousFileTab,
        Quit,
        QuitAndStopSessions,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectTab9,
        ShowAll,
        SplitPaneDown,
        SplitPaneRight,
        SearchFiles,
        ResumeFocusedSession,
        StopFocusedSession,
        ToggleFullscreen,
        ToggleMaximizePane,
        ToggleTerminalDrawer,
        ToggleSidebar,
        ZoomFileIn,
        ZoomFileOut,
        ZoomIn,
        ZoomOut,
        ZoomReset
    ]
);

#[cfg(not(test))]
pub fn run() {
    startup::main();
}
