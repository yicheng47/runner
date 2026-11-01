//! The launcher's project picker: the project new work is filed under, kept
//! in step with its start folder.

use gpui::prelude::*;
use gpui::{div, rems, svg, App, Entity};
use runner_app::ui::{SelectHandler, SelectLeading, SelectOption, StyledSelect};
use runner_core::companion::Catalog;
use runner_core::protocol::project::{deepest_folder_of, ProjectRow, ProjectScope};

use crate::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectChoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) path: String,
}

impl ProjectChoice {
    pub(crate) fn local(projects: &[ProjectRow]) -> Vec<Self> {
        projects
            .iter()
            .map(|project| Self {
                id: project.id.clone(),
                name: project.name.clone(),
                path: project.cwd.clone(),
            })
            .collect()
    }

    pub(crate) fn host(catalog: &Catalog) -> Vec<Self> {
        catalog
            .projects
            .iter()
            .map(|project| Self {
                id: project.id.clone(),
                name: project.name.clone(),
                path: project.path.clone(),
            })
            .collect()
    }
}

pub(crate) struct ProjectField {
    choices: Vec<ProjectChoice>,
    /// Paths on this computer, compared as they exist; a remote computer's
    /// are compared as written.
    local: bool,
    chosen: Option<String>,
    committed_directory: String,
    /// A pane's chat files with its tab.
    locked: bool,
    select: Entity<StyledSelect>,
}

impl ProjectField {
    pub(crate) fn new(
        id: &'static str,
        choices: Vec<ProjectChoice>,
        local: bool,
        chosen: Option<String>,
        locked: bool,
        on_change: SelectHandler,
        cx: &mut App,
    ) -> Self {
        let chosen = chosen.filter(|id| choices.iter().any(|choice| &choice.id == id));
        let options = project_options(&choices);
        let value = chosen.clone().unwrap_or_default();
        let select = cx.new(|select_cx| {
            StyledSelect::new(
                id,
                select_cx.focus_handle(),
                value,
                options,
                on_change,
                select_cx,
            )
            .full_width(true)
            .mono_descriptions(true)
            .disabled(locked)
        });
        let committed_directory = chosen
            .as_ref()
            .and_then(|id| choices.iter().find(|choice| &choice.id == id))
            .map(|choice| choice.path.clone())
            .unwrap_or_default();
        Self {
            choices,
            local,
            chosen,
            committed_directory,
            locked,
            select,
        }
    }

    pub(crate) fn select(&self) -> Entity<StyledSelect> {
        self.select.clone()
    }

    pub(crate) fn scope(&self) -> ProjectScope {
        ProjectScope::or_root(self.chosen.clone())
    }

    pub(crate) fn chosen(&self) -> Option<&ProjectChoice> {
        let id = self.chosen.as_deref()?;
        self.choices.iter().find(|choice| choice.id == id)
    }

    /// The menu chose `id`, empty for No project. Returns the working
    /// directory the choice leaves, when it differs from `directory`: the
    /// project's folder, unless `directory` is already inside it.
    pub(crate) fn choose(&mut self, id: &str, directory: &str) -> Option<String> {
        if self.locked {
            return None;
        }
        self.chosen = (!id.is_empty()).then(|| id.to_owned());
        let replacement = self
            .chosen()
            .and_then(|project| (!self.holds(project, directory)).then(|| project.path.clone()));
        self.committed_directory = replacement
            .as_deref()
            .unwrap_or(directory)
            .trim()
            .to_owned();
        replacement
    }

    /// The working directory was committed: a directory inside a project
    /// chooses it, the deepest when projects nest. Returns whether the
    /// choice changed.
    pub(crate) fn follow(&mut self, directory: &str, cx: &mut App) -> bool {
        let directory = directory.trim();
        if self.committed_directory == directory {
            return false;
        }
        self.committed_directory = directory.to_owned();
        if self.locked || directory.is_empty() {
            return false;
        }
        let Some(index) = deepest_folder_of(
            self.choices.iter().map(|choice| choice.path.as_str()),
            directory,
            self.local,
        ) else {
            return false;
        };
        let id = self.choices[index].id.clone();
        if self.chosen.as_ref() == Some(&id) {
            return false;
        }
        self.select.update(cx, |select, select_cx| {
            select.set_value(id.clone(), select_cx)
        });
        self.chosen = Some(id);
        true
    }

    pub(crate) fn picked(&mut self, directory: &str, cx: &mut App) {
        self.committed_directory.clear();
        self.follow(directory, cx);
    }

    /// A remote computer's projects arrive after the launcher opens.
    pub(crate) fn set_choices(&mut self, choices: Vec<ProjectChoice>, cx: &mut App) {
        if self.choices == choices {
            return;
        }
        if self
            .chosen
            .as_ref()
            .is_some_and(|id| !choices.iter().any(|choice| &choice.id == id))
        {
            self.chosen = None;
        }
        let options = project_options(&choices);
        let value = self.chosen.clone().unwrap_or_default();
        self.choices = choices;
        self.committed_directory.clear();
        self.select.update(cx, |select, select_cx| {
            select.set_options(options, select_cx);
            select.set_value(value, select_cx);
        });
    }

