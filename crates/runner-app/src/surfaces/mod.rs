//! App surfaces and the pure helpers each product area owns.

pub(crate) mod agent_recovery;
pub(crate) mod agent_update;
pub(crate) mod app_shell;
pub(crate) mod background_tasks;
pub(crate) mod changes;
pub(crate) mod chat;
pub(crate) mod chat_lifecycle;
pub(crate) mod command_palette;
pub(crate) mod crews;
pub(crate) mod files;
pub(crate) mod front_page;
pub(crate) mod mission_workspace;
pub(crate) mod panes;
pub(crate) mod profile_page;
pub(crate) mod quit_dialog;
pub(crate) mod remote_client;
pub(crate) mod render_cache;
pub(crate) mod roles;
pub(crate) mod root;
pub(crate) mod settings;
pub(crate) mod side_panel;
pub(crate) mod sidebar;
pub(crate) mod start_chat;
mod terminal_attachment;
pub(crate) mod update_ui;
pub(crate) mod windowing;
pub(crate) mod worktree_pickers;
pub(crate) mod worktree_restore;

pub(crate) use app_shell::AppRoute;
pub(crate) use command_palette::CommandPaletteState;
pub(crate) use crews::CrewSurfaces;
pub(crate) use mission_workspace::MissionWorkspace;
pub(crate) use panes::{
    adjacent_pane_index, drop_allowed, pane_close_behavior, pane_fractions, split_decision,
    ArchiveAllSource, PaneCloseBehavior, PaneKey, PaneRename, PendingPaneClose, SplitDecision,
    TerminalCloseConfirm, TerminalCloseTarget,
};
pub(crate) use roles::RoleSurfaces;
pub(crate) use settings::page::{SettingsPane, SettingsState};
pub(crate) use sidebar::{session_label, session_label_live, ProjectModal, Sidebar};
pub(crate) use start_chat::StartChatModal;
