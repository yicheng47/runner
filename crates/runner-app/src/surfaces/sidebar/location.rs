use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::surfaces::remote_client::model::{
    host_path_leaf, looks_like_windows_path, normalize_remote_host_path, Connection,
};
use gpui::{div, prelude::*, px, rems, AnyElement, App};
use runner_core::app_paths::normalize_checkout_path;
use runner_core::protocol::files::WorktreeKind;
use runner_core::protocol::project::ProjectRow;
use runner_core::protocol::worktree::WorktreeCheckout;

use super::{node_project_id, MissionSummary, NodeRow, SessionDisplay, Sidebar};
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocationKind {
    Folder,
    Branch,
    DetachedCommit,
    MultipleCheckouts,
    MissingWorktree,
}

impl LocationKind {
    pub(crate) fn icon_path(self) -> &'static str {
        match self {
            Self::Folder => "folder.svg",
            Self::Branch => "git-branch.svg",
            Self::DetachedCommit => "git-commit-horizontal.svg",
            Self::MultipleCheckouts => "layers.svg",
            Self::MissingWorktree => "triangle-alert.svg",
        }
    }

    pub(crate) fn is_warning(self) -> bool {
        matches!(self, Self::MissingWorktree)
    }
}

/// Resolved checkout display metadata shared by the Projects and Active sidebar views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowLocation {
    pub(crate) projects_line: Option<String>,
    pub(crate) active_line: String,
    pub(crate) active_missing: Option<(String, String)>,
    pub(crate) kind: LocationKind,
    pub(crate) tooltip: Option<String>,
}