    fn holds(&self, project: &ProjectChoice, directory: &str) -> bool {
        deepest_folder_of([project.path.as_str()], directory.trim(), self.local).is_some()
    }
}

fn project_options(choices: &[ProjectChoice]) -> Vec<SelectOption> {
    std::iter::once(
        SelectOption::new("", crate::i18n::message("ui.no_project"))
            .description(crate::i18n::message(
                "ui.listed_under_recents_starts_in_your_default_directory",
            ))
            .leading(icon_leading("clock.svg")),
    )
    .chain(choices.iter().enumerate().map(|(index, choice)| {
        SelectOption::new(choice.id.clone(), choice.name.clone())
            .separator_before(index == 0)
            .description(choice.path.clone())
            .leading(icon_leading("folder.svg"))
    }))
    .collect()
}

fn icon_leading(icon: &'static str) -> SelectLeading {
    SelectLeading::new(icon, move |size| {
        div()
            .flex_none()
            .size(rems(size / 16.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                svg()
                    .path(icon)
                    .size(rems(size / 16.))
                    .text_color(theme::muted()),
            )
            .into_any_element()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Vec<ProjectChoice> {
        [
            ("parent", "yicheng47", "/repos/yicheng47"),
            ("runner", "runner", "/repos/yicheng47/runner"),
            ("web", "runner-web", "/repos/yicheng47/runner-web"),
            ("quill", "quill", "/repos/yicheng47/quill"),
        ]
        .into_iter()
        .map(|(id, name, path)| ProjectChoice {
            id: id.into(),
            name: name.into(),
            path: path.into(),
        })
        .collect()
    }

    fn project_field(chosen: Option<&str>, locked: bool, cx: &mut App) -> ProjectField {
        ProjectField::new(
            "test-project",
            choices(),
            false,
            chosen.map(str::to_owned),
            locked,
            std::rc::Rc::new(|_, _, _| {}),
            cx,
        )
    }

    #[gpui::test]
    fn choosing_a_project_takes_its_folder_unless_the_directory_is_inside(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut field = project_field(Some("runner"), false, cx);
            let worktree = "/repos/yicheng47/runner/.worktrees/feat-968";
            assert_eq!(field.choose("runner", worktree), None);
            assert_eq!(
                field.choose("quill", worktree).as_deref(),
                Some("/repos/yicheng47/quill")
            );
            assert_eq!(
                field.choose("runner", "").as_deref(),
                Some("/repos/yicheng47/runner")
            );
            assert_eq!(field.choose("", worktree), None);
            assert_eq!(field.scope(), ProjectScope::Root);
            assert_eq!(
                field.choose("web", "/repos/yicheng47/runner"),
                Some("/repos/yicheng47/runner-web".into())
            );
            assert_eq!(field.scope(), ProjectScope::Project("web".into()));
        });
    }

    #[gpui::test]
    fn an_explicit_menu_choice_survives_an_unchanged_directory_commit(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let mut field = project_field(Some("runner"), false, cx);
            let directory = "/repos/yicheng47/runner/.worktrees/feature";
            field.choose("", directory);
            assert!(!field.follow(directory, cx));
            assert_eq!(field.scope(), ProjectScope::Root);
            field.choose("parent", directory);
            assert!(!field.follow(directory, cx));
            assert_eq!(field.scope(), ProjectScope::Project("parent".into()));
            assert!(field.follow("/repos/yicheng47/runner/src", cx));
            assert_eq!(field.scope(), ProjectScope::Project("runner".into()));
            field.choose("", directory);
            field.picked(directory, cx);
            assert_eq!(field.scope(), ProjectScope::Project("runner".into()));
        });
    }

    #[gpui::test]
    fn a_committed_directory_chooses_the_deepest_project_holding_it(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut field = project_field(None, false, cx);
            assert!(field.follow("/repos/yicheng47/runner-web/src", cx));
            assert_eq!(field.scope(), ProjectScope::Project("web".into()));
            assert_eq!(field.select.read(cx).value(), "web");
            assert!(field.follow("/repos/yicheng47/runner/.worktrees/x", cx));
            assert_eq!(field.scope(), ProjectScope::Project("runner".into()));
            assert!(field.follow("/repos/yicheng47/notes", cx));
            assert_eq!(field.scope(), ProjectScope::Project("parent".into()));
            assert!(!field.follow("/tmp/scratch", cx));
            assert!(!field.follow("  ", cx));
            assert_eq!(field.scope(), ProjectScope::Project("parent".into()));
        });
    }

    #[gpui::test]
    fn a_locked_field_keeps_its_project(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut field = project_field(Some("runner"), true, cx);
            assert!(field.select.read(cx).is_disabled());
            assert!(!field.follow("/repos/yicheng47/quill", cx));
            assert_eq!(field.scope(), ProjectScope::Project("runner".into()));
        });
    }

    #[gpui::test]
    fn an_unknown_or_vanished_project_falls_back_to_no_project(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let field = project_field(Some("missing"), false, cx);
            assert_eq!(field.scope(), ProjectScope::Root);
            let mut field = project_field(Some("quill"), false, cx);
            field.set_choices(choices().into_iter().take(2).collect(), cx);
            assert_eq!(field.scope(), ProjectScope::Root);
            assert_eq!(field.select.read(cx).value(), "");
        });
    }
}
