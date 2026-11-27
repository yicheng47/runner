use super::*;

impl ChangesView {
    // ── Shared edge-state screens (Choose, Missing, Message) ───────────

    pub(super) fn render_message(
        &self,
        selector: &'static str,
        icon: &'static str,
        title: &'static str,
        body: String,
        actions: Option<AnyElement>,
    ) -> AnyElement {
        render_centered_message(selector, icon, title.to_owned(), body, actions)
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
                .when(cfg!(test), |el| {
                    el.debug_selector(move || format!("CHANGES_CHOOSE {} {}", id.0, id.1))
                })
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
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
            .map(|(index, (name, path))| entry(("changes-choose-project", index), name, path, cx))
            .collect::<Vec<_>>();
        let recent = self
            .recent
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                entry(
                    ("changes-choose-recent", index),
                    picker::folder_name(&path),
                    path,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let browse = cx.entity();
        div()
            .id("changes-choose")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_CHOOSE".into())
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
                            .child(crate::i18n::text("ui.choose_a_folder_to_show")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "changes.choose_folder_body",
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
                                IconButton::new("changes-browse", "folder-search.svg")
                                    .tooltip(crate::i18n::text("ui.browse"))
                                    .on_press(move |_, cx| {
                                        browse.update(cx, |this, cx| this.browse(cx))
                                    }),
                            ),
                    ),
            )
            .when(!projects.is_empty(), |el| {
                el.child(section(crate::i18n::text("ui.projects")))
                    .children(projects)
            })
            .when(!recent.is_empty(), |el| {
                el.child(section(crate::i18n::text("ui.recent")))
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
                    "changes-show-main-checkout",
                    crate::i18n::tf("format.show_folder", &[("name", name)]),
                )
                .variant(ButtonVariant::Secondary)
                .debug_selector("CHANGES_SHOW_MAIN")
                .on_press(move |_, cx| {
                    let path = path.clone();
                    show_root.update(cx, |this, cx| this.set_root(Some(path), cx));
                })
            }))
            .child(
                Button::new(
                    "changes-choose-root",
                    crate::i18n::text("ui.choose_ellipsis"),
                )
                .variant(ButtonVariant::Outline)
                .on_press(move |_, cx| {
                    choose_root.update(cx, |this, cx| this.show_choose_screen(cx));
                }),
            )
            .into_any_element();
        let shown = compact_missing_path(path);
        self.render_message(
            "CHANGES_MISSING",
            "folder-x.svg",
            crate::i18n::text("ui.this_folder_no_longer_exists"),
            crate::i18n::tf("format.folder_was_moved_or_deleted", &[("path", shown)]),
            Some(actions),
        )
    }
}