pub(super) struct PaneLocationInput<'a> {
    pub(super) label: &'a str,
    pub(super) cwd: Option<&'a str>,
    pub(super) checkout: Option<&'a WorktreeCheckout>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CheckoutBucket {
    Git(PathBuf),
    Plain(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocationMode {
    Local,
    RemoteOnline,
    RemoteOffline,
}

fn location_checkout_key(path: &Path, is_remote: bool) -> PathBuf {
    let raw = path.to_string_lossy();
    if is_remote || (cfg!(not(windows)) && looks_like_windows_path(&raw)) {
        PathBuf::from(normalize_remote_host_path(&raw))
    } else {
        normalize_checkout_path(path)
    }
}

/// Formats location display for a sidebar row across Projects and Active views.
///
/// The line always identifies project and checkout, never a repository subfolder.
pub(super) fn resolve_row_location(
    project: Option<&ProjectRow>,
    panes: &[PaneLocationInput<'_>],
    home: Option<&str>,
    mode: LocationMode,
) -> RowLocation {
    let first_cwd = panes.iter().find_map(|pane| pane.cwd);
    let effective_home = if matches!(mode, LocationMode::Local) {
        home
    } else {
        None
    };
    let is_remote = !matches!(mode, LocationMode::Local);
    if matches!(mode, LocationMode::RemoteOffline)
        || panes.iter().all(|pane| pane.checkout.is_none())
    {
        return RowLocation {
            projects_line: None,
            active_line: fallback_active_label(project, first_cwd, effective_home),
            active_missing: None,
            kind: LocationKind::Folder,
            tooltip: None,
        };
    }

    let mut git_tops: Vec<(PathBuf, &WorktreeCheckout, Option<&str>)> = Vec::new();
    let mut distinct = BTreeSet::new();
    for pane in panes {
        match pane.checkout {
            Some(state @ WorktreeCheckout::Git { top, .. }) => {
                let norm = location_checkout_key(top, is_remote);
                if distinct.insert(CheckoutBucket::Git(norm.clone())) {
                    git_tops.push((norm, state, pane.cwd));
                }
            }
            Some(WorktreeCheckout::Plain { .. }) => {
                if let Some(cwd) = pane.cwd {
                    distinct.insert(CheckoutBucket::Plain(location_checkout_key(
                        Path::new(cwd),
                        is_remote,
                    )));
                }
            }
            None => {}
        }
    }

    if distinct.len() > 1 && !git_tops.is_empty() {
        let count = distinct.len();
        let summary = crate::i18n::tn("sidebar.checkouts_count", count);
        let active_prefix = project.map(|p| p.name.clone()).or_else(|| {
            shared_main_checkout(&git_tops, is_remote)
                .map(|main| compact_path(main, effective_home))
        });
        let active_line = match active_prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix} › {summary}"),
            _ => summary.clone(),
        };
        let tooltip = panes
            .iter()
            .filter_map(|pane| {
                let branch = pane_tooltip_branch(pane.checkout, pane.cwd, effective_home)?;
                Some(if pane.label.is_empty() {
                    branch
                } else {
                    format!("{}: {branch}", pane.label)
                })
            })
            .collect::<Vec<_>>()
            .join("\n");
        return RowLocation {
            projects_line: Some(summary),
            active_line,
            active_missing: None,
            kind: LocationKind::MultipleCheckouts,
            tooltip: (!tooltip.is_empty()).then_some(tooltip),
        };
    }

    let Some((
        norm_top,
        WorktreeCheckout::Git {
            top,
            main_checkout,
            kind,
            branch,
            commit,
            missing,
            ..
        },
        pane_cwd,
    )) = git_tops.into_iter().next()
    else {
        return RowLocation {
            projects_line: None,
            active_line: fallback_active_label(project, first_cwd, home),
            active_missing: None,
            kind: LocationKind::Folder,
            tooltip: None,
        };
    };
    let _ = norm_top;

    if *missing {
        let label = missing_branch_label(branch.as_deref(), top, pane_cwd.or(first_cwd));
        let status = crate::i18n::tf("sidebar.worktree_missing", &[("label", label.clone())]);
        let prefix = project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| compact_path(main_checkout, home));
        let active_line = if prefix.is_empty() {
            status.clone()
        } else {
            format!("{prefix} › {status}")
        };
        return RowLocation {
            projects_line: Some(status.clone()),
            active_line,
            active_missing: Some((prefix, label)),
            kind: LocationKind::MissingWorktree,
            tooltip: Some(status),
        };
    }

    if *kind == WorktreeKind::Main {
        let active_line = project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| compact_path(top, home));
        return RowLocation {
            projects_line: None,
            active_line,
            active_missing: None,
            kind: LocationKind::Folder,
            tooltip: None,
        };
    }

    let (location_kind, checkout_label) = match branch.as_deref().filter(|b| !b.is_empty()) {
        Some(branch) => (LocationKind::Branch, branch.to_owned()),
        None => (
            LocationKind::DetachedCommit,
            commit
                .as_deref()
                .filter(|c| !c.is_empty())
                .unwrap_or("HEAD")
                .to_owned(),
        ),
    };
    let prefix = project
        .map(|p| p.name.clone())
        .unwrap_or_else(|| compact_path(main_checkout, home));
    let active_line = if prefix.is_empty() {
        checkout_label.clone()
    } else {
        format!("{prefix} › {checkout_label}")
    };

    RowLocation {
        projects_line: Some(checkout_label.clone()),
        active_line,
        active_missing: None,
        kind: location_kind,
        tooltip: Some(checkout_label),
    }
}

pub(super) fn render_branch_subtitle(node_id: &str, location: &RowLocation) -> Option<AnyElement> {
    let text = location.projects_line.as_ref()?;
    let color = location_color(location.kind);
    let node_id = node_id.to_owned();
    Some(
        div()
            .debug_selector(move || format!("SIDEBAR_BRANCH {node_id}"))
            .min_w(px(0.))
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .text_size(theme::text_meta())
            .text_color(color)
            .child(
                gpui::svg()
                    .path(location.kind.icon_path())
                    .size(rems(10. / 16.))
                    .flex_none()
                    .text_color(color),
            )
            .child(div().min_w(px(0.)).truncate().child(text.clone()))
            .into_any_element(),
    )
}

