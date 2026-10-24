use runner_app::pane_layout::{PaneLayout, PaneLeaf, PaneNode, PaneSplit, SplitOrientation};
use runner_core::companion::{ActionPane, Pane, SplitDirection, Tab};
use runner_core::companion::{CatalogRole, Session};
use runner_core::protocol::{model::Role, session::DirectSessionEntry};
use std::collections::HashSet;

#[derive(Clone)]
pub(crate) struct SessionDisplay {
    pub(crate) session_id: String,
    pub(crate) agent_runtime: String,
    pub(crate) status: runner_core::protocol::model::SessionStatus,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) resumable: bool,
    pub(crate) native_fork: bool,
    pub(crate) forkable: bool,
    pub(crate) pinned: bool,
    pub(crate) local: Option<DirectSessionEntry>,
    pub(crate) archived_at: Option<runner_core::protocol::model::Timestamp>,
    pub(crate) project_id: Option<String>,
}

impl SessionDisplay {
    pub(crate) fn local(entry: &DirectSessionEntry) -> Self {
        Self {
            session_id: entry.session_id.clone(),
            agent_runtime: entry.agent_runtime.clone(),
            status: entry.status,
            title: entry.title.clone(),
            cwd: entry.cwd.clone(),
            resumable: entry.resumable,
            native_fork: entry.native_fork,
            forkable: entry.forkable,
            pinned: entry.pinned,
            local: Some(entry.clone()),
            archived_at: entry.archived_at,
            project_id: entry.project_id.clone(),
        }
    }

    pub(crate) fn remote(
        id: &str,
        session: &Session,
        pinned: bool,
        native_fork: bool,
        resumable: bool,
    ) -> Self {
        use runner_core::companion::{SessionKind, Status};
        Self {
            session_id: id.into(),
            agent_runtime: if session.kind == SessionKind::Shell {
                "shell".into()
            } else {
                session.runtime.clone().unwrap_or_default()
            },
            status: match session.status.status.effective() {
                Status::Stopped => runner_core::protocol::model::SessionStatus::Stopped,
                Status::Error => runner_core::protocol::model::SessionStatus::Crashed,
                _ => runner_core::protocol::model::SessionStatus::Running,
            },
            title: (!session.title.is_empty()).then(|| session.title.clone()),
            cwd: session.cwd.clone(),
            resumable,
            native_fork,
            forkable: native_fork,
            pinned,
            local: None,
            archived_at: None,
            project_id: None,
        }
    }

    pub(crate) fn label(&self, live: Option<&str>) -> String {
        if let Some(local) = &self.local {
            return crate::surfaces::sidebar::session_label_live(local, live);
        }
        self.title.clone().unwrap_or_else(|| self.default_label())
    }

    pub(crate) fn default_label(&self) -> String {
        if let Some(local) = &self.local {
            return crate::surfaces::sidebar::default_session_label(local);
        }
        if self.agent_runtime == "shell" {
            crate::i18n::text("noun.terminal").into()
        } else {
            crate::surfaces::roles::logic::runtime_display_name(&self.agent_runtime)
        }
    }
}

pub(crate) fn action_pane(node: &PaneNode) -> ActionPane {
    match node {
        PaneNode::Leaf(leaf) => ActionPane::Leaf {
            id: leaf.id.clone(),
            session: leaf.session_id.clone(),
        },
        PaneNode::Split(split) => {
            let sum = split.sizes[0] as f64 + split.sizes[1] as f64;
            ActionPane::Split {
                id: split.id.clone(),
                split: match split.orientation {
                    SplitOrientation::Row => SplitDirection::Row,
                    SplitOrientation::Column => SplitDirection::Column,
                },
                sizes: [split.sizes[0] as f64 / sum, split.sizes[1] as f64 / sum],
                a: Box::new(action_pane(&split.a)),
                b: Box::new(action_pane(&split.b)),
            }
        }
    }
}

fn same_topology(node: &PaneNode, pane: &Pane) -> bool {
    match (node, pane) {
        (PaneNode::Leaf(leaf), Pane::Leaf { session }) => &leaf.session_id == session,
        (PaneNode::Split(node), Pane::Split { split, a, b, .. }) => {
            let orientation = if node.orientation == SplitOrientation::Row {
                SplitDirection::Row
            } else {
                SplitDirection::Column
            };
            orientation == *split && same_topology(&node.a, a) && same_topology(&node.b, b)
        }
        _ => false,
    }
}

