//! Native sidebar: durable node tree, project containers, tab rows, and attention.

mod activation;
mod archive;
mod drag;
mod elements;
mod menus;
mod project;
mod rows;
mod rows_render;
mod shortcuts;
mod state;
#[cfg(test)]
mod tests;
mod view;

use elements::sidebar_tab_target;

pub(crate) use archive::{archive_all_confirmation_body, archive_targets_for_chats};
pub(crate) use elements::{
    default_session_label, direct_chat_display_status, session_label, session_label_live,
};

use super::*;
use crate::surfaces::sidebar_logic::{
    AttentionState, DropTarget, NumberedSidebarRow, SidebarActivationTarget, SidebarShortcutRow,
};
use crate::*;
use gpui::WeakEntity;
use runner_app::ui::TextField;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::ops::project::ProjectScope;
use runner_backend::repo::node::{NodeRow, NodeType};

#[derive(Clone, Copy)]
enum ArchiveErrorTarget {
    App,
    Chat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchiveSessionOperation {
    ArchiveChat { running: bool },
    CloseTerminal,
}

#[derive(Clone)]
enum SidebarRow {
    Tab {
        node: NodeRow,
        layout: PaneLayout,
        members: Vec<DirectSessionEntry>,
        attention: AttentionState,
    },
    Mission {
        node: NodeRow,
        summary: MissionSummary,
        attention: AttentionState,
    },
}

impl SidebarRow {
    fn node(&self) -> &NodeRow {
        match self {
            Self::Tab { node, .. } | Self::Mission { node, .. } => node,
        }
    }

    fn attention(&self) -> AttentionState {
        match self {
            Self::Tab { attention, .. } | Self::Mission { attention, .. } => *attention,
        }
    }

    fn is_live(&self) -> bool {
        match self {
            Self::Tab { members, .. } => members
                .iter()
                .any(|member| member.status == SessionStatus::Running),
            Self::Mission { summary, .. } => summary.any_session_live,
        }
    }

    fn shortcut_row(&self, nodes: &[NodeRow]) -> SidebarShortcutRow {
        let node = self.node();
        let target = match self {
            Self::Tab {
                layout, members, ..
            } => {
                let target = sidebar_tab_target(layout, members);
                SidebarActivationTarget::Tab {
                    tab_id: node.id.clone(),
                    session_id: target.session_id.clone(),
                }
            }
            Self::Mission { summary, .. } => SidebarActivationTarget::Mission {
                mission_id: summary.mission.id.clone(),
                project_id: node_project_id(nodes, node),
            },
        };
        SidebarShortcutRow {
            node_id: node.id.clone(),
            target,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SidebarRenameTarget {
    Tab {
        node_id: String,
        original: String,
        // Single-pane tabs carry their name on the session, the way the pane
        // header renames them; grouped tabs name the tab node.
        session_id: Option<String>,
    },
    Project {
        project_id: String,
        original: String,
    },
    Mission {
        mission_id: String,
        original: String,
    },
}

impl SidebarRenameTarget {
    fn matches(&self, kind: NodeType, id: &str) -> bool {
        match (self, kind) {
            (Self::Tab { node_id, .. }, NodeType::Tab) => node_id == id,
            (Self::Project { project_id, .. }, NodeType::Project) => project_id == id,
            (Self::Mission { mission_id, .. }, NodeType::Mission) => mission_id == id,
            _ => false,
        }
    }
}

pub(crate) struct SidebarRename {
    target: SidebarRenameTarget,
    input: Entity<TextField>,
}

pub(crate) struct ProjectModal {
    cwd: Entity<TextField>,
    name: Entity<TextField>,
    browse_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    error: Option<String>,
    submitting: bool,
}

#[derive(Clone)]
struct SidebarNodeDrag {
    node_id: String,
    label: String,
    icon: Option<(ChatIcon, bool)>,
}

impl Render for SidebarNodeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(rems(220. / 16.))
            .px_3()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(theme::sidebar_selected_border())
            .bg(theme::sidebar_selected())
            .shadow_lg()
            .text_size(theme::text_body())
            .text_color(theme::text())
            .when(self.icon.is_some(), |row| row.flex().items_center().gap_2())
            .children(
                self.icon
                    .map(|(icon, live)| elements::sidebar_icon(icon, live)),
            )
            .child(div().min_w(px(0.)).truncate().child(self.label.clone()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SidebarMenuAction {
    NewChat(ProjectScope),
    NewTerminal(ProjectScope),
    NewMission(ProjectScope),
    TogglePin {
        node_id: String,
        pinned: bool,
    },
    Rename(SidebarRenameTarget),
    ForkChat(String),
    ArchiveTab {
        tab_id: String,
        session_ids: Vec<String>,
    },
    CloseTerminalTab {
        tab_id: String,
        session_id: String,
    },
    ArchiveMission(String),
    DeleteProject(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceEntry {
    NewTab,
    Role,
    Crew,
}

const WORKSPACE_ENTRIES: [WorkspaceEntry; 3] = [
    WorkspaceEntry::NewTab,
    WorkspaceEntry::Role,
    WorkspaceEntry::Crew,
];

impl WorkspaceEntry {
    fn selectable(self) -> bool {
        !matches!(self, Self::NewTab)
    }

    fn selected(self, route: &AppRoute) -> bool {
        if !self.selectable() {
            return false;
        }
        match self {
            Self::NewTab => unreachable!(),
            Self::Role => matches!(route, AppRoute::Roles | AppRoute::RoleDetail(_)),
            Self::Crew => matches!(route, AppRoute::Crews | AppRoute::CrewEditor(_)),
        }
    }
}

pub(crate) struct Sidebar {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    store_revisions: StoreRevisions,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    create_menu: Entity<PopoverMenu>,
    context_menu: Option<Entity<ContextMenu>>,
    rename: Option<SidebarRename>,
    live_titles: HashMap<String, String>,
    archiving_sessions: HashSet<String>,
    archiving_missions: HashSet<String>,
    active_project_id: Option<String>,
    window_id: u64,
    dragged_id: Option<String>,
    drop_target: Option<DropTarget>,
    drop_marker: Option<String>,
    cmd_held_since: Option<Instant>,
    shortcut_key_pressed: bool,
    show_shortcut_pills: bool,
    numbered_shortcut_rows: Vec<NumberedSidebarRow>,
    tab_index_by_node: HashMap<String, u8>,
    _rename_focus_subscription: Option<Subscription>,
    _store_subscription: Subscription,
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_id = window.window_handle().window_id().as_u64();
        self.render_sidebar_contents(cx)
    }
}

fn node_project_id(nodes: &[NodeRow], node: &NodeRow) -> Option<String> {
    let parent = nodes
        .iter()
        .find(|candidate| node.parent_id.as_deref() == Some(candidate.id.as_str()))?;
    (parent.node_type == NodeType::Project)
        .then(|| parent.ref_id.clone())
        .flatten()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SidebarForkMenuTarget {
    session_id: String,
    disabled: bool,
    description: Option<&'static str>,
}