pub(super) fn render_active_location_line(node_id: &str, location: &RowLocation) -> AnyElement {
    let color = location_color(location.kind);
    let node_id = node_id.to_owned();
    let status_node_id = node_id.clone();
    let content = if let Some((prefix, label)) = &location.active_missing {
        let missing_prefix = crate::i18n::text("sidebar.worktree_missing_prefix");
        div()
            .min_w(px(0.))
            .flex_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .when(!prefix.is_empty(), |row| {
                row.child(div().min_w(px(0.)).truncate().child(prefix.clone()))
                    .child(div().flex_none().child(" › "))
            })
            .child(
                div()
                    .flex_none()
                    .when(cfg!(test), |el| {
                        let id = status_node_id.clone();
                        el.debug_selector(move || format!("ACTIVE_MISSING_STATUS {id}"))
                    })
                    .child(missing_prefix),
            )
            .when(!label.is_empty(), |row| {
                row.child(div().min_w(px(0.)).truncate().child(label.clone()))
            })
            .into_any_element()
    } else {
        div()
            .min_w(px(0.))
            .truncate()
            .child(location.active_line.clone())
            .into_any_element()
    };
    div()
        .debug_selector(move || format!("ACTIVE_LOCATION {node_id}"))
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .text_size(theme::text_meta())
        .text_color(color)
        .child(
            gpui::svg()
                .path(location.kind.icon_path())
                .size(rems(10. / 16.))
                .flex_none()
                .text_color(color),
        )
        .child(content)
        .into_any_element()
}

fn location_color(kind: LocationKind) -> gpui::Hsla {
    if kind.is_warning() {
        theme::warning()
    } else {
        theme::faint()
    }
}

fn fallback_active_label(
    project: Option<&ProjectRow>,
    cwd: Option<&str>,
    home: Option<&str>,
) -> String {
    match project {
        Some(project) => project.name.clone(),
        None => cwd
            .map(|path| compact_path(Path::new(path), home))
            .unwrap_or_default(),
    }
}

fn shared_main_checkout<'a>(
    git_tops: &'a [(PathBuf, &WorktreeCheckout, Option<&str>)],
    is_remote: bool,
) -> Option<&'a Path> {
    let mut main: Option<&Path> = None;
    for (_, state, _) in git_tops {
        if let WorktreeCheckout::Git { main_checkout, .. } = state {
            if let Some(prev) = main {
                if location_checkout_key(prev, is_remote)
                    != location_checkout_key(main_checkout, is_remote)
                {
                    return None;
                }
            } else {
                main = Some(main_checkout.as_path());
            }
        }
    }
    main
}

fn pane_tooltip_branch(
    checkout: Option<&WorktreeCheckout>,
    cwd: Option<&str>,
    home: Option<&str>,
) -> Option<String> {
    match checkout {
        Some(WorktreeCheckout::Git {
            top,
            branch,
            missing: true,
            ..
        }) => {
            let label = missing_branch_label(branch.as_deref(), top, cwd);
            Some(crate::i18n::tf(
                "sidebar.worktree_missing",
                &[("label", label)],
            ))
        }
        Some(WorktreeCheckout::Git {
            branch,
            commit,
            missing: false,
            ..
        }) => Some(
            branch
                .as_deref()
                .filter(|b| !b.is_empty())
                .or_else(|| commit.as_deref().filter(|c| !c.is_empty()))
                .unwrap_or("HEAD")
                .to_owned(),
        ),
        _ => cwd.map(|path| compact_path(Path::new(path), home)),
    }
}

fn missing_branch_label(branch: Option<&str>, top: &Path, cwd: Option<&str>) -> String {
    if let Some(branch) = branch.filter(|b| !b.is_empty()) {
        return branch.to_owned();
    }
    let top_raw = top.to_string_lossy();
    if let Some(name) = host_path_leaf(&top_raw)
        && !name.is_empty()
    {
        return name.to_owned();
    }
    cwd.and_then(host_path_leaf)
        .map(str::to_owned)
        .unwrap_or_default()
}