fn matches_tree(node: &PaneNode, pane: &Pane) -> bool {
    match (node, pane) {
        (PaneNode::Leaf(leaf), Pane::Leaf { session }) => &leaf.session_id == session,
        (PaneNode::Split(node), Pane::Split { split, sizes, a, b }) => {
            let sum = sizes[0] + sizes[1];
            let own_sum = node.sizes[0] + node.sizes[1];
            let orientation = if node.orientation == SplitOrientation::Row {
                SplitDirection::Row
            } else {
                SplitDirection::Column
            };
            orientation == *split
                && ((node.sizes[0] / own_sum) as f64 - sizes[0] / sum).abs() < 0.0001
                && matches_tree(&node.a, a)
                && matches_tree(&node.b, b)
        }
        _ => false,
    }
}

pub(crate) fn matches_layout(layout: &PaneLayout, tab: &Tab) -> bool {
    matches_tree(&layout.root, &tab.panes) && layout.drawer_shells() == tab.shells
}

pub(crate) fn retain_presentation(layout: &mut PaneLayout, previous: &PaneLayout) {
    layout.focus_pane(&previous.focused_pane_id);
    layout.set_drawer_height(previous.drawer_height());
    if let Some(shell) = previous.active_drawer_shell() {
        layout.activate_drawer_shell(shell);
    }
    layout.set_drawer_open(previous.drawer_open());
}

pub(crate) fn reconcile_tab(
    tab: &Tab,
    old: Option<&PaneLayout>,
    hint: Option<&PaneLayout>,
) -> PaneLayout {
    let source = hint.filter(|hint| matches_layout(hint, tab)).or(old);
    let mut nodes = Vec::new();
    fn collect<'a>(node: &'a PaneNode, nodes: &mut Vec<&'a PaneNode>) {
        nodes.push(node);
        if let PaneNode::Split(split) = node {
            collect(&split.a, nodes);
            collect(&split.b, nodes);
        }
    }
    if let Some(source) = source {
        collect(&source.root, &mut nodes);
    }
    fn id(node: &PaneNode) -> &str {
        match node {
            PaneNode::Leaf(n) => &n.id,
            PaneNode::Split(n) => &n.id,
        }
    }
    fn id_number(node: &PaneNode) -> Option<usize> {
        match node {
            PaneNode::Leaf(n) => n.id.get(1..)?.parse().ok(),
            PaneNode::Split(n) => n.id.get(1..)?.parse().ok(),
        }
    }
    let mut used = HashSet::new();
    let mut counter = nodes
        .iter()
        .filter_map(|node| id_number(node))
        .max()
        .unwrap_or_default()
        + 1;
    fn reconcile(
        pane: &Pane,
        position: Option<&PaneNode>,
        nodes: &[&PaneNode],
        used: &mut HashSet<String>,
        counter: &mut usize,
    ) -> PaneNode {
        let exact = nodes
            .iter()
            .copied()
            .find(|node| !used.contains(id(node)) && same_topology(node, pane));
        let candidate = match pane {
            Pane::Leaf {
                session: Some(session),
            } => nodes.iter().copied().find(|node| {
                matches!(node, PaneNode::Leaf(leaf) if leaf.session_id.as_ref() == Some(session))
                    && !used.contains(id(node))
            }),
            Pane::Leaf { session: None } => position.filter(|n| {
                matches!(n, PaneNode::Leaf(leaf) if leaf.session_id.is_none())
                    && !used.contains(id(n))
            }),
            _ => None,
        };
        let empty_position = position.filter(|node| {
            matches!(node, PaneNode::Leaf(leaf) if leaf.session_id.is_none())
                && !used.contains(id(node))
        });
        let existing = exact.or(candidate).or(empty_position);
        let key = existing.map(|n| id(n).to_owned()).unwrap_or_else(|| loop {
            let key = format!(
                "{}{}",
                if matches!(pane, Pane::Split { .. }) {
                    "s"
                } else {
                    "p"
                },
                *counter
            );
            *counter += 1;
            if !nodes.iter().any(|n| id(n) == key) && !used.contains(&key) {
                break key;
            }
        });
        used.insert(key.clone());
        match pane {
            Pane::Split { split, sizes, a, b } => {
                let previous = existing.or(position).and_then(|node| {
                    if let PaneNode::Split(n) = node {
                        Some(n)
                    } else {
                        None
                    }
                });
                let total = sizes[0] + sizes[1];
                let ratio = if sizes.iter().all(|s| s.is_finite() && *s > 0.) && total.is_finite() {
                    (sizes[0] / total) as f32
                } else {
                    0.5
                };
                PaneNode::Split(PaneSplit {
                    id: key,
                    orientation: if *split == SplitDirection::Column {
                        SplitOrientation::Column
                    } else {
                        SplitOrientation::Row
                    },
                    sizes: [ratio * 100., (1. - ratio) * 100.],
                    a: Box::new(reconcile(
                        a,
                        previous.map(|n| n.a.as_ref()),
                        nodes,
                        used,
                        counter,
                    )),
                    b: Box::new(reconcile(
                        b,
                        previous.map(|n| n.b.as_ref()),
                        nodes,
                        used,
                        counter,
                    )),
                })
            }
            Pane::Leaf { session } => PaneNode::Leaf(PaneLeaf {
                id: key,
                session_id: session.clone(),
            }),
            Pane::Unknown(_) => PaneNode::Leaf(PaneLeaf {
                id: key,
                session_id: None,
            }),
        }
    }
    let mut layout = source
        .cloned()
        .unwrap_or_else(|| PaneLayout::single(None, &[]));
    layout.id = tab.id.clone();
    layout.parent_id = tab.project.clone();
    layout.name = (!tab.name.is_empty()).then(|| tab.name.clone());
    layout.root = reconcile(
        &tab.panes,
        source.map(|s| &s.root),
        &nodes,
        &mut used,
        &mut counter,
    );
    if !layout
        .root
        .leaves()
        .iter()
        .any(|leaf| leaf.id == layout.focused_pane_id)
    {
        layout.focused_pane_id = layout.root.leaves()[0].id.clone();
    }
    let selected = layout.active_drawer_shell().map(str::to_owned);
    let open = layout.drawer_open();
    for shell in layout.drawer_shells().to_vec() {
        layout.remove_drawer_shell(&shell);
    }
    for shell in &tab.shells {
        layout.add_drawer_shell(shell.clone());
    }
    if let Some(selected) = selected {
        layout.activate_drawer_shell(&selected);
    }
    layout.set_drawer_open(open);
    layout
}

