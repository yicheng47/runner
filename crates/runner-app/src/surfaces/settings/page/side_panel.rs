//! Settings → Side panel: the Panel page, which shows the panel and turns
//! its views on, and the Files page, which sets the tree and the viewer.

use super::*;

use gpui::Div;

use crate::app_settings::{MarkdownOpenAs, TerminalFontFamily};
use crate::file_icons::{self, IconSet};
use crate::surfaces::side_panel::{self as panel, PanelSurface};

/// The header preview's width: a chat's panel at the width it opens with.
const HEADER_PREVIEW_WIDTH: f32 = 360.;
/// The views the Panel page dims while the panel is off.
const DIMMED: f32 = 0.45;

pub(super) fn file_icon_options() -> Vec<SelectOption> {
    IconSet::ALL
        .into_iter()
        .map(|set| {
            SelectOption::new(set.key(), set.label())
                .leading(SelectLeading::new(
                    format!("icons-{}", set.key()),
                    move |_| icon_samples(set),
                ))
                .without_description()
        })
        .collect()
}

/// A plain folder, a Rust file and a Markdown file in `set`, as the tree
/// draws them; names a theme gives a special icon, such as `src` or
/// `README.md`, would not show the set's ordinary ones.
fn icon_samples(set: IconSet) -> AnyElement {
    let dark = !theme::active_variant().is_light();
    let samples = [
        (
            file_icons::folder_icon(set, dark, "unnamed-folder", false),
            true,
        ),
        (file_icons::file_icon(set, dark, "main.rs"), false),
        (file_icons::file_icon(set, dark, "notes.md"), false),
    ];
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .children(samples.into_iter().map(|(icon, folder)| {
            match icon {
                Some(icon) if icon.monochrome => svg()
                    .flex_none()
                    .size(px(14.))
                    .path(icon.path)
                    .text_color(theme::muted())
                    .into_any_element(),
                Some(icon) => gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(
                    icon.path,
                )))
                .flex_none()
                .size(px(14.))
                .into_any_element(),
                None => svg()
                    .flex_none()
                    .size(px(14.))
                    .path(if folder {
                        "folder.svg"
                    } else {
                        "file-text.svg"
                    })
                    .text_color(theme::muted())
                    .into_any_element(),
            }
        }))
        .into_any_element()
}

/// Same as terminal, then the terminal's Bundled and Installed lists, with
/// a saved font that is not installed kept at the end.
pub(super) fn files_font_options(
    family: Option<&TerminalFontFamily>,
    cx: &App,
) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new(
        "",
        crate::i18n::message("side_panel.same_as_terminal"),
    )];
    options.extend(terminal_font_options(
        family.unwrap_or(&TerminalFontFamily::default()),
        cx,
    ));
    options
}

pub(super) fn files_font_value(family: Option<&TerminalFontFamily>) -> String {
    family.map_or_else(String::new, |family| family.family().to_owned())
}

pub(super) fn markdown_open_as_options() -> Vec<SelectOption> {
    MarkdownOpenAs::ALL
        .into_iter()
        .map(|mode| {
            SelectOption::new(
                mode.key(),
                crate::i18n::message(match mode {
                    MarkdownOpenAs::Rendered => "side_panel.rendered",
                    MarkdownOpenAs::Source => "side_panel.source",
                }),
            )
        })
        .collect()
}

fn section_heading(label: &'static str, selector: &'static str) -> Div {
    div()
        .debug_selector(move || selector.into())
        .text_size(theme::text_heading())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text())
        .child(crate::i18n::text(label))
}

/// Names `element` for tests, recording its fill for the theme snapshots.
fn snapshot(id: &'static str, element: Div) -> Div {
    #[cfg(test)]
    let element = crate::theme_snapshot::record_fill(id, element);
    element.debug_selector(move || id.into())
}

fn page_note(text: impl Into<SharedString>) -> Div {
    div()
        .px_1()
        .text_size(theme::text_ui())
        .text_color(theme::faint())
        .child(text.into())
}

