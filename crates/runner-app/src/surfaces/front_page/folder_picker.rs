use std::collections::HashSet;
use std::rc::Rc;

use super::FrontPageClient;
use crate::*;
use gpui::{canvas, KeyDownEvent};
use runner_app::ui::{menu::popup_layer, ButtonVariant, TextField};
use runner_core::companion::{Catalog, FolderPage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderChoice {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) project: bool,
}

pub(crate) fn choices(catalog: &Catalog) -> Vec<FolderChoice> {
    let mut seen = HashSet::new();
    catalog
        .projects
        .iter()
        .map(|project| FolderChoice {
            name: project.name.clone(),
            path: project.path.clone(),
            project: true,
        })
        .chain(catalog.recent_folders.iter().map(|path| {
            FolderChoice {
                name: path
                    .rsplit(['/', '\\'])
                    .find(|s| !s.is_empty())
                    .unwrap_or(path)
                    .into(),
                path: path.clone(),
                project: false,
            }
        }))
        .filter(|choice| seen.insert(choice.path.clone()))
        .collect()
}

pub(crate) fn child_path(parent: &str, child: &str, platform: &str) -> String {
    let separator = if platform.eq_ignore_ascii_case("windows") {
        '\\'
    } else {
        '/'
    };
    format!(
        "{}{}{}",
        parent,
        if parent.ends_with(separator) {
            String::new()
        } else {
            separator.to_string()
        },
        child
    )
}

pub(crate) struct FolderPicker {
    client: FrontPageClient,
    computer: String,
    platform: String,
    input: Entity<TextField>,
    choices: Vec<FolderChoice>,
    pub(crate) open: bool,
    anchor: Option<Bounds<Pixels>>,
    popup_width: Option<Pixels>,
    focus: FocusHandle,
    browsing: bool,
    requested: Option<String>,
    page: Option<FolderPage>,
    parent: Option<String>,
    selected: usize,
    return_focus: Option<FocusHandle>,
    loading: bool,
    failed: bool,
    generation: u64,
    validation: u64,
    pub(crate) path_error: Option<String>,
}

