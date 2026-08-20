//! App surfaces and the pure helpers each product area owns.

pub(crate) mod app_shell;
pub(crate) mod chat;
pub(crate) mod chat_lifecycle;
pub(crate) mod crews;
pub(crate) mod mission_composer;
pub(crate) mod mission_feed;
pub(crate) mod mission_markdown;
pub(crate) mod mission_workspace;
pub(crate) mod panes;
pub(crate) mod runners;
pub(crate) mod settings_page;
pub(crate) mod sidebar;
pub(crate) mod sidebar_logic;
pub(crate) mod start_chat;
pub(crate) mod start_mission;

pub(crate) use app_shell::AppRoute;
pub(crate) use crews::CrewSurfaces;
pub(crate) use mission_workspace::MissionWorkspace;
pub(crate) use panes::{adjacent_pane_index, pane_fractions};
pub(crate) use runners::RunnerSurfaces;
pub(crate) use settings_page::SettingsState;
pub(crate) use sidebar::{session_label, ProjectModal, Sidebar};
pub(crate) use start_chat::StartChatModal;
pub(crate) use start_mission::StartMissionModalState;
