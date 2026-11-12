//! Root resolution and selection, including the root picker and edge states.

use super::*;

const PICKER_WIDTH: f32 = 300.;

fn subject(context: &PanelContext) -> FilesSubject {
    match &context.subject {
        PanelSubject::Session(id) => FilesSubject::Session(id.clone()),
        PanelSubject::Mission(id) => FilesSubject::Mission(id.clone()),
    }
}

/// The last two parts of a missing path, so the message names the folder
/// without the whole path: `…/.worktrees/fix-1116`.
fn compact_missing_path(path: &Path) -> String {
    let parts = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if parts.len() <= 3 {
        return path.display().to_string();
    }
    format!(
        "…{}{}",
        std::path::MAIN_SEPARATOR,
        parts[parts.len() - 2..].join(std::path::MAIN_SEPARATOR_STR)
    )
}

impl FilesView {
    pub(in super::super) fn root_path(&self) -> Option<&Path> {
        match &self.body {
            Body::Root(state) => Some(&state.root.path),
            _ => None,
        }
    }

    pub(in super::super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(in super::super) fn link_root(&self) -> LinkRoot {
        match &self.body {
            Body::Loading => LinkRoot::Pending,
            Body::Root(state) => LinkRoot::Root(state.root.path.clone()),
            Body::Remote | Body::Choose | Body::Missing { .. } | Body::Failed(_) => LinkRoot::None,
        }
    }

    pub(super) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.body = Body::Remote;
            self.sync_watch(cx);
            self.settle_links(cx);
            cx.notify();
            return;
        }
        let subject = subject(&self.context);
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| this.apply_outcome(result, cx),
        );
    }

    /// Saves `path` as the root. A path the daemon refuses leaves the view as
    /// it was, with the error shown, so another path can be tried.
    pub(in super::super) fn set_root(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.context.remote {
            return;
        }
        self.picker = None;
        self.root_change += 1;
        let change = self.root_change;
        let subject = subject(&self.context);
        let client = self.client.clone();
        let previous = self.root_chain.take();
        #[cfg(test)]
        let gate = self.root_gate.take();
        let (reply, result) = futures::channel::oneshot::channel();
        self.root_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            #[cfg(test)]
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            let _ = reply.send(client.files_set_root(&subject, path.as_deref()));
        }));
        cx.spawn(async move |this, cx| {
            let Ok(result) = result.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.root_change != change {
                    return;
                }
                match result {
                    Ok(outcome) => this.apply_outcome(Ok(outcome), cx),
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_outcome(
        &mut self,
        outcome: Result<FilesRootOutcome, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let same = matches!(
            &outcome,
            Ok(FilesRootOutcome::Root(root)) if self.root_path() == Some(root.path.as_path())
        );
        if !same {
            self.generation += 1;
        }
        self.error = None;
        self.status_error = None;
        match outcome {
            Ok(FilesRootOutcome::Root(root)) => match &mut self.body {
                Body::Root(state) if same => state.root = root,
                _ => {
                    self.reset_viewer();
                    self.finder_root_changed();
                    let tree = FileTree::new(root.cwd.clone());
                    let loads = tree.initial_loads();
                    self.body = Body::Root(Box::new(RootState {
                        root,
                        tree,
                        listed: Default::default(),
                        next_listing: 0,
                    }));
                    self.scroll = UniformListScrollHandle::new();
                    let owner = cx.entity_id();
                    let base = self.scroll.0.borrow().base_handle.clone();
                    self.scrollbar = cx.new(|_| Scrollbar::app(base, owner));
                    self.load(loads, cx);
                }
            },
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.reset_viewer();
                self.body = Body::Choose;
                self.load_recent(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.reset_viewer();
                self.body = Body::Missing { path };
            }
            Err(error) => {
                self.reset_viewer();
                self.body = Body::Failed(error.to_string().into());
            }
        }
        if same {
            self.read_again_after_resolve(cx);
        }
        if !self.searchable() {
            self.finder = None;
        }
        self.load_finder_index(cx);
        self.watched_root_changed(cx);
        self.settle_links(cx);
        cx.notify();
    }

    fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.request(
            cx,
            |client| client.files_recent_folders(),
            |this, result, cx| {
                if let Ok(recent) = result {
                    this.recent = recent;
                    this.rebuild_picker(cx);
                    cx.notify();
                }
            },
        );
    }

    /// "chat", "terminal" or "mission", for the copy that names the subject.
    pub(super) fn noun(&self) -> String {
        crate::i18n::text(match self.context.surface {
            PanelSurface::Chat => "noun.chat",
            PanelSurface::Terminal => "noun.terminal",
            PanelSurface::Mission => "noun.mission",
        })
        .to_owned()
    }

    fn projects(&self, cx: &App) -> Vec<(String, PathBuf)> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .filter(|project| !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
            .collect()
    }

    fn own_project(&self, cx: &App) -> Option<(String, PathBuf)> {
        let id = self.context.project.as_deref()?;
        crate::app_store::global_app_store(cx)
            .read(cx)
            .projects
            .iter()
            .find(|project| project.id == id && !project.cwd.trim().is_empty())
            .map(|project| (project.name.clone(), PathBuf::from(&project.cwd)))
    }

    fn home(cx: &App) -> Option<PathBuf> {
        crate::app_store::global_app_store(cx)
            .read(cx)
            .home_dir
            .clone()
    }

    pub(super) fn apply_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.choose_field.read(cx).text().to_owned();
        match picker::typed_path(&text, Self::home(cx).as_deref()) {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.focus.focus(window, cx);
            }
            None => {
                self.error = Some(crate::i18n::text("ui.type_a_full_path").into());
                cx.notify();
            }
        }
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.choose_folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.set_root(Some(path), cx));
        })
        .detach();
        cx.notify();
    }

    // ── The picker ─────────────────────────────────────────────────────

    fn picker_base(&self, cx: &App) -> Option<PathBuf> {
        match &self.body {
            Body::Root(state) => Some(state.root.path.clone()),
            Body::Missing { .. } => self.own_project(cx).map(|(_, path)| path),
            _ => None,
        }
    }

    fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("ui.select_a_worktree_or_type_a_path"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| this.on_picker_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.picker {
                if picker.query != query {
                    picker.query = query;
                    picker.moved = false;
                    picker.highlighted = picker::default_highlight(
                        picker::filter(&picker.items, &picker.query).len(),
                    );
                    picker.scroll.set_offset(gpui::Point::default());
                    cx.notify();
                }
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.picker = Some(Picker {
            field,
            query: String::new(),
            worktrees: Vec::new(),
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            _observe: observe,
        });
        self.rebuild_picker(cx);
        self.load_recent(cx);
        if let Some(base) = self.picker_base(cx) {
            self.request(
                cx,
                move |client| client.files_worktrees(&base),
                |this, result, cx| {
                    if let (Some(picker), Ok(worktrees)) = (&mut this.picker, result) {
                        picker.worktrees = worktrees;
                        this.rebuild_picker(cx);
                        cx.notify();
                    }
                },
            );
        }
        cx.notify();
    }

    fn rebuild_picker(&mut self, cx: &mut Context<Self>) {
        let current = match &self.body {
            Body::Root(state) => state.root.path.clone(),
            Body::Missing { path } => path.clone(),
            _ => PathBuf::new(),
        };
        let projects = if self.context.project.is_none() {
            self.projects(cx)
        } else {
            Vec::new()
        };
        let home = Self::home(cx);
        let Some(picker) = &mut self.picker else {
            return;
        };
        picker.items = picker::picker_items(PickerSources {
            current: &current,
            worktrees: &picker.worktrees,
            recent: &self.recent,
            projects: &projects,
            home: home.as_deref(),
            main_checkout_label: crate::i18n::text("ui.main_checkout"),
            detached_label: crate::i18n::text("ui.detached"),
            choose_folder_label: crate::i18n::text("ui.choose_folder"),
        });
        let visible = picker::filter(&picker.items, &picker.query).len();
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.saturating_sub(1))
        } else {
            picker::default_highlight(visible)
        };
    }

    fn visible_picker_items(&self) -> Vec<usize> {
        self.picker
            .as_ref()
            .map(|picker| picker::filter(&picker.items, &picker.query))
            .unwrap_or_default()
    }

    fn on_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let visible = self.visible_picker_items();
        let Some(picker) = &mut self.picker else {
            return false;
        };
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.moved = true;
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.moved = true;
                picker.reveal.set(true);
            }
            "escape" => {
                self.picker = None;
                self.focus.focus(window, cx);
            }
            "enter" => {
                let typed = picker::typed_path(&picker.query, Self::home(cx).as_deref());
                match typed {
                    Some(path) => self.set_root(Some(path), cx),
                    None => {
                        if let Some(index) = visible.get(picker.highlighted).copied() {
                            self.activate_picker_item(index, window, cx);
                        }
                    }
                }
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn activate_picker_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.items.get(index))
            .cloned()
        else {
            return;
        };
        match item.path {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.focus.focus(window, cx);
            }
            None => self.browse(cx),
        }
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let (name, icon, missing) = match &self.body {
            Body::Root(state) => (
                state.root.name.clone(),
                match &state.root.git {
                    Some(git) if git.worktree == WorktreeKind::Linked => "git-branch.svg",
                    Some(_) => "folder-git-2.svg",
                    None => "folder.svg",
                },
                false,
            ),
            Body::Missing { path } => (picker::folder_name(path), "git-branch.svg", true),
            _ => return div().into_any_element(),
        };
        let anchor = self.picker_anchor.clone();
        let open = self.picker.is_some();
        let ink = if missing {
            theme::faint()
        } else {
            theme::text()
        };
        let trigger = div()
            .id("files-root-picker")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_ROOT_BUTTON".into())
            })
            .relative()
            .min_w(px(0.))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_md()
            .cursor_pointer()
            .text_color(ink)
            .when(open, |element| {
                element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
            })
            .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(svg().flex_none().size(px(14.)).path(icon).text_color(ink))
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_ui())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(12.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );
        let status = match self.tree_ref() {
            Some(tree) if self.root_is_git() => Some(match tree.status() {
                None => div()
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_STATUS_READING".into())
                    })
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(theme::faint())
                    .child(
                        svg()
                            .size(px(12.))
                            .path("loader.svg")
                            .text_color(theme::faint()),
                    )
                    .child(crate::i18n::text("ui.reading_status"))
                    .into_any_element(),
                Some(status) if status.changed > 0 => div()
                    .when(cfg!(test), |element| {
                        let count = status.changed;
                        element.debug_selector(move || format!("FILES_CHANGED {count}"))
                    })
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(theme::muted())
                    .child(div().size(px(6.)).rounded_full().bg(theme::warning()))
                    .child(crate::i18n::tn("count.changed", status.changed))
                    .into_any_element(),
                Some(_) => div().into_any_element(),
            }),
            _ => None,
        };
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_ROOT_HEADER".into())
            })
            .flex_none()
            .h(px(44.))
            .pl(px(10.))
            .pr(px(16.))
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .text_size(theme::text_meta())
            .child(div().min_w(px(0.)).flex_shrink(1.).child(trigger))
            .child(div().flex_1())
            .children(status)
            .when(!missing, |element| {
                element.child(
                    div()
                        .id("files-search")
                        .when(cfg!(test), |element| {
                            element.debug_selector(|| "FILES_SEARCH".into())
                        })
                        .flex_none()
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .when(self.finder.is_some(), |element| {
                            element.bg(theme::sidebar_selected())
                        })
                        .hover(|element| {
                            element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                        })
                        .tooltip(|_, cx| {
                            runner_app::ui::tooltip::tooltip_view(
                                crate::i18n::t("ui.search_files"),
                                cx,
                            )
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_finder(window, cx)))
                        .child(
                            svg()
                                .size(px(14.))
                                .path("search.svg")
                                .text_color(theme::muted()),
                        ),
                )
            })
            .into_any_element()
    }

    pub(in super::super) fn root_is_git(&self) -> bool {
        matches!(&self.body, Body::Root(state) if state.root.git.is_some())
    }

    pub(super) fn render_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let visible = picker::filter(&picker.items, &picker.query);
        let mut rows = Vec::new();
        let mut last_kind: Option<PickerKind> = None;
        for (position, index) in visible.iter().copied().enumerate() {
            let item = &picker.items[index];
            let section = match (last_kind, item.kind) {
                (Some(PickerKind::Project), PickerKind::Project)
                | (Some(PickerKind::Recent), PickerKind::Recent)
                | (Some(PickerKind::Worktree(_)), PickerKind::Worktree(_)) => None,
                (_, PickerKind::Project) => Some(crate::i18n::text("ui.projects")),
                (_, PickerKind::Recent) => Some(crate::i18n::text("ui.recent")),
                _ => None,
            };
            let divider = last_kind.is_some()
                && last_kind != Some(item.kind)
                && !matches!(
                    (last_kind, item.kind),
                    (Some(PickerKind::Worktree(_)), PickerKind::Worktree(_))
                );
            last_kind = Some(item.kind);
            if divider {
                rows.push(
                    div()
                        .my(px(4.))
                        .h(px(1.))
                        .bg(theme::border())
                        .into_any_element(),
                );
            }
            if let Some(section) = section {
                rows.push(
                    div()
                        .px(px(10.))
                        .pt(px(4.))
                        .pb(px(2.))
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(section)
                        .into_any_element(),
                );
            }
            let highlighted = position == picker.highlighted;
            if highlighted && picker.reveal.replace(false) {
                picker.scroll.scroll_to_item(rows.len());
            }
            let icon = if item.current {
                "check.svg"
            } else {
                match item.kind {
                    PickerKind::Worktree(WorktreeKind::Main) => "folder-git-2.svg",
                    PickerKind::Worktree(WorktreeKind::Linked) => "git-branch.svg",
                    PickerKind::Recent | PickerKind::Project => "folder.svg",
                    PickerKind::ChooseFolder => "folder-search.svg",
                }
            };
            rows.push(
                div()
                    .id(("files-picker-item", position))
                    .when(cfg!(test), |element| {
                        element.debug_selector(move || format!("FILES_PICKER_ITEM {position}"))
                    })
                    .px(px(10.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(highlighted, |element| element.bg(theme::sidebar_selected()))
                    .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_picker_item(index, window, cx)
                    }))
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path(icon)
                            .text_color(if item.current {
                                theme::accent()
                            } else {
                                theme::muted()
                            }),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::text())
                                    .child(item.title.clone()),
                            )
                            .when(!item.subtitle.is_empty(), |element| {
                                element.child(
                                    div()
                                        .truncate()
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .child(item.subtitle.clone()),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_PICKER".into())
            })
            .flex()
            .flex_col()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path("search.svg")
                            .text_color(theme::faint()),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(picker.field.clone())),
            )
            .child(
                div()
                    .id("files-picker-list")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILES_PICKER_LIST".into())
                    })
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&picker.scroll)
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_sized(
            self.picker_anchor.get(),
            window,
            PopupWidth::Fixed(px(PICKER_WIDTH)),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| {
                    this.picker = None;
                    this.focus.focus(window, cx);
                    cx.notify();
                })
            }),
        ))
    }

    pub(super) fn render_message(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: &'static str,
        body: String,
        actions: Option<AnyElement>,
    ) -> AnyElement {
        div()
            .when(cfg!(test), |element| {
                element.debug_selector(move || selector.into())
            })
            .flex_1()
            .min_h(px(0.))
            .px(px(28.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_center()
            .child(svg().size(px(28.)).path(icon).text_color(theme::faint()))
            .child(
                div()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_text("FILES_TITLE", element);
                        element
                    })
                    .child(title),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .map(|element| {
                        #[cfg(test)]
                        let element = crate::theme_snapshot::record_text("FILES_BODY", element);
                        element
                    })
                    .child(body),
            )
            .children(actions.map(|actions| div().pt(px(8.)).child(actions)))
            .into_any_element()
    }

    pub(super) fn render_choose(&self, cx: &mut Context<Self>) -> AnyElement {
        let home = Self::home(cx);
        let entry = |id: (&'static str, usize),
                     name: String,
                     path: PathBuf,
                     cx: &mut Context<Self>| {
            let subtitle = abbreviate_home(HomePath::Native(&path, home.as_deref()));
            div()
                .id(id)
                .when(cfg!(test), |element| {
                    element.debug_selector(move || format!("FILES_CHOOSE {} {}", id.0, id.1))
                })
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .cursor_pointer()
                .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_root(Some(path.clone()), cx)))
                .child(
                    svg()
                        .flex_none()
                        .size(px(16.))
                        .path("folder.svg")
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_ui())
                                .text_color(theme::text())
                                .child(name),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_meta())
                                .text_color(theme::faint())
                                .child(subtitle),
                        ),
                )
                .into_any_element()
        };
        let section = |label: &'static str| {
            div()
                .px(px(12.))
                .pt(px(14.))
                .pb(px(4.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(label)
        };
        let projects = self
            .projects(cx)
            .into_iter()
            .enumerate()
            .map(|(index, (name, path))| entry(("files-choose-project", index), name, path, cx))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                entry(
                    ("files-choose-recent", index),
                    picker::folder_name(&path),
                    path,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let browse = cx.entity();
        div()
            .id("files-choose")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_CHOOSE".into())
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px(px(16.))
            .pt(px(28.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        svg()
                            .size(px(22.))
                            .path("folder-open.svg")
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .pt(px(4.))
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .map(|element| {
                                #[cfg(test)]
                                let element =
                                    crate::theme_snapshot::record_text("FILES_TITLE", element);
                                element
                            })
                            .child(crate::i18n::text("ui.choose_a_folder_to_show")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "format.files_choose_folder_body",
                                &[("subject", self.noun())],
                            )),
                    ),
            )
            .child(
                div()
                    .pt(px(16.))
                    .relative()
                    .child(self.choose_field.clone())
                    .child(
                        div()
                            .absolute()
                            .top(px(16.))
                            .right(px(4.))
                            .bottom_0()
                            .flex()
                            .items_center()
                            .child(
                                IconButton::new("files-browse", "folder-search.svg")
                                    .tooltip(crate::i18n::text("ui.browse"))
                                    .on_press(move |_, cx| {
                                        browse.update(cx, |this, cx| this.browse(cx))
                                    }),
                            ),
                    ),
            )
            .when(!projects.is_empty(), |element| {
                element
                    .child(section(crate::i18n::text("ui.projects")))
                    .children(projects)
            })
            .when(!recent.is_empty(), |element| {
                element
                    .child(section(crate::i18n::text("ui.recent")))
                    .children(recent)
            })
            .into_any_element()
    }

    pub(super) fn render_missing(&self, path: &Path, cx: &mut Context<Self>) -> AnyElement {
        let project = self.own_project(cx);
        let show_root = cx.entity();
        let choose_root = cx.entity();
        let actions = div()
            .flex()
            .gap_2()
            .justify_center()
            .children(project.map(|(name, path)| {
                Button::new(
                    "files-show-main-checkout",
                    crate::i18n::tf("format.show_folder", &[("name", name)]),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("FILES_SHOW_MAIN")
                .on_press(move |_, cx| {
                    let path = path.clone();
                    show_root.update(cx, |this, cx| this.set_root(Some(path), cx));
                })
            }))
            .child(
                Button::new("files-choose-root", crate::i18n::text("ui.choose_ellipsis"))
                    .variant(ButtonVariant::Outline)
                    .on_press(move |window, cx| {
                        choose_root.update(cx, |this, cx| this.toggle_picker(window, cx))
                    }),
            )
            .into_any_element();
        let shown = compact_missing_path(path);
        self.render_message(
            "FILES_MISSING",
            "folder-x.svg",
            crate::i18n::text("ui.this_folder_no_longer_exists"),
            crate::i18n::tf("format.folder_was_moved_or_deleted", &[("path", shown)]),
            Some(actions),
        )
    }
}