#[derive(Clone)]
pub(crate) struct RoleChoice {
    pub(crate) id: String,
    pub(crate) handle: String,
    pub(crate) display_name: String,
    pub(crate) runtime: String,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) codex_speed: Option<runner_core::protocol::model::CodexSpeed>,
    pub(crate) working_dir: Option<String>,
}
impl From<Role> for RoleChoice {
    fn from(role: Role) -> Self {
        Self {
            id: role.id,
            handle: role.handle,
            display_name: role.display_name,
            runtime: role.runtime,
            model: role.model,
            effort: role.effort,
            codex_speed: role.codex_speed,
            working_dir: role.working_dir,
        }
    }
}
impl From<CatalogRole> for RoleChoice {
    fn from(role: CatalogRole) -> Self {
        Self {
            id: role.id,
            handle: role.handle,
            display_name: role.name,
            runtime: role.runtime,
            model: role.model,
            effort: role.effort,
            codex_speed: role.speed.as_deref().and_then(|s| match s {
                "standard" => Some(runner_core::protocol::model::CodexSpeed::Standard),
                "fast" => Some(runner_core::protocol::model::CodexSpeed::Fast),
                _ => None,
            }),
            working_dir: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeChoice {
    pub(crate) name: runner_core::protocol::model::Runtime,
    pub(crate) display_name: String,
    pub(crate) command: Option<String>,
    pub(crate) available: bool,
    pub(crate) default_model: Option<String>,
    pub(crate) default_effort: Option<String>,
    pub(crate) models: Vec<runner_core::protocol::runtime::RuntimeCatalogOption>,
    pub(crate) efforts: Vec<runner_core::protocol::runtime::RuntimeCatalogOption>,
    pub(crate) speed: bool,
    pub(crate) effort_needs_launch_model: bool,
}
impl From<runner_core::protocol::runtime::RuntimeCatalogEntry> for RuntimeChoice {
    fn from(runtime: runner_core::protocol::runtime::RuntimeCatalogEntry) -> Self {
        Self {
            name: runtime.name,
            display_name: runtime.display_name,
            command: Some(runtime.command),
            available: runtime.available,
            default_model: runtime.default_model,
            default_effort: runtime.default_effort,
            models: runtime.models,
            efforts: runtime.efforts,
            speed: runtime.capabilities.codex_speed,
            effort_needs_launch_model: runtime.capabilities.effort_needs_launch_model,
        }
    }
}
impl RuntimeChoice {
    pub(crate) fn remote(runtime: &runner_core::companion::CatalogRuntime) -> Option<Self> {
        let name = runner_core::protocol::model::Runtime::parse(&runtime.name)?;
        let option = |value: &runner_core::companion::CatalogOption| {
            runner_core::protocol::runtime::RuntimeCatalogOption {
                value: value.value.clone(),
                label: value.label.clone(),
                description: None,
                supported_efforts: value.supported_efforts.clone(),
            }
        };
        Some(Self {
            name,
            display_name: runtime.label.clone(),
            command: None,
            available: runtime.available,
            default_model: runtime.default_model.clone(),
            default_effort: runtime.default_effort.clone(),
            models: runtime.models.iter().map(option).collect(),
            efforts: runtime.efforts.iter().map(option).collect(),
            speed: runtime.speeds.iter().any(|s| s == "fast"),
            effort_needs_launch_model: runtime.name == "antigravity",
        })
    }
    pub(crate) fn efforts_for_model(
        &self,
        model: &str,
    ) -> Vec<runner_core::protocol::runtime::RuntimeCatalogOption> {
        let model = model.trim();
        let supported = self
            .models
            .iter()
            .find(|option| option.value == model)
            .and_then(|option| option.supported_efforts.as_ref());
        self.efforts
            .iter()
            .filter(|effort| {
                effort.value.is_empty()
                    || (!model.is_empty()
                        && supported.is_none_or(|levels| levels.contains(&effort.value)))
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SidebarNodeDisplay {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) node_type: runner_core::protocol::node::NodeType,
    pub(crate) ref_id: Option<String>,
    pub(crate) position: i64,
    pub(crate) pinned_position: Option<i64>,
    pub(crate) created_at: Option<String>,
}
impl From<runner_core::protocol::node::NodeRow> for SidebarNodeDisplay {
    fn from(node: runner_core::protocol::node::NodeRow) -> Self {
        Self {
            id: node.id,
            parent_id: node.parent_id,
            node_type: node.node_type,
            ref_id: node.ref_id,
            position: node.position,
            pinned_position: node.pinned_position,
            created_at: Some(node.created_at),
        }
    }
}
impl From<runner_core::companion::SidebarNode> for SidebarNodeDisplay {
    fn from(node: runner_core::companion::SidebarNode) -> Self {
        use runner_core::companion::SidebarNodeKind;
        Self {
            ref_id: (!matches!(node.kind, SidebarNodeKind::Tab)).then(|| node.id.clone()),
            id: node.id,
            parent_id: node.parent,
            node_type: match node.kind {
                SidebarNodeKind::Project => runner_core::protocol::node::NodeType::Project,
                SidebarNodeKind::Mission => runner_core::protocol::node::NodeType::Mission,
                _ => runner_core::protocol::node::NodeType::Tab,
            },
            position: node.position,
            pinned_position: node.pinned_position,
            created_at: None,
        }
    }
}
#[derive(Clone)]
pub(crate) struct ProjectDisplay {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) cwd: String,
    pub(crate) local: Option<runner_core::protocol::project::ProjectRow>,
}
impl From<runner_core::protocol::project::ProjectRow> for ProjectDisplay {
    fn from(project: runner_core::protocol::project::ProjectRow) -> Self {
        Self {
            id: project.id.clone(),
            name: project.name.clone(),
            cwd: project.cwd.clone(),
            local: Some(project),
        }
    }
}
#[derive(Clone)]
pub(crate) struct MissionDisplay {
    pub(crate) mission: MissionFields,
    pub(crate) any_session_live: bool,
    pub(crate) session_statuses: Vec<(String, runner_core::protocol::AgentStatus)>,
}
#[derive(Clone)]
pub(crate) struct MissionFields {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) running: bool,
    pub(crate) project_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) archived_at: Option<runner_core::protocol::model::Timestamp>,
}
impl From<runner_core::protocol::mission::MissionSummary> for MissionDisplay {
    fn from(summary: runner_core::protocol::mission::MissionSummary) -> Self {
        let mission = summary.mission;
        Self {
            mission: MissionFields {
                id: mission.id,
                title: mission.title,
                running: mission.status == runner_core::protocol::model::MissionStatus::Running,
                project_id: mission.project_id,
                cwd: mission.cwd,
                archived_at: mission.archived_at,
            },
            any_session_live: summary.any_session_live,
            session_statuses: summary.session_statuses,
        }
    }
}

impl SidebarNodeDisplay {
    pub(crate) fn target(&self) -> runner_core::companion::ActionTarget {
        use runner_core::{companion::ActionKind, protocol::node::NodeType};
        runner_core::companion::ActionTarget {
            kind: match self.node_type {
                NodeType::Tab => ActionKind::Tab,
                NodeType::Mission => ActionKind::Mission,
                NodeType::Project => ActionKind::Project,
            },
            id: self.ref_id.clone().unwrap_or_else(|| self.id.clone()),
        }
    }
}

impl From<DirectSessionEntry> for SessionDisplay {
    fn from(entry: DirectSessionEntry) -> Self {
        Self::local(&entry)
    }
}
