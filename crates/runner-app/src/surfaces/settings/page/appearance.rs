use super::*;

pub(super) fn missions_settings_pane(default_crew: Entity<StyledSelect>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .debug_selector(|| "SETTINGS_MISSIONS_PANE".into())
        .child(PaneHeader::new(crate::i18n::text("ui.missions"), ""))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rems(14. / 16.))
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_DEFAULTS_HEADING".into())
                        .text_size(theme::text_heading())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text("ui.defaults")),
                )
                .child(
                    div()
                        .debug_selector(|| "SETTINGS_MISSIONS_CARD".into())
                        .child(SettingsCard::new(vec![SettingsRow::new(
                            crate::i18n::text("ui.default_crew"),
                            div()
                                .debug_selector(|| "SETTINGS_DEFAULT_CREW".into())
                                .child(default_crew),
                        )
                        .subtitle(crate::i18n::text(
                            "ui.pre_selected_when_starting_a_new_mission",
                        ))
                        .into_any_element()])),
                ),
        )
        .into_any_element()
}

impl NativeRoot {
    pub(super) fn render_missions_settings(&self, _cx: &mut Context<Self>) -> AnyElement {
        missions_settings_pane(self.settings_page.default_crew.clone())
    }

    pub(super) fn render_appearance_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.appearance"),
                crate::i18n::text("ui.theme_and_the_app_and_terminal_palettes_for_each_mode"),
            ))
            .child(SettingsCard::new({
                let rows = vec![SettingsRow::new(crate::i18n::text("ui.theme"), self.render_theme_segmented(cx))
                    .subtitle(crate::i18n::text("ui.match_the_os_or_pin_to_light_or_dark"))
                    .into_any_element()];
                #[cfg(target_os = "macos")]
                let rows = {
                    let mut rows = rows;
                    rows.push(SettingsRow::new(crate::i18n::text("ui.window_material"), self.render_material_segmented(cx))
                        .subtitle(crate::i18n::text("ui.glass_adds_depth_to_the_sidebar_and_window_chrome"))
                        .into_any_element());
                    rows
                };
                rows
            }))
            .child(self.render_theme_preview(cx))
            .child(palette_section(
                crate::i18n::text("ui.light"),
                "SETTINGS_APPEARANCE_LIGHT",
                self.settings_page.light_theme.clone(),
                crate::i18n::text("ui.chrome_colours_when_the_app_is_light"),
                self.settings_page.light_terminal_theme.clone(),
                crate::i18n::text("ui.terminal_colours_when_the_app_is_light"),
            ))
            .child(palette_section(
                crate::i18n::text("ui.dark"),
                "SETTINGS_APPEARANCE_DARK",
                self.settings_page.dark_theme.clone(),
                crate::i18n::text("ui.chrome_colours_when_the_app_is_dark"),
                self.settings_page.dark_terminal_theme.clone(),
                crate::i18n::text("ui.terminal_colours_when_the_app_is_dark"),
            ))
            .when(cfg!(target_os = "macos"), |page| page.child(
                div().text_size(theme::text_ui()).text_color(theme::muted())
                    .child(crate::i18n::text("ui.runner_follows_macos_reduce_transparency_when_enabled_glass_uses_the_solid_appearance")),
            ))
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn render_material_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        use runner_app::appearance::WindowMaterial;
        let selected = self.settings(cx).window_material;
        div()
            .debug_selector(|| "SETTINGS_WINDOW_MATERIAL".into())
            .flex()
            .gap(rems(2. / 16.))
            .p(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .children(
                [
                    (WindowMaterial::Glass, crate::i18n::text("ui.glass")),
                    (WindowMaterial::Solid, crate::i18n::text("ui.solid")),
                ]
                .into_iter()
                .map(|(material, label)| {
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-material-{label}")))
                        .tab_index(0)
                        .px(rems(12. / 16.))
                        .py(rems(5. / 16.))
                        .rounded(rems(4. / 16.))
                        .text_size(theme::text_ui())
                        .when(material == selected, |item| item.bg(theme::raised()))
                        .cursor_pointer()
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, cx| this.set_window_material(material, cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root
                                    .update(cx, |this, cx| this.set_window_material(material, cx));
                            }
                        })
                }),
            )
            .into_any_element()
    }

    #[cfg(target_os = "macos")]
    pub(super) fn set_window_material(
        &self,
        material: runner_app::appearance::WindowMaterial,
        cx: &mut Context<Self>,
    ) {
        self.update_app_settings(cx, true, |settings| {
            if settings.window_material == material {
                return false;
            }
            settings.window_material = material;
            true
        });
        runner_app::appearance::configure(material, cx);
        cx.refresh_windows();
    }

    /// Both panes are built from the picks, not from the active variant: the
    /// left pane is always the light picks and the right the dark picks, and
    /// only the outline and caption follow which mode is resolved now.
    pub(super) fn render_theme_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx);
        let light_variant = theme::resolve_variant(
            ThemeIntent::Light,
            true,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let dark_variant = theme::resolve_variant(
            ThemeIntent::Dark,
            false,
            settings.light_app_theme,
            settings.dark_app_theme,
        );
        let is_light = theme::active_variant().is_light();
        let pick_root = cx.entity();
        let on_pick: theme_preview::PickHandler = Rc::new(move |mode, _, cx| {
            pick_root.update(cx, |this, cx| this.set_theme_intent(mode.intent(), cx));
        });
        theme_preview::theme_preview(
            PreviewPane {
                mode: PreviewMode::Light,
                colors: theme::colors_for(light_variant),
                palette: settings.light_terminal_theme.palette(),
                font: settings.terminal_font_family.font(cx),
                caption: crate::i18n::tf(
                    "format.light",
                    &[
                        (
                            "app_theme",
                            (light_theme_label(settings.light_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.light_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: is_light,
            },
            PreviewPane {
                mode: PreviewMode::Dark,
                colors: theme::colors_for(dark_variant),
                palette: settings.dark_terminal_theme.palette(),
                font: settings.terminal_font_family.font(cx),
                caption: crate::i18n::tf(
                    "format.dark",
                    &[
                        (
                            "app_theme",
                            (dark_theme_label(settings.dark_app_theme)).to_string(),
                        ),
                        (
                            "terminal_theme",
                            (settings.dark_terminal_theme.label()).to_string(),
                        ),
                    ],
                )
                .into(),
                active: !is_light,
            },
            on_pick,
        )
    }

    pub(super) fn render_theme_segmented(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .p(rems(2. / 16.))
            .children(
                [
                    (
                        ThemeIntent::Auto,
                        crate::i18n::text("ui.auto"),
                        "monitor.svg",
                    ),
                    (ThemeIntent::Light, crate::i18n::text("ui.light"), "sun.svg"),
                    (ThemeIntent::Dark, crate::i18n::text("ui.dark"), "moon.svg"),
                ]
                .into_iter()
                .map(|(intent, label, icon)| {
                    let active = intent == self.settings(cx).app_theme;
                    let foreground = if active {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let hover_group = SharedString::from(format!(
                        "settings-theme-{}",
                        label.to_ascii_lowercase()
                    ));
                    let root = cx.entity();
                    let key_root = root.clone();
                    div()
                        .id(SharedString::from(format!("settings-theme-{label}")))
                        .group(hover_group.clone())
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .rounded(rems(4. / 16.))
                        .px(rems(10. / 16.))
                        .py(rems(5. / 16.))
                        .bg(if active {
                            theme::raised()
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(theme::text_ui())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .cursor(CursorStyle::PointingHand)
                        .hover(|button| button.text_color(theme::text()))
                        .child(
                            svg()
                                .path(icon)
                                .size(rems(12. / 16.))
                                .flex_none()
                                .text_color(foreground)
                                .group_hover(hover_group, |icon| icon.text_color(theme::text())),
                        )
                        .child(label)
                        .on_click(move |_, _, cx| {
                            root.update(cx, |this, root_cx| this.set_theme_intent(intent, root_cx));
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root.update(cx, |this, root_cx| {
                                    this.set_theme_intent(intent, root_cx)
                                });
                            }
                        })
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_terminal_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let size = self
            .settings(cx)
            .terminal_font_size
            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX);
        let decrement_root = cx.entity();
        let increment_root = decrement_root.clone();
        let decrement: StepHandler = Rc::new(move |_, cx| {
            decrement_root.update(cx, |this, root_cx| {
                this.set_terminal_font_size(size.saturating_sub(1), root_cx)
            });
        });
        let increment: StepHandler = Rc::new(move |_, cx| {
            increment_root.update(cx, |this, root_cx| {
                this.set_terminal_font_size(size.saturating_add(1), root_cx)
            });
        });
        let size_stepper = Stepper::new(
            "settings-terminal-font-size",
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
            decrement,
            increment,
        )
        .decrement_disabled(size == TERMINAL_FONT_SIZE_MIN)
        .increment_disabled(size == TERMINAL_FONT_SIZE_MAX);

        let rows = vec![
            SettingsRow::new(
                crate::i18n::text("ui.font_family"),
                div()
                    .debug_selector(|| "SETTINGS_TERMINAL_FONT".into())
                    .child(self.settings_page.terminal_font.clone()),
            )
            .subtitle(crate::i18n::text(
                "ui.typeface_used_by_the_embedded_terminal",
            ))
            .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.terminal_font_size"), size_stepper)
                .subtitle(crate::i18n::text("ui.glyph_size_for_the_embedded_terminal"))
                .into_any_element(),
            SettingsRow::new(
                crate::i18n::text("ui.cursor_style"),
                self.settings_page.terminal_cursor.clone(),
            )
            .subtitle(crate::i18n::text(
                "ui.block_underline_or_bar_affects_the_prompt_caret_only",
            ))
            .into_any_element(),
        ];
        #[cfg(target_os = "macos")]
        let rows = {
            let mut rows = rows;
            rows.push(
                SettingsRow::new(
                    crate::i18n::text("terminal.option_as_meta"),
                    div()
                        .debug_selector(|| "SETTINGS_OPTION_AS_META".into())
                        .child(self.settings_page.option_as_meta.clone()),
                )
                .subtitle(crate::i18n::text("terminal.option_as_meta_description"))
                .into_any_element(),
            );
            rows
        };

        div()
            .flex()
            .flex_col()
            .gap_5()
            .debug_selector(|| "SETTINGS_TERMINAL_PANE".into())
            .child(PaneHeader::new(
                crate::i18n::text("ui.terminal"),
                crate::i18n::text(
                    "ui.font_size_and_cursor_for_the_embedded_terminal_palettes_live_in_appearance",
                ),
            ))
            .child(SettingsCard::new(rows))
            .into_any_element()
    }
}

pub(super) fn light_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner-light", "Runner Light").swatch(0x00a66a),
        SelectOption::new("catppuccin-latte", "Catppuccin Latte").swatch(0x8839ef),
    ]
}

pub(super) fn light_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
    ]
}

pub(super) fn dark_terminal_theme_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("runner", "Runner").swatch(0x00ff9c),
        SelectOption::new("catppuccin-mocha", "Catppuccin Mocha").swatch(0xcba6f7),
        SelectOption::new("rose-pine-dawn", "Rosé Pine Dawn").swatch(0xd7827e),
    ]
}

/// A mode's label and its App palette + Terminal palette card.
pub(super) fn palette_section(
    label: &'static str,
    selector: &'static str,
    app_palette: Entity<StyledSelect>,
    app_subtitle: &'static str,
    terminal_palette: Entity<StyledSelect>,
    terminal_subtitle: &'static str,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .debug_selector(move || selector.into())
        .child(
            div()
                .text_size(theme::text_body())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(label),
        )
        .child(SettingsCard::new(vec![
            SettingsRow::new(crate::i18n::text("ui.app_palette"), app_palette)
                .subtitle(app_subtitle)
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.terminal_palette"), terminal_palette)
                .subtitle(terminal_subtitle)
                .into_any_element(),
        ]))
        .into_any_element()
}

pub(super) fn light_theme_label(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "Runner Light",
        LightTheme::CatppuccinLatte => "Catppuccin Latte",
    }
}

pub(super) fn dark_theme_label(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "Runner",
        DarkTheme::CatppuccinMocha => "Catppuccin Mocha",
    }
}

