//! The root picker's list: Choose folder…, then the repository's worktrees,
//! then projects and recent folders; and what its field's text means.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui::App;
use runner_core::protocol::files::{WorktreeEntry, WorktreeKind};
use runner_core::protocol::SessionStatus;

use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface};
use crate::text_util::{abbreviate_home, HomePath};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CheckoutSessionKind {
    Chat,
    Terminal,
    MissionSlot(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckoutSessionCandidate {
    pub(crate) cwd: PathBuf,
    pub(crate) kind: CheckoutSessionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckoutSessionLabel {
    pub(crate) text: String,
    pub(crate) is_own: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    Worktree(WorktreeKind),
    Recent,
    Project,
    ChooseFolder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PickerItem {
    pub(crate) kind: PickerKind,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    /// `None` for Choose folder….
    pub(crate) path: Option<PathBuf>,
    pub(crate) current: bool,
    pub(crate) uncommitted: Option<usize>,
    pub(crate) session_label: Option<CheckoutSessionLabel>,
}

pub(crate) fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) fn same_checkout_path(a: &Path, b: &Path) -> bool {
    runner_core::app_paths::same_checkout_path(a, b)
}

fn normalized_checkout_path(path: &Path) -> PathBuf {
    runner_core::app_paths::normalize_checkout_path(path)
}

/// Shortens a checkout's path from the left (`…/feat-645-m4-host-api` for a
/// linked worktree under `.worktrees/`, or `~/repos/…/runner` when deep).
pub(crate) fn shorten_checkout_path(path: &Path, home: Option<&Path>) -> String {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim_end_matches(['/', '\\']);
    let mut rev = trimmed.rsplit(['/', '\\']).filter(|s| !s.is_empty());
    if let (Some(leaf), Some(".worktrees")) = (rev.next(), rev.next()) {
        return format!("…/{leaf}");
    }
    let abbreviated = abbreviate_home(HomePath::Native(path, home)).replace('\\', "/");
    let parts: Vec<&str> = abbreviated
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if abbreviated.starts_with("~/") {
        if parts.len() <= 3 {
            abbreviated
        } else {
            format!("~/{}/…/{}", parts[1], parts[parts.len() - 1])
        }
    } else if parts.len() <= 2 {
        abbreviated
    } else {
        format!("…/{}", parts[parts.len() - 1])
    }
}

pub(crate) fn own_surface_session_text(surface: PanelSurface) -> String {
    crate::i18n::text(match surface {
        PanelSurface::Chat => "changes.session_this_chat",
        PanelSurface::Terminal => "changes.session_this_terminal",
        PanelSurface::Mission => "changes.session_this_mission",
    })
    .to_owned()
}

/// Matches running sessions to their deepest containing worktree and builds the
/// right-hand session label for each worktree path.
pub(crate) fn checkout_session_labels(
    surface: PanelSurface,
    own_checkout_top: Option<&Path>,
    worktrees: &[WorktreeEntry],
    candidates: &[CheckoutSessionCandidate],
) -> BTreeMap<PathBuf, CheckoutSessionLabel> {
    let live: Vec<(PathBuf, PathBuf)> = worktrees
        .iter()
        .filter(|wt| !wt.prunable)
        .map(|wt| (wt.path.clone(), normalized_checkout_path(&wt.path)))
        .collect();
    let own_norm = own_checkout_top.map(normalized_checkout_path);

    let mut buckets: BTreeMap<PathBuf, Vec<&CheckoutSessionKind>> = BTreeMap::new();
    for candidate in candidates {
        let cwd_norm = normalized_checkout_path(&candidate.cwd);
        let matched = live
            .iter()
            .filter(|(_, wt_norm)| {
                runner_core::app_paths::normalized_checkout_starts_with(&cwd_norm, wt_norm)
            })
            .max_by_key(|(_, wt_norm)| wt_norm.components().count());
        if let Some((raw_path, _)) = matched {
            buckets
                .entry(raw_path.clone())
                .or_default()
                .push(&candidate.kind);
        }
    }

    let mut labels = BTreeMap::new();
    for (raw_path, wt_norm) in &live {
        if own_norm
            .as_ref()
            .is_some_and(|own| runner_core::app_paths::same_normalized_checkout_path(own, wt_norm))
        {
            labels.insert(
                raw_path.clone(),
                CheckoutSessionLabel {
                    text: own_surface_session_text(surface),
                    is_own: true,
                },
            );
            continue;
        }
        let Some(entries) = buckets.get(raw_path) else {
            continue;
        };
        let text = match entries.as_slice() {
            [] => continue,
            [CheckoutSessionKind::Chat] => crate::i18n::text("changes.session_chat").to_owned(),
            [CheckoutSessionKind::Terminal] => {
                crate::i18n::text("changes.session_terminal").to_owned()
            }
            [CheckoutSessionKind::MissionSlot(slot)] => {
                crate::i18n::tf("changes.session_mission_slot", &[("slot", slot.clone())])
            }
            many => crate::i18n::tn("changes.session_count", many.len()),
        };
        labels.insert(
            raw_path.clone(),
            CheckoutSessionLabel {
                text,
                is_own: false,
            },
        );
    }
    labels
}

pub(crate) fn collect_checkout_session_labels(
    context: &PanelContext,
    own_checkout_top: Option<&Path>,
    worktrees: &[WorktreeEntry],
    cx: &App,
) -> BTreeMap<PathBuf, CheckoutSessionLabel> {
    collect_checkout_session_labels_for(Some(context), own_checkout_top, worktrees, cx)
}

pub(crate) fn collect_launcher_checkout_session_labels(
    worktrees: &[WorktreeEntry],
    cx: &App,
) -> BTreeMap<PathBuf, CheckoutSessionLabel> {
    collect_checkout_session_labels_for(None, None, worktrees, cx)
}

fn collect_checkout_session_labels_for(
    context: Option<&PanelContext>,
    own_checkout_top: Option<&Path>,
    worktrees: &[WorktreeEntry],
    cx: &App,
) -> BTreeMap<PathBuf, CheckoutSessionLabel> {
    let store = crate::app_store::global_app_store(cx).read(cx);
    let project_cwd = |project_id: Option<&str>| -> Option<PathBuf> {
        let id = project_id?;
        store
            .projects
            .iter()
            .find(|project| project.id == id && !project.cwd.trim().is_empty())
            .map(|project| PathBuf::from(&project.cwd))
    };

    let mut candidates = Vec::new();
    for entry in &store.sessions {
        if entry.archived_at.is_some() || entry.status != SessionStatus::Running {
            continue;
        }
        if context.is_some_and(
            |ctx| matches!(&ctx.subject, PanelSubject::Session(id) if id == &entry.session_id),
        ) {
            continue;
        }
        let is_shell = entry.agent_runtime == "shell";
        let cwd = if is_shell {
            store
                .live_terminal_cwd(&entry.session_id)
                .map(Path::to_path_buf)
        } else {
            None
        }
        .or_else(|| {
            entry
                .cwd
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| project_cwd(entry.project_id.as_deref()));

        if let Some(cwd) = cwd {
            candidates.push(CheckoutSessionCandidate {
                cwd,
                kind: if is_shell {
                    CheckoutSessionKind::Terminal
                } else {
                    CheckoutSessionKind::Chat
                },
            });
        }
    }

    for summary in &store.missions {
        if summary.mission.archived_at.is_some() {
            continue;
        }
        if context.is_some_and(
            |ctx| matches!(&ctx.subject, PanelSubject::Mission(id) if id == &summary.mission.id),
        ) {
            continue;
        }
        let Some(rows) = store.mission_sessions.get(&summary.mission.id) else {
            continue;
        };
        let mission_fallback_cwd = summary
            .mission
            .cwd
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| project_cwd(summary.mission.project_id.as_deref()));

        for row in rows {
            if row.session.status != SessionStatus::Running {
                continue;
            }
            let is_shell = row.runtime == "shell";
            let cwd = if is_shell {
                store
                    .live_terminal_cwd(&row.session.id)
                    .map(Path::to_path_buf)
            } else {
                None
            }
            .or_else(|| {
                row.session
                    .cwd
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| mission_fallback_cwd.clone());

            if let Some(cwd) = cwd {
                candidates.push(CheckoutSessionCandidate {
                    cwd,
                    kind: if is_shell {
                        CheckoutSessionKind::Terminal
                    } else {
                        CheckoutSessionKind::MissionSlot(row.handle.clone())
                    },
                });
            }
        }
    }

    checkout_session_labels(
        context.map_or(PanelSurface::Chat, |ctx| ctx.surface),
        own_checkout_top,
        worktrees,
        &candidates,
    )
}

pub(crate) struct PickerSources<'a> {
    pub(crate) current: &'a Path,
    pub(crate) worktrees: &'a [WorktreeEntry],
    pub(crate) session_labels: &'a BTreeMap<PathBuf, CheckoutSessionLabel>,
    pub(crate) recent: &'a [PathBuf],
    /// `(name, folder)`; empty unless the subject is outside a project.
    pub(crate) projects: &'a [(String, PathBuf)],
    pub(crate) home: Option<&'a Path>,
    pub(crate) main_checkout_label: &'a str,
    pub(crate) detached_label: &'a str,
    pub(crate) choose_folder_label: &'a str,
}

pub(crate) fn picker_items(sources: PickerSources<'_>) -> Vec<PickerItem> {
    let mut worktrees = sources
        .worktrees
        .iter()
        .filter(|worktree| !worktree.prunable)
        .collect::<Vec<_>>();
    worktrees.sort_by_key(|worktree| worktree.kind != WorktreeKind::Main);
    let mut items = vec![PickerItem {
        kind: PickerKind::ChooseFolder,
        title: sources.choose_folder_label.to_owned(),
        subtitle: String::new(),
        path: None,
        current: false,
        uncommitted: None,
        session_label: None,
    }];
    items.extend(worktrees.into_iter().map(|worktree| {
        let mut subtitle = vec![worktree
            .branch
            .clone()
            .unwrap_or_else(|| sources.detached_label.to_owned())];
        subtitle.extend(worktree.commit.clone());
        if worktree.kind == WorktreeKind::Main {
            subtitle.push(sources.main_checkout_label.to_owned());
        }
        PickerItem {
            kind: PickerKind::Worktree(worktree.kind),
            title: folder_name(&worktree.path),
            subtitle: subtitle.join(" · "),
            current: same_checkout_path(&worktree.path, sources.current),
            path: Some(worktree.path.clone()),
            uncommitted: worktree.uncommitted,
            session_label: sources.session_labels.get(&worktree.path).cloned(),
        }
    }));
    let listed = |items: &[PickerItem], path: &Path| {
        items.iter().any(|item| {
            item.path
                .as_deref()
                .is_some_and(|p| same_checkout_path(p, path))
        })
    };
    for (name, path) in sources.projects {
        if !listed(&items, path) {
            items.push(PickerItem {
                kind: PickerKind::Project,
                title: name.clone(),
                subtitle: abbreviate_home(HomePath::Native(path, sources.home)),
                current: same_checkout_path(path, sources.current),
                path: Some(path.clone()),
                uncommitted: None,
                session_label: None,
            });
        }
    }
    for path in sources.recent {
        if !listed(&items, path) {
            items.push(PickerItem {
                kind: PickerKind::Recent,
                title: folder_name(path),
                subtitle: abbreviate_home(HomePath::Native(path, sources.home)),
                current: same_checkout_path(path, sources.current),
                path: Some(path.clone()),
                uncommitted: None,
                session_label: None,
            });
        }
    }
    items
}

/// The row Enter takes until the arrows move: the first folder listed, so
/// that Enter does not open the system picker from Choose folder… above it.
pub(crate) fn default_highlight(visible: usize) -> usize {
    usize::from(visible > 1)
}

/// The items whose name, branch or path contains `query`, ignoring case;
/// Choose folder… always stays.
pub(crate) fn filter(items: &[PickerItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            query.is_empty()
                || item.kind == PickerKind::ChooseFolder
                || item.title.to_lowercase().contains(&query)
                || item.subtitle.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

/// The folder `text` names when it reads as a path rather than a filter:
/// absolute, or starting at the home folder with `~`.
pub(crate) fn typed_path(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    let text = text.trim();
    if text == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return home.map(|home| home.join(rest));
    }
    let path = Path::new(text);
    path.is_absolute().then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree(path: &str, branch: Option<&str>, kind: WorktreeKind) -> WorktreeEntry {
        WorktreeEntry {
            path: path.into(),
            branch: branch.map(Into::into),
            commit: Some("9094032".into()),
            kind,
            locked: false,
            prunable: false,
            uncommitted: None,
        }
    }

    fn items(current: &str, projects: &[(String, PathBuf)]) -> Vec<PickerItem> {
        let worktrees = [
            worktree(
                "/r/runner/.worktrees/feat-x",
                Some("feat/x"),
                WorktreeKind::Linked,
            ),
            worktree("/r/runner", Some("main"), WorktreeKind::Main),
            worktree("/r/runner/.worktrees/detached", None, WorktreeKind::Linked),
        ];
        let recent = [PathBuf::from("/r/runner"), PathBuf::from("/home/me/notes")];
        let session_labels = BTreeMap::new();
        picker_items(PickerSources {
            current: Path::new(current),
            worktrees: &worktrees,
            session_labels: &session_labels,
            recent: &recent,
            projects,
            home: Some(Path::new("/home/me")),
            main_checkout_label: "main checkout",
            detached_label: "detached",
            choose_folder_label: "Choose folder…",
        })
    }

    #[test]
    fn choose_folder_comes_first_then_worktrees_from_the_main_checkout_then_folders() {
        let projects = [("quill".to_owned(), PathBuf::from("/home/me/quill"))];
        let items = items("/r/runner/.worktrees/feat-x", &projects);
        let rows = items
            .iter()
            .map(|item| {
                let subtitle = item.subtitle.replace(std::path::MAIN_SEPARATOR, "/");
                (item.title.as_str(), subtitle, item.current)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ("Choose folder…", String::new(), false),
                ("runner", "main · 9094032 · main checkout".into(), false),
                ("feat-x", "feat/x · 9094032".into(), true),
                ("detached", "detached · 9094032".into(), false),
                ("quill", "~/quill".into(), false),
                ("notes", "~/notes".into(), false),
            ]
        );
        assert_eq!(items[0].path, None);
        assert_eq!(items[4].kind, PickerKind::Project);
        assert_eq!(items[5].kind, PickerKind::Recent);
    }

    #[test]
    fn typing_filters_by_name_branch_or_path_and_keeps_choose_folder() {
        let items = items("/r/runner", &[]);
        let titles = |query: &str| {
            filter(&items, query)
                .into_iter()
                .map(|index| items[index].title.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("FEAT"), ["Choose folder…", "feat-x"]);
        assert_eq!(titles("main"), ["Choose folder…", "runner"]);
        assert_eq!(titles("NOTES"), ["Choose folder…", "notes"]);
        assert_eq!(titles("zzz"), ["Choose folder…"]);
        assert_eq!(titles("  ").len(), items.len());
    }

    #[test]
    fn enter_starts_on_the_first_folder_unless_only_choose_folder_is_left() {
        assert_eq!(default_highlight(1), 0);
        assert_eq!(default_highlight(2), 1);
        assert_eq!(default_highlight(6), 1);
    }

    #[test]
    fn a_typed_path_is_absolute_or_from_home() {
        let home = Path::new("/home/me");
        assert_eq!(
            typed_path("~/repos/x ", Some(home)),
            Some(PathBuf::from("/home/me/repos/x"))
        );
        assert_eq!(typed_path("~", Some(home)), Some(home.to_path_buf()));
        assert_eq!(typed_path("runner", Some(home)), None);
        assert_eq!(typed_path("~/x", None), None);
        #[cfg(unix)]
        assert_eq!(typed_path("/tmp", None), Some(PathBuf::from("/tmp")));
        #[cfg(windows)]
        assert_eq!(typed_path(r"C:\tmp", None), Some(PathBuf::from(r"C:\tmp")));
    }
}
