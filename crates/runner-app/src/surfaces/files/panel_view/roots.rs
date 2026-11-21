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
pub(crate) fn compact_missing_path(path: &Path) -> String {
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
                    Ok(outcome) => {
                        let panel_subject = this.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = this.client.clone();
                        this.apply_outcome(Ok(outcome), cx);
                        crate::surfaces::files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
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
                        uncommitted_paths: None,
                        uncommitted_seq: 0,
                    }));
                    self.scroll = UniformListScrollHandle::new();
                    let owner = cx.entity_id();
                    let base = self.scroll.0.borrow().base_handle.clone();
                    self.scrollbar = cx.new(|_| Scrollbar::app(base, owner));
                    self.load(loads, cx);
                    if let Some(saved) = self.pending_restore.take() {
                        self.apply_saved_files(saved, cx);
                    }
                }
            },
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.pending_restore = None;
                self.reset_viewer();
                self.body = Body::Choose;
                self.load_recent(cx);
                self.notify_owner(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.pending_restore = None;
                self.reset_viewer();
                self.body = Body::Missing { path };
                self.notify_owner(cx);
            }
            Err(error) => {
                self.pending_restore = None;
                self.reset_viewer();
                self.body = Body::Failed(error.to_string().into());
                self.notify_owner(cx);
            }
        }
        if same {
            self.read_again_after_resolve(cx);
            self.refresh_uncommitted_paths(true, cx);
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
        let initial_worktrees = match &self.body {
            Body::Root(state) => state
                .root
                .git
                .as_ref()
                .map(|git| git.worktrees.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        field.read(cx).focus_handle().focus(window, cx);
        self.picker = Some(Picker {
            field,
            query: String::new(),
            worktrees: initial_worktrees,
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
        let (current, own_checkout_top) = match &self.body {
            Body::Root(state) => (state.root.path.clone(), state.root.own_checkout_top.clone()),
            Body::Missing { path } => (path.clone(), None),
            _ => (PathBuf::new(), None),
        };
        let projects = if self.context.project.is_none() {
            self.projects(cx)
        } else {
            Vec::new()
        };
        let home = Self::home(cx);
        let context = self.context.clone();
        let Some(picker) = &mut self.picker else {
            return;
        };
        let session_labels = picker::collect_checkout_session_labels(
            &context,
            own_checkout_top.as_deref(),
            &picker.worktrees,
            cx,
        );
        picker.items = picker::picker_items(PickerSources {
            current: &current,
            worktrees: &picker.worktrees,
            session_labels: &session_labels,
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
                let is_own_worktree = matches!(item.kind, PickerKind::Worktree(_))
                    && matches!(
                        &self.body,
                        Body::Root(state)
                            if state
                                .root
                                .own_checkout_top
                                .as_deref()
                                .is_some_and(|own| picker::same_checkout_path(own, &path))
                    );
                self.set_root((!is_own_worktree).then_some(path), cx);
                self.focus.focus(window, cx);
            }
            None => self.browse(cx),
        }
    }

    fn open_root_overflow_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root_path) = self.root_path().map(Path::to_path_buf) else {
            return;
        };
        type RootMenuAction = Box<dyn Fn(&mut FilesView, &mut Window, &mut Context<FilesView>)>;
        let mut items = Vec::new();
        let mut actions: Vec<RootMenuAction> = Vec::new();
        items.push(MenuItem::new(crate::i18n::message("ui.copy_path")).icon("copy.svg"));
        let copied = root_path.clone();
        actions.push(Box::new(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(
                copied.to_string_lossy().into_owned(),
            ));
        }));
        items.push(MenuItem::new(reveal_label()).icon("app-window.svg"));
        let reveal_path = root_path;
        actions.push(Box::new(move |_, _, cx| {
            let path = reveal_path.clone();
            let task = cx.background_spawn(async move { reveal(path) });
            cx.spawn(async move |this, cx| {
                if let Err(error) = task.await {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error.into());
                        cx.notify();
                    });
                }
            })
            .detach();
        }));
        items.push(
            MenuItem::new(crate::i18n::message("ui.choose_another_folder")).icon("folder.svg"),
        );
        actions.push(Box::new(|this, _, cx| {
            this.browse(cx);
        }));
        let actions = Rc::new(actions);
        let root = cx.weak_entity();
        let dismiss_root = root.clone();
        let return_focus = self.focus.clone();
        let menu = cx.new(move |menu_cx| {
            ContextMenu::new(
                "files-root-overflow-menu",
                menu_cx.focus_handle(),
                event.position,
                items,
                Rc::new(move |index, window, cx| {
                    let actions = actions.clone();
                    let _ = root.update(cx, |this, cx| {
                        if let Some(action) = actions.get(index) {
                            action(this, window, cx);
                        }
                    });
                }),
                Rc::new(move |_, cx| {
                    let _ = dismiss_root.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                }),
            )
            .return_focus(Some(return_focus))
        });
        menu.read(cx).focus_handle().focus(window, cx);
        self.menu = Some(menu);
        cx.notify();
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
            .px(px(6.))
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

        let action_btn = |id: &'static str,
                          selector: &'static str,
                          icon_path: &'static str,
                          tip_key: &'static str| {
            div()
                .id(id)
                .when(cfg!(test), move |element| {
                    element.debug_selector(move || selector.into())
                })
                .flex_none()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(|element| element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                .tooltip(move |_, cx| {
                    runner_app::ui::tooltip::tooltip_view(crate::i18n::t(tip_key), cx)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    svg()
                        .size(px(13.))
                        .path(icon_path)
                        .text_color(theme::muted()),
                )
        };

        let top_row = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILES_ROOT_HEADER".into())
            })
            .flex_none()
            .h(px(38.))
            .pl(px(8.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap_1()
            .text_size(theme::text_meta())
            .child(div().min_w(px(0.)).flex_shrink(1.).child(trigger))
            .child(div().flex_1())
            .when(!missing, |element| {
                element
                    .child(
                        action_btn(
                            "files-collapse-all",
                            "FILES_COLLAPSE_ALL",
                            "chevrons-down-up.svg",
                            "ui.collapse_all_folders",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.collapse_all_folders(cx))),
                    )
                    .child(
                        action_btn(
                            "files-refresh",
                            "FILES_REFRESH",
                            "refresh-cw.svg",
                            "ui.refresh",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_tree(cx))),
                    )
                    .child(
                        div()
                            .id("files-root-more")
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "FILES_ROOT_MORE".into())
                            })
                            .flex_none()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                            .tooltip(|_, cx| {
                                runner_app::ui::tooltip::tooltip_view(
                                    crate::i18n::t("ui.more_actions"),
                                    cx,
                                )
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.open_root_overflow_menu(event, window, cx);
                                }),
                            )
                            .child(
                                svg()
                                    .size(px(13.))
                                    .path("more-horizontal.svg")
                                    .text_color(theme::muted()),
                            ),
                    )
            });

        let following_bar = self.context.following.clone().map(|following| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "FILES_FOLLOWING_BADGE".into())
                })
                .flex_none()
                .px(px(10.))
                .py(px(4.))
                .bg(theme::with_alpha(theme::raised(), 0.45))
                .border_t_1()
                .border_color(theme::border())
                .text_size(theme::text_caption())
                .text_color(theme::muted())
                .truncate()
                .child(following)
        });

        let controls = (!missing).then(|| {
            let is_git = self.root_is_git();
            let status = self.tree_ref().and_then(|tree| tree.status());
            let changed_count = status.map_or(0, |s| s.changed);
            let reading_status = is_git && status.is_none();
            let active_filter = if is_git { self.filter } else { TreeFilter::All };

            let segmented = is_git.then(|| {
                let seg = |id: &'static str,
                           selector: &'static str,
                           active: bool,
                           target: TreeFilter,
                           label: AnyElement| {
                    div()
                        .id(id)
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || {
                                if active {
                                    format!("{selector} active")
                                } else {
                                    selector.into()
                                }
                            })
                        })
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(22.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(5.))
                        .cursor_pointer()
                        .text_size(theme::text_meta())
                        .when(active, |el| {
                            el.bg(theme::raised())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                        })
                        .when(!active, |el| {
                            el.text_color(theme::muted())
                                .hover(|h| h.text_color(theme::text()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.filter != target {
                                this.filter = target;
                                cx.notify();
                            }
                        }))
                        .child(label)
                };

                let changed_label = div()
                    .when(cfg!(test) && changed_count > 0, |el| {
                        el.debug_selector(move || format!("FILES_CHANGED {changed_count}"))
                    })
                    .when(cfg!(test) && reading_status, |el| {
                        el.debug_selector(|| "FILES_STATUS_READING".into())
                    })
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .truncate()
                    .when(changed_count > 0, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .size(px(5.))
                                .rounded_full()
                                .bg(theme::warning()),
                        )
                    })
                    .when(reading_status, |el| {
                        el.child(
                            svg()
                                .flex_none()
                                .size(px(11.))
                                .path("loader.svg")
                                .text_color(theme::faint()),
                        )
                    })
                    .child(if changed_count > 0 {
                        crate::i18n::tf(
                            "files.changed_count",
                            &[("count", changed_count.to_string())],
                        )
                    } else {
                        crate::i18n::text("files.changed_tab").to_owned()
                    })
                    .into_any_element();

                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "FILES_FILTER_TABS".into())
                    })
                    .p(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(seg(
                        "files-filter-all",
                        "FILES_FILTER_ALL",
                        active_filter == TreeFilter::All,
                        TreeFilter::All,
                        div()
                            .truncate()
                            .child(crate::i18n::text("files.all_files"))
                            .into_any_element(),
                    ))
                    .child(seg(
                        "files-filter-changed",
                        "FILES_FILTER_CHANGED",
                        active_filter == TreeFilter::Changed,
                        TreeFilter::Changed,
                        changed_label,
                    ))
            });

            let shortcut = crate::surfaces::files::settings(cx)
                .and_then(|s| crate::keymap::effective_binding("search-files", &s.keymap_overrides))
                .map(|combo| crate::keymap::format_combo(&combo));

            let find_bar = div()
                .id("files-search")
                .when(cfg!(test), |element| {
                    element.debug_selector(|| "FILES_SEARCH".into())
                })
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(6.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .cursor_pointer()
                .when(self.finder.is_some(), |element| {
                    element.bg(theme::sidebar_selected())
                })
                .hover(|element| element.bg(theme::with_alpha(theme::raised(), 0.7)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| this.toggle_finder(window, cx)))
                .child(
                    svg()
                        .flex_none()
                        .size(px(13.))
                        .path("search.svg")
                        .text_color(theme::faint()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("ui.find_files")),
                )
                .children(shortcut.map(|combo| {
                    div()
                        .flex_none()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_micro())
                        .text_color(theme::faint())
                        .child(combo)
                }));

            div()
                .flex_none()
                .px(px(8.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(segmented)
                .child(find_bar)
        });

        div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border())
            .child(top_row)
            .children(following_bar)
            .children(controls)
            .into_any_element()
    }

    pub(in super::super) fn root_is_git(&self) -> bool {
        matches!(&self.body, Body::Root(state) if state.root.git.is_some())
    }

    pub(in super::super) fn root_has_head(&self) -> bool {
        matches!(
            &self.body,
            Body::Root(state) if state.root.git.as_ref().is_some_and(|git| git.commit.is_some())
        )
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
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_size(theme::text_ui())
                                            .text_color(theme::text())
                                            .child(item.title.clone()),
                                    )
                                    .when_some(item.session_label.as_ref(), |row, label| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .text_size(theme::text_meta())
                                                .text_color(if label.is_own {
                                                    theme::muted()
                                                } else {
                                                    theme::faint()
                                                })
                                                .child(label.text.clone()),
                                        )
                                    }),
                            )
                            .when(
                                !item.subtitle.is_empty() || item.uncommitted.is_some(),
                                |element| {
                                    element.child(
                                        div()
                                            .w_full()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .min_w(px(0.))
                                            .text_size(theme::text_meta())
                                            .text_color(theme::faint())
                                            .when(!item.subtitle.is_empty(), |sub| {
                                                sub.child(
                                                    div()
                                                        .min_w(px(0.))
                                                        .truncate()
                                                        .child(item.subtitle.clone()),
                                                )
                                            })
                                            .when_some(item.uncommitted, |sub, count| {
                                                let (text, color) = if count == 0 {
                                                    (
                                                        crate::i18n::text("changes.clean")
                                                            .to_owned(),
                                                        theme::faint(),
                                                    )
                                                } else {
                                                    (
                                                        crate::i18n::tn(
                                                            "changes.uncommitted_count",
                                                            count,
                                                        ),
                                                        theme::warning(),
                                                    )
                                                };
                                                sub.when(!item.subtitle.is_empty(), |s| {
                                                    s.child(div().flex_none().child("·"))
                                                })
                                                .child(
                                                    div().flex_none().text_color(color).child(text),
                                                )
                                            }),
                                    )
                                },
                            ),
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