pub(super) fn light_theme_value(value: LightTheme) -> &'static str {
    match value {
        LightTheme::RunnerLight => "runner-light",
        LightTheme::CatppuccinLatte => "catppuccin-latte",
    }
}

pub(super) fn parse_light_theme(value: &str) -> Option<LightTheme> {
    match value {
        "runner-light" => Some(LightTheme::RunnerLight),
        "catppuccin-latte" => Some(LightTheme::CatppuccinLatte),
        _ => None,
    }
}

pub(super) fn dark_theme_value(value: DarkTheme) -> &'static str {
    match value {
        DarkTheme::Runner => "carbon",
        DarkTheme::CatppuccinMocha => "catppuccin-mocha",
    }
}

pub(super) fn parse_dark_theme(value: &str) -> Option<DarkTheme> {
    match value {
        "carbon" => Some(DarkTheme::Runner),
        "catppuccin-mocha" => Some(DarkTheme::CatppuccinMocha),
        _ => None,
    }
}

pub(super) fn terminal_cursor_value(value: TerminalCursorStyle) -> &'static str {
    match value {
        TerminalCursorStyle::Block => "block",
        TerminalCursorStyle::Underline => "underline",
        TerminalCursorStyle::Bar => "bar",
    }
}

pub(super) fn parse_terminal_cursor(value: &str) -> Option<TerminalCursorStyle> {
    match value {
        "block" => Some(TerminalCursorStyle::Block),
        "underline" => Some(TerminalCursorStyle::Underline),
        "bar" => Some(TerminalCursorStyle::Bar),
        _ => None,
    }
}