impl NativeRoot {
    pub(super) fn render_side_panel_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx).clone();
        let shown = settings.show_side_panel;
        let show_root = cx.entity();
        let show_row = SettingsRow::new(
            crate::i18n::text("side_panel.show"),
            div()
                .debug_selector(|| "SETTINGS_SHOW_SIDE_PANEL_TOGGLE".into())
                .child(Toggle::new("settings-show-side-panel", shown).on_change(
                    move |on, _, cx| {
                        show_root.update(cx, |this, cx| {
                            this.set_side_panel_setting(
                                cx,
                                |settings| &mut settings.show_side_panel,
                                on,
                            )
                        });
                    },
                )),
        )
        .subtitle(crate::i18n::text("side_panel.show_description"));

        let view_row =
            |id: &'static str,
             icon: &'static str,
             label: &'static str,
             description: &'static str,
             on: bool,
             field: fn(&mut AppSettings) -> &mut bool,
             cx: &mut Context<Self>| {
                let root = cx.entity();
                div()
                    .debug_selector(move || id.to_uppercase().replace('-', "_"))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .pt_2()
                    .pb_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .items_start()
                            .gap_3()
                            .when(!shown, |text| text.opacity(DIMMED))
                            .child(
                                svg()
                                    .flex_none()
                                    .mt(rems(2. / 16.))
                                    .size(rems(1.))
                                    .path(icon)
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_body())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::text())
                                            .child(crate::i18n::text(label)),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_meta())
                                            .text_color(theme::muted())
                                            .child(crate::i18n::text(description)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .debug_selector(move || {
                                format!("{}_TOGGLE", id.to_uppercase().replace('-', "_"))
                            })
                            .child(Toggle::new(id, on).disabled(!shown).on_change(
                                move |on, _, cx| {
                                    root.update(cx, |this, cx| {
                                        this.set_side_panel_setting(cx, field, on)
                                    });
                                },
                            )),
                    )
                    .into_any_element()
            };
        let info_row = view_row(
            "settings-info-view",
            "info.svg",
            "ui.info",
            "side_panel.info_description",
            settings.info_view,
            |settings| &mut settings.info_view,
            cx,
        );
        let files_row = view_row(
            "settings-files-view",
            "folder.svg",
            "ui.files",
            "side_panel.files_description",
            settings.files_view,
            |settings| &mut settings.files_view,
            cx,
        );
        let preview = div()
            .px_4()
            .pt_4()
            .pb_4()
            .border_b_1()
            .border_color(theme::border())
            .flex()
            .flex_col()
            .gap_2()
            .when(!shown, |preview| preview.opacity(DIMMED))
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("side_panel.header_in_a_chat")),
            )
            .child(
                div()
                    .w(rems(HEADER_PREVIEW_WIDTH / 16.))
                    .max_w_full()
                    .h(rems(40. / 16.))
                    .pl_4()
                    .pr_2()
                    .flex()
                    .items_center()
                    .rounded(rems(8. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .map(|preview| snapshot("SIDE_PANEL_HEADER_PREVIEW", preview))
                    .child(panel::header_views(
                        PanelSurface::Chat,
                        &settings,
                        panel::shown_view(PanelSurface::Chat, &settings, panel::PanelViewId::Info),
                        Rc::new(|_, _| {}),
                        IconButton::new("settings-preview-collapse", "panel-right-open.svg"),
                    )),
            );
        let views_card = div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .map(|card| snapshot("SETTINGS_SIDE_PANEL_VIEWS", card))
            .child(preview)
            .child(info_row)
            .child(files_row);

        div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "SETTINGS_SIDE_PANEL_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("side_panel.panel"),
                crate::i18n::text("side_panel.panel_description"),
            ))
            .child(SettingsCard::new(vec![show_row.into_any_element()]))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .child(
                        section_heading("side_panel.views", "SETTINGS_SIDE_PANEL_VIEWS_HEADING")
                            .when(!shown, |heading| heading.opacity(DIMMED)),
                    )
                    .child(views_card)
                    .child(
                        page_note(crate::i18n::text("side_panel.sessions_note"))
                            .when(!shown, |note| note.opacity(DIMMED)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_files_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx).clone();
        let size = settings.files_font_size();
        let follows = settings.files_font_size.is_none();
        let step = |delta: i32, cx: &mut Context<Self>| -> StepHandler {
            let root = cx.entity();
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    // The size now, which another window may have changed
                    // since this page drew.
                    let size = this.settings(cx).files_font_size();
                    let size = (size as i32 + delta)
                        .clamp(TERMINAL_FONT_SIZE_MIN as i32, TERMINAL_FONT_SIZE_MAX as i32)
                        as u16;
                    this.set_files_font_size(Some(size), cx);
                })
            })
        };
        let stepper = Stepper::new(
            "settings-files-font-size",
            64.,
            div()
                .flex()
                .items_center()
                .gap(rems(3. / 16.))
                .font_family(theme::UI_MONOSPACE_FONT)
                .child(
                    div()
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .child(size.to_string()),
                )
                .child(
                    div()
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child("px"),
                ),
            step(-1, cx),
            step(1, cx),
        )
        .decrement_disabled(size <= TERMINAL_FONT_SIZE_MIN)
        .increment_disabled(size >= TERMINAL_FONT_SIZE_MAX);
        let reset_root = cx.entity();
        let size_control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "SETTINGS_FILES_FONT_SIZE".into())
                    .child(stepper),
            )
            .when(!follows, |control| {
                control.child(
                    Button::new(
                        "settings-files-font-size-reset",
                        crate::i18n::text("ui.reset"),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .debug_selector("SETTINGS_FILES_FONT_SIZE_RESET")
                    .on_press(move |_, cx| {
                        reset_root.update(cx, |this, cx| this.set_files_font_size(None, cx))
                    }),
                )
            });
        let toggle_row = |id: &'static str,
                          label: &'static str,
                          description: SharedString,
                          on: bool,
                          field: fn(&mut AppSettings) -> &mut bool,
                          cx: &mut Context<Self>| {
            let root = cx.entity();
            SettingsRow::new(
                crate::i18n::text(label),
                div()
                    .debug_selector(move || {
                        format!("{}_TOGGLE", id.to_uppercase().replace('-', "_"))
                    })
                    .child(Toggle::new(id, on).on_change(move |on, _, cx| {
                        root.update(cx, |this, cx| this.set_side_panel_setting(cx, field, on));
                    })),
            )
            .subtitle(description)
            .into_any_element()
        };
        let highlighting = toggle_row(
            "settings-files-highlighting",
            "side_panel.syntax_highlighting",
            crate::i18n::tf(
                "side_panel.syntax_highlighting_description",
                &[
                    ("dark", settings.dark_terminal_theme.label().to_owned()),
                    ("light", settings.light_terminal_theme.label().to_owned()),
                ],
            )
            .into(),
            settings.files_syntax_highlighting,
            |settings| &mut settings.files_syntax_highlighting,
            cx,
        );

        div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "SETTINGS_FILES_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("ui.files"),
                crate::i18n::text("side_panel.files_page_description"),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .child(section_heading(
                        "side_panel.tree",
                        "SETTINGS_FILES_TREE_HEADING",
                    ))
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("side_panel.file_icons"),
                        div()
                            .debug_selector(|| "SETTINGS_FILE_ICONS".into())
                            .child(self.settings_page.file_icons.clone()),
                    )
                    .subtitle(crate::i18n::text("side_panel.file_icons_description"))
                    .into_any_element()])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .child(section_heading(
                        "side_panel.viewer",
                        "SETTINGS_FILES_VIEWER_HEADING",
                    ))
                    .child(SettingsCard::new(vec![
                        SettingsRow::new(
                            crate::i18n::text("side_panel.font"),
                            div()
                                .debug_selector(|| "SETTINGS_FILES_FONT".into())
                                .child(self.settings_page.files_font.clone()),
                        )
                        .subtitle(crate::i18n::text("side_panel.font_description"))
                        .into_any_element(),
                        SettingsRow::new(crate::i18n::text("side_panel.font_size"), size_control)
                            .subtitle(crate::i18n::text("side_panel.font_size_description"))
                            .into_any_element(),
                        highlighting,
                        SettingsRow::new(
                            crate::i18n::text("side_panel.markdown_open_as"),
                            div()
                                .debug_selector(|| "SETTINGS_MARKDOWN_OPEN_AS".into())
                                .child(self.settings_page.markdown_open_as.clone()),
                        )
                        .subtitle(crate::i18n::text("side_panel.markdown_open_as_description"))
                        .into_any_element(),
                    ]))
                    .child(page_note(crate::i18n::tf(
                        "side_panel.open_in_note",
                        &[("shortcut", crate::platform_ui::PRIMARY_MODIFIER.to_string())],
                    ))),
            )
            .into_any_element()
    }

    fn set_side_panel_setting(
        &mut self,
        cx: &mut Context<Self>,
        field: fn(&mut AppSettings) -> &mut bool,
        on: bool,
    ) {
        if self.update_app_settings(cx, true, |settings| update_if_changed(field(settings), on)) {
            cx.notify();
        }
    }

    fn set_files_font_size(&mut self, size: Option<u16>, cx: &mut Context<Self>) {
        if self.update_app_settings(cx, true, |settings| {
            update_if_changed(&mut settings.files_font_size, size)
        }) {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{testing::with_locale, Locale};
    use crate::theme::ThemeIntent;
    use crate::theme_snapshot::{assert_fill_color, ThemeGuard};
    use gpui::{size, Modifiers, TestAppContext, VisualTestContext, WindowHandle};

    struct Page {
        _temp: tempfile::TempDir,
        _cx: TestAppContext,
        store: Entity<crate::app_store::AppStore>,
        host: WindowHandle<NativeRoot>,
        visual: VisualTestContext,
    }

    impl Page {
        fn new(pane: SettingsPane, intent: ThemeIntent) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| store.settings.app_theme = intent);
            let host = cx.add_window(|window, cx| {
                NativeRoot::new(
                    "side-panel-settings".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(1440.), px(900.)));
            host.update(&mut visual, |root, window, cx| {
                root.enter_settings_pane(pane, window, cx)
            })
            .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
            visual.run_until_parked();
            Self {
                _temp: temp,
                _cx: cx,
                store,
                host,
                visual,
            }
        }

        fn has(&mut self, selector: &'static str) -> bool {
            self.visual.debug_bounds(selector).is_some()
        }

        fn click(&mut self, selector: &'static str) {
            let bounds = self
                .visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            self.visual
                .simulate_click(bounds.center(), Modifiers::default());
            self.visual.run_until_parked();
        }

        fn settings(&mut self) -> AppSettings {
            self.store
                .read_with(&self.visual, |store, _| store.settings.clone())
        }

        fn nav_label(&mut self, pane: SettingsPane) -> String {
            self.host
                .read_with(&self.visual, |_, _| pane.label().to_owned())
                .unwrap()
        }
    }

    #[test]
    fn the_panel_page_previews_the_header_and_dims_the_views_while_the_panel_is_off() {
        let _theme = ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                    let mut page = Page::new(SettingsPane::SidePanel, intent);
                    assert_eq!(
                        page.nav_label(SettingsPane::SidePanel),
                        if locale == Locale::English {
                            "Panel"
                        } else {
                            "面板"
                        }
                    );
                    assert!(page.has("SETTINGS_SIDE_PANEL_PANE"));
                    assert_fill_color(
                        &mut page.visual,
                        "SETTINGS_SIDE_PANEL_VIEWS",
                        Some(theme::panel()),
                    );
                    assert_fill_color(
                        &mut page.visual,
                        "SIDE_PANEL_HEADER_PREVIEW",
                        Some(theme::bg()),
                    );
                    let preview = page
                        .visual
                        .debug_bounds("SIDE_PANEL_HEADER_PREVIEW")
                        .unwrap();
                    let title = page.visual.debug_bounds("SIDE_PANEL_TITLE Info").unwrap();
                    let info = page.visual.debug_bounds("PANEL_VIEW Info").unwrap();
                    let files = page.visual.debug_bounds("PANEL_VIEW Files").unwrap();
                    assert!(preview.contains(&info.center()) && preview.contains(&files.center()));
                    assert!(title.right() < info.left() && info.right() < files.left());
                    assert_fill_color(&mut page.visual, "PANEL_VIEW Info", Some(theme::raised()));
                    assert_fill_color(&mut page.visual, "PANEL_VIEW Files", None);
                    assert!(
                        (preview.size.width - px(HEADER_PREVIEW_WIDTH)).abs() <= px(1.),
                        "a real-size header: {preview:?}"
                    );

                    // The preview redraws as the switches change.
                    // Files alone: its title, and no view buttons.
                    page.click("SETTINGS_INFO_VIEW_TOGGLE");
                    assert!(!page.settings().info_view);
                    assert!(page.has("SIDE_PANEL_TITLE Files"));
                    assert!(!page.has("PANEL_VIEW Info") && !page.has("PANEL_VIEW Files"));
                    page.click("SETTINGS_FILES_VIEW_TOGGLE");
                    assert!(!page.has("SIDE_PANEL_TITLE Files"));
                    page.click("SETTINGS_INFO_VIEW_TOGGLE");
                    page.click("SETTINGS_FILES_VIEW_TOGGLE");
                    assert!(page.has("SIDE_PANEL_TITLE Info"));
                    assert!(page.has("PANEL_VIEW Info") && page.has("PANEL_VIEW Files"));

                    // Off: the switches keep their values and stop answering.
                    page.click("SETTINGS_SHOW_SIDE_PANEL_TOGGLE");
                    assert!(!page.settings().show_side_panel);
                    page.click("SETTINGS_INFO_VIEW_TOGGLE");
                    page.click("SETTINGS_FILES_VIEW_TOGGLE");
                    let settings = page.settings();
                    assert!(settings.info_view && settings.files_view);
                    page.click("SETTINGS_SHOW_SIDE_PANEL_TOGGLE");
                    assert!(page.settings().show_side_panel);
                }
            });
        }
    }

    #[test]
    fn the_files_page_sets_every_option_and_the_size_resets_to_follow_the_terminal() {
        let _theme = ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
                    let mut page = Page::new(SettingsPane::Files, intent);
                    assert_eq!(
                        page.nav_label(SettingsPane::Files),
                        if locale == Locale::English {
                            "Files"
                        } else {
                            "文件"
                        }
                    );
                    for selector in [
                        "SETTINGS_FILES_PANE",
                        "SETTINGS_FILES_TREE_HEADING",
                        "SETTINGS_FILE_ICONS",
                        "SETTINGS_FILES_VIEWER_HEADING",
                        "SETTINGS_FILES_FONT",
                        "SETTINGS_MARKDOWN_OPEN_AS",
                    ] {
                        assert!(page.has(selector), "{selector}");
                    }
                    assert!(
                        !page.has("SETTINGS_FILES_FONT_SIZE_RESET"),
                        "follows the terminal"
                    );
                    let icons = page.visual.debug_bounds("SETTINGS_FILE_ICONS").unwrap();
                    assert!(
                        icons.size.width >= px(199.),
                        "room for Catppuccin beside its samples: {icons:?}"
                    );

                    assert!(
                        !page.has("SETTINGS_FILES_SINGLE_CLICK_TOGGLE"),
                        "every open keeps its tab, with no setting"
                    );
                    page.click("SETTINGS_FILES_HIGHLIGHTING_TOGGLE");
                    assert!(!page.settings().files_syntax_highlighting);

                    page.host
                        .update(&mut page.visual, |root, _, cx| {
                            root.set_files_font_size(Some(15), cx)
                        })
                        .unwrap();
                    page.visual.run_until_parked();
                    assert!(page.has("SETTINGS_FILES_FONT_SIZE_RESET"));
                    page.click("SETTINGS_FILES_FONT_SIZE_RESET");
                    assert_eq!(page.settings().files_font_size, None);
                    assert!(!page.has("SETTINGS_FILES_FONT_SIZE_RESET"));

                    for (selection, value) in [
                        (SettingsSelection::FileIcons, "material"),
                        (SettingsSelection::FilesFont, "Menlo"),
                        (SettingsSelection::MarkdownOpenAs, "source"),
                    ] {
                        page.host
                            .update(&mut page.visual, |root, window, cx| {
                                root.apply_settings_selection(selection, value, window, cx)
                            })
                            .unwrap();
                    }
                    let settings = page.settings();
                    assert_eq!(settings.file_icons, IconSet::Material);
                    assert_eq!(
                        settings.files_font_family,
                        Some(TerminalFontFamily::new("Menlo"))
                    );
                    assert_eq!(settings.markdown_open_as, MarkdownOpenAs::Source);
                    page.host
                        .update(&mut page.visual, |root, window, cx| {
                            root.apply_settings_selection(
                                SettingsSelection::FilesFont,
                                "",
                                window,
                                cx,
                            )
                        })
                        .unwrap();
                    assert_eq!(page.settings().files_font_family, None, "Same as terminal");
                }
            });
        }
    }

    #[test]
    fn the_files_page_in_another_window_follows_changes_made_here() {
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let window = |name: &'static str, cx: &mut TestAppContext| {
            let store = store.clone();
            let logs = temp.path().join(name);
            let host = cx.add_window(move |window, cx| {
                NativeRoot::new(name.into(), logs, None, None, store, window, cx)
            });
            let visual = VisualTestContext::from_window(host.into(), cx);
            visual.simulate_resize(size(px(1440.), px(900.)));
            visual.run_until_parked();
            (host, visual)
        };
        let (here, mut here_visual) = window("here", &mut cx);
        let (there, mut there_visual) = window("there", &mut cx);
        there
            .update(&mut there_visual, |root, window, cx| {
                root.enter_settings_pane(SettingsPane::Files, window, cx)
            })
            .unwrap();
        there_visual.run_until_parked();
        assert!(there_visual.debug_bounds("SETTINGS_FILES_PANE").is_some());

        here.update(&mut here_visual, |root, window, cx| {
            for (selection, value) in [
                (SettingsSelection::FileIcons, "material"),
                (SettingsSelection::FilesFont, "Menlo"),
                (SettingsSelection::MarkdownOpenAs, "source"),
            ] {
                root.apply_settings_selection(selection, value, window, cx);
            }
            root.set_files_font_size(Some(15), cx);
            root.set_side_panel_setting(
                cx,
                |settings| &mut settings.files_syntax_highlighting,
                false,
            );
        })
        .unwrap();
        here_visual.run_until_parked();
        there_visual.run_until_parked();

        let values = there
            .read_with(&there_visual, |root, cx| {
                [
                    root.settings_page.file_icons.read(cx).value().to_owned(),
                    root.settings_page.files_font.read(cx).value().to_owned(),
                    root.settings_page
                        .markdown_open_as
                        .read(cx)
                        .value()
                        .to_owned(),
                ]
            })
            .unwrap();
        assert_eq!(values, ["material", "Menlo", "source"]);
        assert!(
            there_visual
                .debug_bounds("SETTINGS_FILES_FONT_SIZE_RESET")
                .is_some(),
            "the visible page redraws with the size set elsewhere"
        );

        // A step there starts from the size set here, not the one it drew.
        here.update(&mut here_visual, |root, _, cx| {
            root.set_files_font_size(Some(18), cx)
        })
        .unwrap();
        let stepper = there_visual
            .debug_bounds("SETTINGS_FILES_FONT_SIZE")
            .unwrap();
        there_visual.simulate_click(
            gpui::point(stepper.right() - px(6.), stepper.center().y),
            Modifiers::default(),
        );
        there_visual.run_until_parked();
        assert_eq!(
            store.read_with(&there_visual, |store, _| store.settings.files_font_size),
            Some(19)
        );
    }

    #[test]
    fn the_font_list_puts_same_as_terminal_first_and_keeps_a_missing_saved_font() {
        let cx = TestAppContext::single();
        cx.update(|cx| {
            let options = files_font_options(None, cx);
            assert_eq!(options[0].value, "");
            assert_eq!(options[1].value, crate::terminal_fonts::BUNDLED_FONT);
            let missing = TerminalFontFamily::new("Gone Mono");
            let options = files_font_options(Some(&missing), cx);
            assert!(options.iter().any(|option| option.value == "Gone Mono"));
            assert_eq!(files_font_value(Some(&missing)), "Gone Mono");
            assert_eq!(files_font_value(None), "");
        });
        // The samples are each set's ordinary folder and files.
        for set in IconSet::ALL {
            for dark in [false, true] {
                assert_eq!(
                    file_icons::folder_icon(set, dark, "unnamed-folder", false),
                    file_icons::folder_icon(set, dark, "another-plain-folder", false),
                    "{set:?}"
                );
                assert_eq!(
                    file_icons::file_icon(set, dark, "notes.md"),
                    file_icons::file_icon(set, dark, "other.md"),
                    "{set:?}"
                );
            }
        }
        assert_eq!(
            file_icon_options()
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["catppuccin", "material", "minimal"]
        );
        assert_eq!(
            markdown_open_as_options()
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["rendered", "source"]
        );
    }
}