impl Sidebar {
    pub(super) fn tab_location(
        &self,
        node: &NodeRow,
        members: &[SessionDisplay],
        cx: &App,
    ) -> RowLocation {
        let store = self.app_store.read(cx);
        let fallback_project = members.iter().find_map(|member| member.project_id.clone());
        let project_id = node_project_id(&self.source_nodes(cx), node).or(fallback_project);
        let projects = self.source_projects(cx);
        let project = project_for_id(&projects, project_id.as_deref());
        let remote = self.remote_computer_id();
        let clients = remote.map(|_| store.remote_clients.read(cx));
        let (mode, home) = location_mode_and_home(remote, clients, store);
        let pane_labels = members
            .iter()
            .map(|member| member.label(self.live_title(&member.session_id, cx).as_deref()))
            .collect::<Vec<_>>();
        let panes = members
            .iter()
            .zip(pane_labels.iter())
            .map(|(member, label)| {
                let checkout = match (remote, clients) {
                    (Some(computer), Some(clients)) => clients
                        .remote_session_checkout(
                            computer,
                            &member.session_id,
                            member.cwd.as_deref(),
                        )
                        .map(|entry| &entry.checkout),
                    (None, _) => member.cwd.as_deref().and_then(|cwd| {
                        store.worktree_checkout_for(
                            cwd,
                            project_id.as_deref().or(member.project_id.as_deref()),
                        )
                    }),
                    (Some(_), None) => None,
                };
                PaneLocationInput {
                    label: label.as_str(),
                    cwd: member.cwd.as_deref(),
                    checkout,
                }
            })
            .collect::<Vec<_>>();
        resolve_row_location(project.as_ref(), &panes, home, mode)
    }

    pub(super) fn mission_location(
        &self,
        node: &NodeRow,
        summary: &MissionSummary,
        project_id_hint: Option<&str>,
        cx: &App,
    ) -> RowLocation {
        let store = self.app_store.read(cx);
        let project_id = node_project_id(&self.source_nodes(cx), node)
            .or_else(|| project_id_hint.map(str::to_owned))
            .or_else(|| summary.mission.project_id.clone());
        let projects = self.source_projects(cx);
        let project = project_for_id(&projects, project_id.as_deref());
        let remote = self.remote_computer_id();
        let clients = remote.map(|_| store.remote_clients.read(cx));
        let (mode, home) = location_mode_and_home(remote, clients, store);
        let checkout = match (remote, clients) {
            (Some(computer), Some(clients)) => clients
                .remote_mission_checkout(
                    computer,
                    &summary.mission.id,
                    summary.mission.cwd.as_deref(),
                )
                .map(|entry| &entry.checkout),
            (None, _) => summary
                .mission
                .cwd
                .as_deref()
                .and_then(|cwd| store.worktree_checkout_for(cwd, project_id.as_deref())),
            (Some(_), None) => None,
        };
        let pane = PaneLocationInput {
            label: summary.mission.title.as_str(),
            cwd: summary.mission.cwd.as_deref(),
            checkout,
        };
        resolve_row_location(project.as_ref(), &[pane], home, mode)
    }
}

fn project_for_id(
    projects: &[crate::surfaces::front_page::model::ProjectDisplay],
    project_id: Option<&str>,
) -> Option<ProjectRow> {
    let id = project_id?;
    let project = projects.iter().find(|project| project.id == id)?;
    Some(project.local.clone().unwrap_or_else(|| ProjectRow {
        id: project.id.clone(),
        name: project.name.clone(),
        cwd: project.cwd.clone(),
        position: 0,
        created_at: String::new(),
    }))
}

fn location_mode_and_home<'a>(
    remote: Option<&str>,
    clients: Option<&crate::surfaces::remote_client::model::ClientState>,
    store: &'a crate::app_store::AppStore,
) -> (LocationMode, Option<&'a str>) {
    let mode = match (remote, clients) {
        (Some(computer), Some(clients))
            if clients.connection(computer) == Connection::Connected =>
        {
            LocationMode::RemoteOnline
        }
        (Some(_), _) => LocationMode::RemoteOffline,
        (None, _) => LocationMode::Local,
    };
    let home = matches!(mode, LocationMode::Local)
        .then(|| store.home_dir.as_deref().and_then(|path| path.to_str()))
        .flatten();
    (mode, home)
}

fn compact_path(path: &Path, home: Option<&str>) -> String {
    let raw = path.to_string_lossy();
    runner_app::ui::field::compact_path_candidates(&raw, home)
        .into_iter()
        .next()
        .unwrap_or_default()
}