impl FolderPicker {
    pub(crate) fn new(
        client: FrontPageClient,
        computer: String,
        platform: String,
        input: Entity<TextField>,
        folder_choices: Vec<FolderChoice>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            client,
            computer,
            platform,
            input,
            choices: folder_choices,
            open: false,
            anchor: None,
            popup_width: None,
            focus: cx.focus_handle(),
            browsing: false,
            requested: None,
            page: None,
            parent: None,
            selected: 0,
            return_focus: None,
            loading: false,
            failed: false,
            generation: 0,
            validation: 0,
            path_error: None,
        }
    }

    pub(crate) fn return_focus(&mut self, focus: Option<FocusHandle>) {
        self.return_focus = focus;
    }

    pub(crate) fn set_popup_width(&mut self, width: Pixels) {
        self.popup_width = Some(width);
    }

    pub(crate) fn set_anchor(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.anchor = Some(anchor);
        cx.notify();
    }

    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.client.supports(&["folders"]) {
            return;
        }
        if self.open {
            self.dismiss(window, cx);
        } else {
            self.open = true;
            self.browsing = false;
            self.selected = 0;
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.client.supports(&["folders"]) {
            return;
        }
        self.open = true;
        self.browsing = true;
        self.focus.focus(window, cx);
        self.load(None, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.loading = false;
        self.generation = self.generation.wrapping_add(1);
        self.return_focus
            .clone()
            .unwrap_or_else(|| self.input.read(cx).focus_handle())
            .focus(window, cx);
        cx.notify();
    }

    fn choose(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.client.valid() {
            self.dismiss(window, cx);
            return;
        }
        if self.browsing
            && (self.loading
                || self.failed
                || !self.page.as_ref().is_some_and(|page| page.path == path))
        {
            return;
        }
        self.validation = self.validation.wrapping_add(1);
        self.input.update(cx, |input, cx| {
            input.reset(path, cx);
            input.set_validation(runner_app::ui::FieldValidation::Valid, cx);
        });
        self.path_error = None;
        self.dismiss(window, cx);
    }

    fn load(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.loading || !self.client.supports(&["folders"]) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        if self.requested != path {
            self.parent = self
                .page
                .as_ref()
                .filter(|page| {
                    page.folders
                        .iter()
                        .any(|folder| Some(child_path(&page.path, folder, &self.platform)) == path)
                })
                .map(|page| page.path.clone());
        }
        self.browsing = true;
        self.requested = path.clone();
        self.page = None;
        self.loading = true;
        self.failed = false;
        self.selected = 0;
        let client = self.client.clone();
        let work = cx.background_spawn(async move { client.folders(path) });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |picker, cx| {
                if !picker.open || picker.generation != generation || !picker.client.valid() {
                    return;
                }
                picker.loading = false;
                match result {
                    Ok(page) => {
                        picker.requested = Some(page.path.clone());
                        picker.parent = page.parent.clone();
                        picker.page = Some(page);
                    }
                    Err(_) => picker.failed = true,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn validate(&mut self, cx: &mut Context<Self>) {
        if self.open || !self.client.supports(&["folders"]) {
            return;
        }
        self.validation = self.validation.wrapping_add(1);
        let validation = self.validation;
        let path = self.input.read(cx).text().to_owned();
        self.path_error = None;
        if path.trim().is_empty() {
            self.input.update(cx, |input, cx| {
                input.set_validation(runner_app::ui::FieldValidation::Valid, cx)
            });
            cx.notify();
            return;
        }
        let client = self.client.clone();
        let request = path.clone();
        let work = cx.background_spawn(async move { client.folders(Some(request)) });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |picker, cx| {
                if picker.validation != validation
                    || picker.input.read(cx).text() != path
                    || !picker.client.valid()
                {
                    return;
                }
                picker.path_error = result.err().map(|_| {
                    crate::surfaces::remote_client::label("client.folder_invalid", &picker.computer)
                });
                picker.input.update(cx, |input, cx| {
                    input.set_validation(
                        picker
                            .path_error
                            .clone()
                            .map(runner_app::ui::FieldValidation::error)
                            .unwrap_or_default(),
                        cx,
                    )
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn anchor(owner: Entity<Self>) -> AnyElement {
        canvas(
            move |bounds, _, cx| {
                let owner = owner.clone();
                cx.defer(move |cx| {
                    owner.update(cx, |picker, cx| {
                        if picker.anchor != Some(bounds) {
                            picker.anchor = Some(bounds);
                            cx.notify();
                        }
                    })
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
}

#[derive(Clone, PartialEq, Eq)]
enum FolderAction {
    Choices,
    Load(Option<String>),
    Choose(String),
}
impl FolderPicker {
    fn keyboard_actions(&self) -> Vec<FolderAction> {
        if self.loading {
            return vec![FolderAction::Choices];
        }
        if !self.browsing {
            return self
                .choices
                .iter()
                .map(|choice| FolderAction::Choose(choice.path.clone()))
                .chain([FolderAction::Load(None)])
                .collect();
        }
        let mut actions = vec![FolderAction::Choices];
        if let Some(parent) = &self.parent {
            actions.push(FolderAction::Load(Some(parent.clone())));
        }
        if self.failed {
            actions.push(FolderAction::Load(self.requested.clone()));
        }
        if let Some(page) = &self.page {
            actions.extend(page.folders.iter().map(|folder| {
                FolderAction::Load(Some(child_path(&page.path, folder, &self.platform)))
            }));
        }
        if !self.failed {
            if let Some(page) = &self.page {
                actions.push(FolderAction::Choose(page.path.clone()));
            }
        }
        actions
    }
    fn activate(&mut self, action: FolderAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading && action != FolderAction::Choices {
            return;
        }
        match action {
            FolderAction::Choices => {
                self.generation += 1;
                self.loading = false;
                self.browsing = false;
                self.selected = 0;
                cx.notify();
            }
            FolderAction::Load(path) => self.load(path, cx),
            FolderAction::Choose(path) => self.choose(path, window, cx),
        }
    }
}

impl FolderPicker {
    fn row(
        &self,
        id: String,
        label: String,
        detail: Option<String>,
        icon: &'static str,
        action: FolderAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.loading && action != FolderAction::Choices;
        let selected = self.keyboard_actions().get(self.selected) == Some(&action);
        let root = cx.entity();
        div()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || format!("HOST_FOLDER_ROW:{id}"))
            .flex_none()
            .px_2()
            .py_1()
            .rounded_md()
            .flex()
            .items_center()
            .gap_2()
            .when(!disabled, |row| row.cursor_pointer())
            .when(disabled, |row| row.opacity(0.5))
            .when(selected, |row| row.bg(theme::raised()))
            .hover(|row| row.bg(theme::raised()))
            .child(
                gpui::svg()
                    .path(icon)
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::muted()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().text_size(theme::text_ui()).child(label))
                    .children(detail.map(|detail| {
                        div()
                            .truncate()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(detail)
                    })),
            )
            .on_click(move |_, window, cx| {
                root.update(cx, |picker, cx| picker.activate(action.clone(), window, cx))
            })
            .into_any_element()
    }
}

impl Render for FolderPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(anchor) = self.anchor.filter(|_| self.open && self.client.valid()) else {
            return div().into_any_element();
        };
        let mut rows = Vec::new();
        if self.browsing {
            rows.push(self.row(
                "choices".into(),
                crate::i18n::text("client.folder_choices").into(),
                None,
                "chevron-left.svg",
                FolderAction::Choices,
                cx,
            ));
            let path = self
                .page
                .as_ref()
                .map(|page| page.path.as_str())
                .or(self.requested.as_deref())
                .unwrap_or_default();
            rows.push(
                div()
                    .px_2()
                    .py_1()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(path.to_owned())
                    .into_any_element(),
            );
            if let Some(parent) = self.parent.clone() {
                rows.push(self.row(
                    "parent".into(),
                    crate::i18n::text("client.folder_parent").into(),
                    None,
                    "arrow-up.svg",
                    FolderAction::Load(Some(parent)),
                    cx,
                ));
            }
            if self.loading {
                rows.push(
                    div()
                        .debug_selector(|| "HOST_FOLDER_LOADING".into())
                        .p_2()
                        .text_size(theme::text_ui())
                        .text_color(theme::muted())
                        .child(crate::i18n::text("client.folder_loading"))
                        .into_any_element(),
                );
            } else if self.failed {
                rows.push(
                    div()
                        .debug_selector(|| "HOST_FOLDER_FAILED".into())
                        .p_2()
                        .text_size(theme::text_ui())
                        .text_color(theme::danger())
                        .child(crate::surfaces::remote_client::label(
                            "client.folder_failed",
                            &self.computer,
                        ))
                        .into_any_element(),
                );
                rows.push(self.row(
                    "retry".into(),
                    crate::i18n::text("ui.retry").into(),
                    None,
                    "rotate-ccw.svg",
                    FolderAction::Load(self.requested.clone()),
                    cx,
                ));
            } else if let Some(page) = &self.page {
                if page.folders.is_empty() {
                    rows.push(
                        div()
                            .debug_selector(|| "HOST_FOLDER_EMPTY".into())
                            .p_2()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("client.folder_empty"))
                            .into_any_element(),
                    );
                }
                for (index, folder) in page.folders.iter().enumerate() {
                    rows.push(self.row(
                        format!("child-{index}"),
                        folder.clone(),
                        None,
                        "folder.svg",
                        FolderAction::Load(Some(child_path(&page.path, folder, &self.platform))),
                        cx,
                    ));
                }
            }
        } else {
            for project in [true, false] {
                if !self.choices.iter().any(|choice| choice.project == project) {
                    continue;
                }
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::text(if project {
                            "heading.projects"
                        } else {
                            "heading.recents"
                        }))
                        .into_any_element(),
                );
                for (index, choice) in self
                    .choices
                    .iter()
                    .enumerate()
                    .filter(|(_, choice)| choice.project == project)
                {
                    rows.push(self.row(
                        format!("choice-{index}"),
                        choice.name.clone(),
                        Some(choice.path.clone()),
                        "folder.svg",
                        FolderAction::Choose(choice.path.clone()),
                        cx,
                    ));
                }
            }
            rows.push(self.row(
                "browse".into(),
                crate::i18n::text("client.folder_browse").into(),
                None,
                "folder.svg",
                FolderAction::Load(None),
                cx,
            ));
        }
        let mut content = div()
            .debug_selector(|| "HOST_FOLDER_MENU".into())
            .track_focus(&self.focus)
            .key_context("HostFolders")
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(theme::panel())
            .border_1()
            .border_color(theme::border())
            .rounded(rems(6. / 16.))
            .on_key_down(cx.listener(|picker, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" | "tab" => {
                        cx.stop_propagation();
                        picker.dismiss(window, cx);
                    }
                    "up" | "down" => {
                        cx.stop_propagation();
                        let count = picker.keyboard_actions().len();
                        if count > 0 {
                            picker.selected = if event.keystroke.key == "up" {
                                (picker.selected + count - 1) % count
                            } else {
                                (picker.selected + 1) % count
                            };
                            cx.notify();
                        }
                    }
                    "enter" | "space" if !event.is_held => {
                        cx.stop_propagation();
                        if let Some(action) =
                            picker.keyboard_actions().get(picker.selected).cloned()
                        {
                            picker.activate(action, window, cx);
                        }
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(crate::surfaces::remote_client::label(
                        "client.folders_on",
                        &self.computer,
                    )),
            )
            .child(
                div()
                    .id("host-folder-scroll")
                    .max_h(rems(280. / 16.))
                    .overflow_y_scroll()
                    .scrollbar_width(px(0.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows),
            );
        if self.browsing {
            let root = cx.entity();
            let path = self
                .page
                .as_ref()
                .filter(|_| !self.failed)
                .map(|page| page.path.clone());
            content = content.child(
                div().debug_selector(|| "HOST_FOLDER_USE".into()).child(
                    Button::new("host-folder-use", crate::i18n::text("client.folder_use"))
                        .variant(ButtonVariant::Secondary)
                        .size(ButtonSize::Sm)
                        .disabled(self.loading || self.failed || path.is_none())
                        .on_press(move |window, cx| {
                            if let Some(path) = &path {
                                root.update(cx, |picker, cx| {
                                    picker.choose(path.clone(), window, cx)
                                });
                            }
                        }),
                ),
            );
        }
        let root = cx.entity();
        popup_layer(
            anchor,
            window,
            self.popup_width.unwrap_or(anchor.size.width),
            content.into_any_element(),
            Rc::new(move |window, cx| root.update(cx, |picker, cx| picker.dismiss(window, cx))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::remote_client::tests::{
        add_window, choose_host, enable_front_page, fixture,
    };
    use gpui::{TestAppContext, VisualTestContext};

    #[test]
    fn pending_listing_blocks_navigation_and_selection_but_back_discards_its_reply() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "pending-folders");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let picker = window
            .read_with(&cx, |root, _| {
                root.start_chat_modal
                    .as_ref()
                    .unwrap()
                    .test_folder_picker()
                    .unwrap()
            })
            .unwrap();
        window
            .update(&mut cx, |_, window, cx| {
                picker.update(cx, |picker, cx| {
                    picker.browse(window, cx);
                    assert!(picker.loading);
                    assert_eq!(picker.keyboard_actions().len(), 1);
                    assert!(picker.keyboard_actions()[0] == FolderAction::Choices);
                    picker.activate(FolderAction::Load(Some("other".into())), window, cx);
                    picker.choose("not-listed".into(), window, cx);
                    assert_eq!(picker.requested, None);
                    assert!(picker.input.read(cx).text().is_empty());
                    picker.activate(FolderAction::Choices, window, cx);
                    assert!(!picker.loading);
                    assert!(!picker.browsing);
                })
            })
            .unwrap();
        cx.run_until_parked();
        picker.read_with(&cx, |picker, _| {
            assert!(picker.page.is_none());
            assert!(!picker.browsing);
        });
        assert_eq!(fake.request_count("folders"), 1);
        fake.queue_result(
            "folders",
            Err(runner_core::protocol::ClientError::msg("cannot list")),
        );
        window
            .update(&mut cx, |_, window, cx| {
                picker.update(cx, |picker, cx| picker.browse(window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |_, window, cx| {
                picker.update(cx, |picker, cx| {
                    assert!(picker.failed);
                    assert!(!picker
                        .keyboard_actions()
                        .iter()
                        .any(|action| matches!(action, FolderAction::Choose(_))));
                    picker.choose("unlisted-folder".into(), window, cx);
                    assert!(picker.open);
                    assert!(picker.input.read(cx).text().is_empty());
                })
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let menu = visual.debug_bounds("HOST_FOLDER_MENU").unwrap();
        assert_eq!(
            menu.size.width,
            picker.read_with(&visual, |picker, _| picker.anchor.unwrap().size.width)
        );
        let use_folder = visual.debug_bounds("HOST_FOLDER_USE").unwrap();
        visual.simulate_click(use_folder.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("HOST_FOLDER_FAILED").is_some());
        assert_eq!(fake.request_count("folders"), 2);
        let retry = visual.debug_bounds("HOST_FOLDER_ROW:retry").unwrap();
        visual.simulate_click(retry.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(fake.request_count("folders"), 3);
        window
            .update(&mut visual, |_, window, cx| {
                picker.update(cx, |picker, cx| {
                    assert!(picker.page.as_ref().unwrap().folders.is_empty());
                    let path = picker.page.as_ref().unwrap().path.clone();
                    assert!(picker
                        .keyboard_actions()
                        .contains(&FolderAction::Choose(path.clone())));
                    picker.choose(path.clone(), window, cx);
                    assert_eq!(picker.input.read(cx).text(), path);
                    assert!(!picker.open);
                })
            })
            .unwrap();
    }

    #[test]
    fn new_project_menu_has_a_readable_width_independent_of_the_plus_anchor() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for zoom in [1., 1.5] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            store.update(&mut cx, |store, _| store.settings.app_zoom = zoom);
            let window = add_window(&mut cx, &store, temp.path(), "project-menu-width");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            visual.run_until_parked();
            let plus = visual.debug_bounds("SIDEBAR_NEW_PROJECT").unwrap();
            visual.simulate_click(plus.center(), Default::default());
            visual.run_until_parked();
            let menu = visual.debug_bounds("HOST_FOLDER_MENU").unwrap();
            assert!(plus.size.width <= px(32. * zoom));
            assert_eq!(menu.size.width, px(320. * zoom));
            assert!(visual.debug_bounds("HOST_FOLDER_EMPTY").is_some());
            window
                .read_with(&visual, |root, cx| {
                    let picker = root
                        .remote_window
                        .as_ref()
                        .unwrap()
                        .project_picker
                        .as_ref()
                        .unwrap()
                        .read(cx);
                    assert_eq!(picker.anchor.unwrap().size, plus.size);
                })
                .unwrap();
        }
    }
}
